// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');

const root = path.resolve(__dirname, '..');
const source = fs.readFileSync(path.join(root, 'web/commands.js'), 'utf8');
const tui = fs.readFileSync(path.join(root, 'crates/blora-tui/src/slash.rs'), 'utf8').split('pub const MAX_VISIBLE')[0];
const cli = [...tui.matchAll(/cmd\(\s*"([^"]+)"\s*,\s*&\[([^\]]*)\]\s*,\s*"([^"]*)"\s*,\s*"[^"]*"\s*,\s*"([^"]*)"/g)]
  .map(([, name, aliases, hint, group]) => ({ name, aliases: [...aliases.matchAll(/"([^"]+)"/g)].map(match => match[1]), hint, group }));
const plain = value => JSON.parse(JSON.stringify(value));

function harness() {
  const calls = [];
  const prefs = new Map();
  const attributes = {};
  const controls = {};
  const state = { sessionId: 'ses_test', running: false, permission: 'default' };
  const document = {
    documentElement: { style: {}, setAttribute: (key, value) => { attributes[key] = value; } },
    querySelector: selector => controls[selector] || null,
  };
  const window = {
    location: {}, matchMedia: () => ({ matches: true }),
    navigator: { clipboard: { writeText: async text => calls.push({ kind: 'copy', text }) } },
  };
  let approved = true;
  const workbench = {
    open: (name, options) => { calls.push({ kind: 'open', name, query: options.query }); },
    requestPermission: async mode => { calls.push({ kind: 'permission', mode }); return approved; },
    confirm: async (title, text, risk) => { calls.push({ kind: 'confirm', title, text, risk }); return approved; },
  };
  window.BloraWorkbench = workbench;
  vm.runInNewContext(source, { window, document });
  const bridge = {
    getState: () => state,
    stored: (key, fallback) => prefs.has(key) ? prefs.get(key) : fallback,
    store: (key, value) => prefs.set(key, value),
    api: async (url, options) => {
      const body = options?.body ? JSON.parse(options.body) : undefined;
      calls.push({ kind: 'api', url, body });
      if (url === '/api/sessions' && body) return { id: 'ses_new' };
      if (url === '/api/tasks') return { id: 'task_new' };
      if (url.endsWith('/actions')) return { output: '操作输出' };
      if (url.endsWith('/memory')) return { memories: [{ key: 'key', value: 'value' }], forgotten: true };
      if (url.startsWith('/api/sessions?')) return [{ id: 'ses_match', mode: 'code', title: '匹配' }];
      return { id: 'ses_test', provider: 'provider', model: 'model', transcript: [{ kind: 'assistant', text: '回复' }] };
    },
    applyPermission: mode => { state.permission = mode; calls.push({ kind: 'apply-permission', mode }); },
    applyRunMode: mode => calls.push({ kind: 'mode', mode }),
    refreshSessions: async () => calls.push({ kind: 'refresh' }),
    selectSession: async id => { state.sessionId = id; calls.push({ kind: 'select', id }); },
    showNotice: text => calls.push({ kind: 'notice', text }),
    showAlert: text => calls.push({ kind: 'alert', text }),
    enterOnboarding: step => calls.push({ kind: 'onboarding', step }),
    renderTranscript: () => calls.push({ kind: 'render' }),
    setSearch: query => calls.push({ kind: 'search', query }),
  };
  return {
    commands: window.BloraCommands, bridge, state, window, workbench, calls, prefs, attributes, controls,
    approve: value => { approved = value; },
    execute: raw => window.BloraCommands.execute(raw, bridge),
    requests: () => calls.filter(call => call.kind === 'api'),
  };
}

test('catalog matches every CLI name, alias, hint, group and display order', () => {
  const { commands } = harness();
  assert.equal(cli.length, 80);
  assert.deepEqual(plain(commands.catalog).map(({ name, aliases, hint, group }) => ({ name, aliases, hint, group })), cli);
  for (const row of cli) {
    assert.equal(commands.resolve(` /${row.name.toUpperCase()} `).name, row.name);
    for (const alias of row.aliases) assert.equal(commands.resolve(alias).name, row.name);
    assert.match(commands.resolve(row.name).about, /[\u3400-\u9fff]/);
  }
  assert.equal(commands.resolve('unknown'), null);
  assert.ok(Object.isFrozen(commands.catalog));
});

test('matches requires slash and ranks exact, prefix, substring and subsequence', () => {
  const { commands } = harness();
  assert.deepEqual(plain(commands.matches('/')).map(row => row.name), cli.map(row => row.name));
  assert.equal(commands.matches('model').length, 0);
  assert.equal(commands.matches('/MODEL')[0].name, 'model');
  assert.equal(commands.matches('/yolo')[0].name, 'yes');
  assert.equal(commands.matches('/gto')[0].name, 'goto');
  assert.ok(commands.matches('/cpt').some(row => row.name === 'compact'));
  const query = 'co';
  const rank = name => name === query ? 0 : name.startsWith(query) ? 1 : name.includes(query) ? 2 : 3;
  const ranked = commands.matches('/co').map(row => ({ name: row.name, score: Math.min(...[row.name, ...row.aliases].map(rank)) }));
  const sorted = ranked.slice().sort((a, b) => a.score - b.score || a.name.length - b.name.length || (a.name < b.name ? -1 : 1));
  assert.deepEqual(plain(ranked), plain(sorted));
});

test('core commands and every core alias fall back without side effects', async () => {
  const h = harness();
  for (const name of ['compact', 'fork', 'export', 'archive', 'resume', 'cancel', 'steer', 'usage', 'read', 'install', 'approvals', 'timeline', 'artifacts']) {
    const row = h.commands.resolve(name);
    for (const token of [name, ...row.aliases]) assert.equal(await h.execute(`/${token} 参数`), false);
  }
  assert.equal(await h.execute('普通提示词'), false);
  assert.equal(await h.execute('/unknown'), false);
  assert.equal(h.calls.length, 0);
});

test('help delegates query to the UI help window', async () => {
  const h = harness();
  assert.equal(await h.execute('/? memory'), true);
  assert.deepEqual(h.calls, [{ kind: 'open', name: 'help', query: 'memory' }]);
});

test('state.permission overrides saved preferences; explicit confirmation applies permissions', async () => {
  const h = harness();
  h.prefs.set('blora-permission', 'yolo');
  h.approve(false);
  await h.execute('/yes on');
  assert.equal(h.state.permission, 'default');
  h.approve(true);
  await h.execute('/always-approve on');
  assert.equal(h.state.permission, 'bypassPermissions');
  await h.execute('/yes');
  assert.equal(h.state.permission, 'default');
  await h.execute('/settings permission-mode=acceptEdits');
  assert.equal(h.state.permission, 'acceptEdits');
});

test('task, cron and loop preserve bypass with confirmed true and cancel without creating', async () => {
  for (const command of ['/task 检查代码', '/cron 0 1 * * * 检查代码', '/loop 5m 检查代码']) {
    const h = harness();
    h.state.permission = 'yolo';
    h.approve(false);
    await h.execute(command);
    assert.equal(h.requests().length, 0);
    h.approve(true);
    await h.execute(command);
    const request = h.requests()[0];
    assert.equal(request.url, '/api/tasks');
    assert.equal(request.body.auto_approve, true);
    assert.equal(request.body.confirmed, true);
    assert.equal(request.body.prompt, '检查代码');
    if (command.startsWith('/cron')) assert.equal(request.body.cron, '0 1 * * *');
    if (command.startsWith('/loop')) assert.equal(request.body.delay_seconds, 300);
    assert.ok(h.calls.some(call => call.kind === 'permission' && call.mode === 'bypassPermissions'));
  }
});

test('background tasks do not inherit plan or acceptEdits and ignore stale bypass preferences', async () => {
  for (const permission of ['default', 'plan', 'acceptEdits']) {
    const h = harness();
    h.state.permission = permission;
    h.prefs.set('blora-permission', 'yolo');
    await h.execute('/task 复核');
    assert.equal(h.requests()[0].body.auto_approve, false);
    assert.equal(h.requests()[0].body.confirmed, false);
    assert.equal(h.calls.filter(call => call.kind === 'permission').length, 0);
  }
});

test('parameter settings persist, apply to controls and render without opening a dialog', async () => {
  const h = harness();
  const control = { setAttribute: (key, value) => { control[key] = value; } };
  h.controls['#pref-worktree'] = control;
  for (const command of ['/settings theme=indigo', '/settings scheme=dark', '/settings message-roles=off', '/settings hide-tools=on', '/settings worktree=yes']) await h.execute(command);
  assert.equal(h.prefs.get('blora-theme'), 'indigo');
  assert.equal(h.attributes['data-blora-theme'], 'indigo');
  assert.equal(h.prefs.get('blora-color-scheme'), 'dark');
  assert.equal(h.attributes['data-blora-color-scheme'], 'dark');
  assert.equal(h.prefs.get('blora-message-roles'), 'off');
  assert.equal(h.prefs.get('blora-hide-tools'), 'on');
  assert.equal(h.prefs.get('blora-worktree'), 'on');
  assert.equal(control.checked, true);
  assert.equal(control.value, 'on');
  assert.equal(h.calls.filter(call => call.kind === 'render').length, 5);
  assert.equal(h.calls.filter(call => call.kind === 'open').length, 0);
  await h.execute('/theme light');
  assert.equal(h.attributes['data-blora-color-scheme'], 'light');
  await h.execute('/settings scheme=auto');
  assert.equal(h.prefs.get('blora-color-scheme'), 'auto');
  assert.equal(h.attributes['data-blora-color-scheme'], 'dark');
});

test('invalid settings and delays are handled once without mutating preferences or creating tasks', async () => {
  const h = harness();
  for (const command of ['/settings unknown=on', '/settings hide-tools=invalid', '/settings theme=invalid', '/settings scheme=invalid', '/loop 0s prompt', '/loop 999999999999999d prompt']) assert.equal(await h.execute(command), true);
  assert.equal(h.calls.filter(call => call.kind === 'alert').length, 6);
  assert.equal(h.prefs.size, 0);
  assert.equal(h.requests().length, 0);
});

test('logout uses the existing UI click handler or a refresh bridge', async () => {
  const h = harness();
  h.controls['#passport-logout'] = { click: () => h.calls.push({ kind: 'logout-click' }) };
  await h.execute('/logout');
  assert.deepEqual(h.calls, [{ kind: 'logout-click' }]);
  const refresh = harness();
  refresh.bridge.refreshPassport = async () => refresh.calls.push({ kind: 'passport-refresh' });
  await refresh.execute('/logout');
  assert.equal(refresh.requests()[0].url, '/api/auth/logout');
  assert.ok(refresh.calls.some(call => call.kind === 'passport-refresh'));
});

test('forget confirms risk, cancels without deleting and sends confirmed true', async () => {
  const h = harness();
  h.approve(false);
  await h.execute('/forget key');
  assert.equal(h.requests().length, 0);
  h.approve(true);
  await h.execute('/forget key');
  assert.deepEqual(h.requests()[0].body, { action: 'forget', key: 'key', confirmed: true });
  assert.ok(h.calls.every(call => call.kind !== 'confirm' || call.risk === true));
});

test('distill, uninstall and init confirm mutations; users is read-only with no prompt', async () => {
  for (const [token, action] of [['flush', 'distill'], ['remove plugin', 'uninstall'], ['init', 'init'], ['users', 'users']]) {
    const h = harness();
    h.approve(false);
    await h.execute(`/${token}`);
    assert.equal(h.requests().length, action === 'users' ? 1 : 0);
    if (action === 'users') assert.equal(h.calls.filter(call => call.kind === 'confirm').length, 0);
    else {
      h.approve(true);
      await h.execute(`/${token}`);
    }
    assert.equal(h.requests()[0].url, '/api/sessions/ses_test/actions');
    assert.deepEqual(h.requests()[0].body, { action, ...(action === 'uninstall' ? { name: 'plugin' } : {}), confirmed: true });
    assert.ok(h.calls.some(call => call.kind === 'notice' && call.text === '操作输出'));
  }
});

test('missing confirmation export fails closed and mutations reject running sessions', async () => {
  const h = harness();
  delete h.workbench.confirm;
  await h.execute('/forget key');
  assert.equal(h.requests().length, 0);
  assert.ok(h.calls.some(call => call.kind === 'alert'));
  const running = harness();
  running.state.running = true;
  for (const command of ['/distill', '/init', '/uninstall plugin', '/forget key', '/checkpoint', '/model chosen']) await running.execute(command);
  assert.equal(running.requests().length, 0);
  assert.equal(running.calls.filter(call => call.kind === 'alert').length, 6);
});

test('plan toggles the read-only plan permission mode without creating a session', async () => {
  const h = harness();
  await h.execute('/plan');
  assert.equal(h.state.permission, 'plan');
  assert.equal(h.requests().length, 0);
  await h.execute('/plan');
  assert.equal(h.state.permission, 'default');
  assert.equal(await h.execute('/plan 参数'), true);
  assert.ok(h.calls.some(call => call.kind === 'alert'));
});

test('reasoning toggles the stored preference and re-renders', async () => {
  const h = harness();
  await h.execute('/reasoning');
  assert.equal(h.prefs.get('blora-show-reasoning'), 'on');
  await h.execute('/thinking');
  assert.equal(h.prefs.get('blora-show-reasoning'), 'off');
  assert.equal(h.calls.filter(call => call.kind === 'render').length, 2);
});

test('mode parameters create sessions; model aliases use routing; search and tools render locally', async () => {
  const h = harness();
  await h.execute('/mode work');
  assert.deepEqual(h.requests()[0].body, { mode: 'work', title: 'work' });
  await h.execute('/m chosen');
  assert.ok(h.requests().some(call => call.url.endsWith('/routing') && call.body.model === 'chosen'));
  await h.execute('/find-transcript query');
  await h.execute('/clear');
  assert.deepEqual(h.calls.filter(call => call.kind === 'search').map(call => call.query), ['query', '']);
  await h.execute('/tools');
  assert.equal(h.prefs.get('blora-hide-tools'), 'on');
  await h.execute('/tools');
  assert.equal(h.prefs.get('blora-hide-tools'), 'off');
});
