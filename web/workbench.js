(() => {
  "use strict";
  const dialogs = window.BloraDialogs;
  const bridge = () => {
    if (!window.BloraWeb) throw new Error('工作台桥尚未就绪，请刷新后重试。');
    return window.BloraWeb;
  };
  const state = () => bridge().getState();
  const sessionId = () => state().sessionId || state().session?.id || state().projection?.session?.id;
  const sessionPath = (suffix = '') => {
    let owner = dialogs.top(); while (owner && !owner.sessionScoped) owner = owner.parent;
    const id = owner ? owner.sessionId : sessionId(); if (owner && id !== sessionId()) throw new Error('会话已切换，请重新打开窗口。'); if (!id) throw new Error('请先选择会话。');
    return `/api/sessions/${encodeURIComponent(id)}${suffix}`;
  };
  const post = (path, body = {}) => bridge().api(path, { method: 'POST', body: JSON.stringify(body), silent: true });
  const rows = (data, key) => Array.isArray(data) ? data : data?.[key] || [];
  function node(tag, text, className) {
    const el = document.createElement(tag); if (text != null) el.textContent = String(text);
    if (className) el.className = className; return el;
  }
  function note(container, text) { const el = node('p', text, 'ba-window__note'); container.append(el); return el; }
  function output(container, text) { const el = node('pre', typeof text === 'string' ? text : JSON.stringify(text, null, 2), 'ba-window__output'); container.append(el); return el; }
  function button(container, label, action, primary = false) {
    const el = node('button', label, 'blora-button'); el.type = 'button'; el.dataset.variant = primary ? 'primary' : 'secondary';
    el.addEventListener('click', action); container.append(el); return el;
  }
  function field(container, label, { value = '', type = 'text', multiline = false, required = false } = {}) {
    const wrap = node('label', null, 'ba-window__field'); wrap.append(node('span', label));
    const input = node(multiline ? 'textarea' : 'input', null, multiline ? 'blora-textarea' : 'blora-input');
    if (!multiline) input.type = type; input.value = value; input.required = required;
    wrap.append(input); container.append(wrap); return input;
  }
  function check(container, label, checked = false) {
    const wrap = node('label', null, 'ba-window__check'); const input = node('input'); input.type = 'checkbox'; input.checked = checked;
    wrap.append(input, node('span', label)); container.append(wrap); return input;
  }
  function tabs(container, names) {
    const bar = node('div', null, 'ba-window__tabs'); bar.setAttribute('role', 'tablist'); container.append(bar);
    const panels = names.map((name, index) => {
      const panel = node('section'); panel.hidden = index !== 0; panel.setAttribute('role', 'tabpanel'); container.append(panel);
      const tab = button(bar, name, () => {
        panels.forEach((entry, i) => { entry.panel.hidden = i !== index; entry.tab.setAttribute('aria-selected', String(i === index)); entry.tab.tabIndex = i === index ? 0 : -1; });
      });
      tab.setAttribute('role', 'tab'); tab.setAttribute('aria-selected', String(index === 0)); tab.tabIndex = index === 0 ? 0 : -1;
      tab.addEventListener('keydown', event => {
        if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
          event.preventDefault(); const next = panels[(index + (event.key === 'ArrowRight' ? 1 : -1) + names.length) % names.length].tab; next.click(); next.focus();
        }
      });
      return { panel, tab };
    });
    return panels.map(entry => { entry.panel.activate = () => entry.tab.click(); return entry.panel; });
  }
  function error(win, err) {
    if (win.closed) return;
    if (!win.error) { win.error = node('p', null, 'ba-window__error'); win.error.setAttribute('role', 'alert'); win.body.prepend(win.error); }
    win.error.textContent = err.message || String(err);
  }
  async function load(win, action, render, scoped = true) {
    const id = sessionId(); const version = (win.versions ||= new Map()).get(action.channel || 'default') || 0;
    const channel = action.channel; if (channel) win.versions.set(channel, version + 1);
    const status = note(win.body, '加载中…');
    try {
      const data = await action();
      if (win.closed || channel && win.versions.get(channel) !== version + 1) { status.remove(); return; }
      if (scoped && sessionId() !== id) { status.textContent = '会话已切换，请关闭后重新打开。'; return; }
      status.remove(); render(data);
    } catch (err) { status.remove(); error(win, err); }
  }
  function action(win, container, label, callback, primary = false) {
    return button(container, label, async event => {
      const el = event.currentTarget; if (win.busy) return; win.busy = true; el.disabled = true;
      const id = sessionId();
      try {
        if (win.sessionScoped && id !== win.sessionId) throw new Error('会话已切换，请重新打开此窗口。');
        await callback();
      } catch (err) { error(win, err); }
      finally { win.busy = false; el.disabled = false; }
    }, primary);
  }
  async function confirm(title, text, risk = false) {
    const win = dialogs.open(`confirm:${title}`, { title });
    if (!win.body.childNodes.length) {
      note(win.body, text);
      const ack = risk ? check(win.body, '我理解风险，并明确授权本次操作') : null;
      const accept = button(win.body, '确认', () => win.close(true), true);
      if (ack) { accept.disabled = true; ack.addEventListener('change', () => { accept.disabled = !ack.checked; }); }
      button(win.body, '取消', () => win.close(false));
    }
    return Boolean(await win.result);
  }
  const permissions = { ask: 'default', default: 'default', 'auto-edit': 'acceptEdits', acceptEdits: 'acceptEdits', plan: 'plan', yolo: 'bypassPermissions', bypassPermissions: 'bypassPermissions' };
  async function requestPermission(mode) {
    const canonical = permissions[mode]; if (!canonical) return false;
    if (canonical !== 'bypassPermissions') return true;
    const id = sessionId();
    const allowed = await confirm('全部允许：风险确认', '将绕过审批，允许工具写入、删除文件及执行命令。这可能导致数据丢失或泄露。任务的自动批准是独立授权，外部内核仍受自身权限边界约束。取消不会改变当前权限。', true);
    return allowed && sessionId() === id;
  }
  function sessionDialog(win) {
    const search = field(win.body, '搜索会话'); const list = node('div', null, 'ba-window__list'); win.body.append(list);
    action(win, win.body, '新会话', async () => {
      const created = await post('/api/sessions'); await bridge().refreshSessions(); await bridge().selectSession(created.id); win.close(true);
    }, true);
    const refresh = () => { const query = search.value; const fetch = () => bridge().api(`/api/sessions?q=${encodeURIComponent(query)}`, { silent: true }); fetch.channel = 'search'; return load(win, fetch, data => {
      list.replaceChildren(); const items = rows(data, 'sessions'); if (!items.length) note(list, '没有匹配的会话。');
      items.forEach(item => {
        const row = node('div', null, 'ba-window__row'); list.append(row);
        action(win, row, `${item.title || item.id} · ${item.mode || ''} · ${item.status || ''}`, async () => { await bridge().selectSession(item.id); win.close(true); });
        action(win, row, item.status === 'archived' ? '恢复' : '归档', async () => {
          await post(`/api/sessions/${encodeURIComponent(item.id)}/${item.status === 'archived' ? 'resume' : 'archive'}`); await bridge().refreshSessions(); refresh();
        });
      });
    }, false); };
    let timer; search.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(refresh, 180); });
    win.controller.signal.addEventListener('abort', () => clearTimeout(timer)); refresh();
  }
  function workspaceDialog(win, options) {
    const path = field(win.body, '目录（限服务端允许范围）', { value: options.path || '' });
    const list = node('div', null, 'ba-window__list'); win.body.append(list);
    const browse = () => load(win, () => bridge().api(`/api/directories?path=${encodeURIComponent(path.value || '.')}`, { silent: true }), data => {
      list.replaceChildren(); path.value = data.path || path.value;
      if (data.parent) button(list, '上一级', () => { path.value = data.parent; browse(); });
      const directories = data.directories || [];
      const fileEntries = data.files || [];
      if (!directories.length && !fileEntries.length) note(list, '目录为空。');
      for (const entry of directories) {
        const name = typeof entry === 'string' ? entry : entry.path || entry.name;
        button(list, `目录 · ${entry.name || name}`, () => { path.value = name.startsWith('/') ? name : `${data.path}/${name}`; browse(); });
      }
      for (const entry of fileEntries) {
        const name = typeof entry === 'string' ? entry : entry.path || entry.name;
        button(list, `文件 · ${entry.name || name}`, () => open('file', { path: name.startsWith('/') ? name : `${data.path}/${name}` }));
      }
    });
    action(win, win.body, '浏览', async () => browse());
    action(win, win.body, '查看当前会话文件', async () => {
      const data = await bridge().api(`/api/workspace${sessionId() ? `?session=${encodeURIComponent(sessionId())}` : ''}`, { silent: true });
      if (win.closed) return; list.replaceChildren(); note(list, data.path);
      for (const entry of data.entries || []) button(list, entry, () => entry.endsWith('/') ? open('folder', { path: `${data.path}/${entry}` }) : open('file', { path: entry }));
    });
    action(win, win.body, '在此项目新建会话', async () => {
      if (!path.value.trim()) throw new Error('请选择目录。');
      const created = await post('/api/sessions', { workspace: path.value.trim() });
      await bridge().refreshSessions(); await bridge().selectSession(created.id); win.close(true);
    }, true); browse();
  }
  function modeDialog(win) {
    for (const [mode, label] of [['code', 'Code · 编码：读改代码、测试与 Git'], ['work', 'Work · 工作产出：联网调研、读文档、产出报告'], ['agent', 'Agent · 自主助理：委派、定时、自学技能与记忆'], ['imagine', 'Imagine · 图像']]) {
      action(win, win.body, label, async () => {
        if (state().running) throw new Error('运行中不能切换模式。');
        if (sessionId()) await post(sessionPath('/mode'), { mode });
        await bridge().applyRunMode(mode); if (sessionId()) await bridge().selectSession(sessionId()); win.close(true);
      });
    }
  }
  function harnessDialog(win) {
    load(win, () => bridge().api('/api/harnesses', { silent: true }), data => {
      const items = rows(data, 'harnesses'); if (!items.length) note(win.body, '没有可用内核，请先在服务端安装。');
      items.forEach(item => {
        note(win.body, `${item.name || item.id} · ${item.installed === false ? '未安装' : '已安装'} · 实际模型：${item.model || '由内核配置'} · 能力：${JSON.stringify(item.capabilities || { provider_selection: item.provider_selection, imagine: item.imagine, permissions: item.permissions })}`);
        const choose = action(win, win.body, `使用 ${item.name || item.id}`, async () => {
          if (state().running) throw new Error('运行中不能切换内核。');
          await post(sessionPath('/harness'), { harness: item.id }); await bridge().selectSession(sessionId()); win.close(true);
        }); choose.disabled = item.installed === false || state().running;
      });
      note(win.body, '外部 CLI 使用自身认证、模型配置及权限边界。');
    });
  }
  function providerDialog(win, modelOnly = false, options = {}) {
    const provider = options.provider || bridge().stored('blora-provider');
    if (!modelOnly) button(win.body, '新增供应商', () => open('add-provider'));
    const endpoint = modelOnly ? `/api/providers/${encodeURIComponent(provider || 'default')}/models` : '/api/providers';
    load(win, () => bridge().api(endpoint, { silent: true }), data => {
      const items = rows(data, modelOnly ? 'models' : 'providers');
      if (!items.length) { note(win.body, modelOnly ? '没有可用模型，请检查凭据或供应商配置。' : '没有已配置的供应商。'); return; }
      items.forEach(item => {
        const id = typeof item === 'string' ? item : item.id || item.name;
        const select = action(win, win.body, `${item.display_name || item.name || id}${(item.has_credentials === false || item.has_api_key === false || item.available === false) ? ' · 缺少凭据' : ''}`, async () => {
          if (state().running) throw new Error('运行中不能切换供应商或模型。');
          if (!modelOnly) { bridge().store('blora-provider', id); open('model', { provider: id }); }
          else { if (sessionId()) { await post(sessionPath('/routing'), { provider: provider || 'blora', model: id }); await bridge().selectSession(sessionId()); } bridge().store('blora-model', id); if (provider) bridge().store('blora-provider', provider); await bridge().refreshSettings(); win.close(true); }
        }); select.disabled = (item.has_credentials === false || item.has_api_key === false || item.available === false) || state().running;
      });
    });
  }
  function addProvider(win) {
    const name = field(win.body, '供应商 ID', { required: true });
    const display = field(win.body, '显示名称', { required: true });
    const format = node('select', null, 'blora-input'); format.setAttribute('aria-label', '消息格式');
    for (const value of ['openai', 'anthropic', 'gemini', 'responses']) { const option = node('option', value); option.value = value; format.append(option); } win.body.append(format);
    const base = field(win.body, 'API 地址（HTTPS）', { type: 'url', required: true });
    const model = field(win.body, '默认模型');
    const key = field(win.body, 'API 密钥（仅发送至服务端）', { type: 'password', required: true }); key.autocomplete = 'new-password';
    note(win.body, '密钥不会写入浏览器存储，也不会在响应中展示。');
    action(win, win.body, '保存供应商', async () => {
      if (!name.value.trim() || !display.value.trim() || !key.value || !base.reportValidity()) throw new Error('请填写供应商、API 地址和密钥。');
      if (new URL(base.value).protocol !== 'https:') throw new Error('API 地址必须使用 HTTPS。');
      try { await post('/api/providers', { id: name.value.trim(), name: display.value.trim(), base_url: base.value, format: format.value, models: model.value ? [{ id: model.value, name: model.value }] : [], api_key: key.value }); }
      catch (err) { throw new Error(`供应商保存失败${err.status ? `（HTTP ${err.status}）` : ''}，请检查配置或服务端日志。`); } finally { key.value = ''; }
      await win.close(true); if (dialogs.top()?.name === 'provider') { await dialogs.close(); open('provider'); }
    }, true);
    win.onClose = () => { key.value = ''; };
  }
  function permissionDialog(win) {
    note(win.body, `当前权限：${permissions[state().permission || state().session?.permission_mode || state().projection?.session?.permission_mode || bridge().stored('blora-permission', 'default')] || 'default'}`);
    for (const [mode, label] of [['default', '默认确认'], ['plan', '计划（只读）'], ['acceptEdits', '自动接受编辑'], ['bypassPermissions', '跳过权限确认']]) action(win, win.body, label, async () => {
      if (!await requestPermission(mode)) return;
      await bridge().applyPermission(mode); win.close(true);
    });
  }
  function themeDialog(win) {
    const root = document.documentElement;
    const original = { theme: root.getAttribute('data-blora-theme'), scheme: root.getAttribute('data-blora-color-scheme'), color: root.style.colorScheme };
    const preview = { ...original }; let saved = false;
    win.onClose = () => {
      if (saved) return;
      if (original.theme == null) root.removeAttribute('data-blora-theme'); else root.setAttribute('data-blora-theme', original.theme);
      if (original.scheme == null) root.removeAttribute('data-blora-color-scheme'); else root.setAttribute('data-blora-color-scheme', original.scheme);
      root.style.colorScheme = original.color;
    };
    note(win.body, '实时预览；关闭或取消会回滚。');
    const palettes = node('div', null, 'ba-window__grid'); win.body.append(palettes);
    for (const theme of ['coral', 'indigo', 'graphite', 'mono', 'circuit', 'dusk']) button(palettes, theme, () => {
      preview.theme = theme; root.setAttribute('data-blora-theme', theme);
    });
    for (const [scheme, label] of [['light', '浅色'], ['dark', '深色']]) button(win.body, label, () => {
      preview.scheme = scheme; root.setAttribute('data-blora-color-scheme', scheme); root.style.colorScheme = scheme;
    });
    button(win.body, '保存主题', () => {
      bridge().store('blora-theme', preview.theme); bridge().store('blora-color-scheme', preview.scheme); saved = true; win.close(true);
    }, true); button(win.body, '取消', () => win.close(false));
  }
  function settingsDialog(win) {
    const [general, appearance, model] = tabs(win.body, ['常规', '外观', '模型']);
    button(general, '权限', () => open('permission')); button(general, '工作模式', () => open('mode')); button(general, '执行内核', () => open('harness'));
    const worktree = check(general, '在 Git worktree 中执行', bridge().stored('blora-worktree') === 'on');
    const worktreeControl = document.querySelector('#pref-worktree');
    worktree.checked = worktreeControl ? (worktreeControl.checked || worktreeControl.getAttribute('value') === 'on') : worktree.checked;
    worktree.addEventListener('change', () => { bridge().store('blora-worktree', worktree.checked ? 'on' : 'off'); if (worktreeControl) { worktreeControl.setAttribute('value', worktree.checked ? 'on' : 'off'); worktreeControl.checked = worktree.checked; } });
    button(general, 'PassPort 账号', () => open('passport')); button(general, '首次使用引导', () => { win.close(); bridge().enterOnboarding(); });
    button(appearance, '主题预览', () => open('theme'));
    for (const [key, label] of [['blora-message-roles', '显示消息角色'], ['blora-hide-tools', '隐藏工具摘要']]) {
      const input = check(appearance, label, key === 'blora-hide-tools' ? bridge().stored(key) === 'on' : bridge().stored(key, 'on') !== 'off');
      input.addEventListener('change', () => { bridge().store(key, input.checked ? 'on' : 'off'); bridge().renderTranscript?.(); window.dispatchEvent(new CustomEvent('blora:preferences-changed', { detail: { key, value: input.checked } })); });
    }
    button(model, '选择供应商', () => open('provider')); button(model, '选择模型', () => open('model')); button(model, '新增供应商', () => open('add-provider'));
    load(win, () => bridge().api('/api/settings', { silent: true }), data => {
      note(model, `默认供应商：${data.provider_display || data.provider} · 模型：${data.model} · 凭据：${data.has_api_key ? '已配置' : '未配置'}`);
      note(general, `执行环境：${data.exec || '-'} · MCP：${data.mcp ? '已配置' : '未配置'}`);
    }, false);
  }
  function contextDialog(win) {
    const search = field(win.body, '搜索上下文'); const result = node('div'); win.body.append(result);
    const projection = state().projection || state().session || {};
    output(result, projection);
    action(win, win.body, '搜索', async () => {
      const data = await bridge().api(`${sessionPath('/context')}?q=${encodeURIComponent(search.value)}`, { silent: true });
      if (!win.closed) { result.replaceChildren(); const query = search.value.toLowerCase(); output(result, query ? { ...data, transcript: (data.transcript || []).filter(item => JSON.stringify(item).toLowerCase().includes(query)) } : data); }
    });
    action(win, win.body, '压缩上下文', async () => {
      if (await confirm('压缩上下文', '创建摘要产物并压缩当前会话上下文？')) { await post(sessionPath('/compact')); await bridge().selectSession(sessionId()); win.close(true); }
    });
    action(win, win.body, '创建检查点', async () => { await post(sessionPath('/checkpoint')); note(win.body, '检查点已创建。'); });
  }
  function gitDialog(win) {
    const [operations, graph, logs] = tabs(win.body, ['操作', 'Git 图', '输出']);
    const files = node('div', null, 'ba-window__list'); operations.append(files);
    const diff = output(operations, '选择文件查看完整差异。'); diff.classList.add('ba-git-diff');
    const message = field(operations, '提交说明', { multiline: true });
    const log = output(logs, '尚未执行操作。'); const topology = output(graph, '加载 Git 图…');
    let selected = new Set(); let fileItems = []; let diffVersion = 0;
    const selectedPaths = () => [...new Set(fileItems.filter(item => selected.has(item.path)).flatMap(item => item.old_path ? [item.old_path, item.path] : [item.path]))];
    note(graph, '服务端 git log --graph 保留分支与合并拓扑。');
    const refresh = () => load(win, () => bridge().api(sessionPath('/git/files'), { silent: true }), data => {
      files.replaceChildren();
      fileItems = (data.files || []).map(item => typeof item === 'string' ? { path: item } : item);
      selected = new Set(fileItems.map(item => item.path));
      if (!fileItems.length) note(files, '没有变更文件。');
      fileItems.forEach(item => {
        const path = item.path;
        const row = node('div', null, 'ba-window__row ba-git-file'); row.tabIndex = 0; files.append(row);
        const input = check(row, `${item.status || ''} ${item.old_path ? `${item.old_path} → ` : ''}${path}`, true);
        input.addEventListener('change', () => input.checked ? selected.add(path) : selected.delete(path));
        const toggle = () => { input.checked = !input.checked; input.dispatchEvent(new Event('change')); };
        const showDiff = async () => {
          const version = ++diffVersion; diff.textContent = '加载差异…';
          files.querySelectorAll('.ba-git-file').forEach(entry => entry.classList.toggle('ba-git-file--active', entry === row));
          try {
            const data = await bridge().api(`${sessionPath('/git/diff')}?file=${encodeURIComponent(path)}`, { silent: true });
            if (!win.closed && version === diffVersion && sessionId() === win.sessionId) {
              diff.replaceChildren(); const text = typeof data === 'string' ? data : data.diff || data.output || '';
              for (const line of text.split('\n')) diff.append(node('span', `${line}\n`, line.startsWith('+') ? 'ba-git-diff__added' : line.startsWith('-') ? 'ba-git-diff__removed' : ''));
            }
          } catch (err) { if (!win.closed && version === diffVersion) { diff.textContent = err.message; error(win, err); } }
        };
        row.addEventListener('click', event => { if (event.target !== input) event.preventDefault(); showDiff(); });
        row.addEventListener('dblclick', event => { if (event.target !== input) { event.preventDefault(); toggle(); } });
        row.addEventListener('keydown', event => {
          if (event.target === input) return;
          if (event.key === ' ') { event.preventDefault(); toggle(); }
          else if (event.key === 'Enter') { event.preventDefault(); showDiff(); }
        });
      });
    });
    load(win, () => bridge().api(sessionPath('/git/log'), { silent: true }), data => {
      topology.textContent = typeof data === 'string' ? data : data.graph || data.log || data.output || JSON.stringify(data, null, 2);
    });
    action(win, operations, '生成提交说明', async () => {
      if (!selected.size) throw new Error('请选择至少一个文件。');
      const data = await post(sessionPath('/git/message'), { files: selectedPaths() }); if (!win.closed) message.value = data.message || data.output || '';
    });
    const execute = async (operation, body, warning) => {
      if (!await confirm(`Git ${operation}`, warning, true)) return;
      try {
        const data = await post(sessionPath(`/git/${operation === 'push' || operation === 'pull' ? 'sync' : operation}`), { ...body, confirmed: true, ...(['push', 'pull'].includes(operation) ? { direction: operation } : {}) });
        if (!win.closed) { log.textContent = data.output || JSON.stringify(data, null, 2); refresh(); }
      } catch (err) { if (!win.closed) { log.textContent = err.message; logs.activate(); } throw err; }
    };
    action(win, operations, '提交选中文件', async () => {
      if (!selected.size || !message.value.trim()) throw new Error('请选择文件并填写提交说明。');
      await execute('commit', { files: selectedPaths(), message: message.value }, '仅提交所选文件，未选变更应保留。确认创建 Git 提交？');
    }, true);
    action(win, operations, '推送', () => execute('push', {}, '显式推送到已配置的 upstream；不会强制推送。'));
    action(win, operations, '快进拉取', () => execute('pull', { ff_only: true }, '仅执行 fast-forward 拉取；脏状态或分叉时应拒绝。'));
    button(operations, '刷新', refresh); refresh();
  }
  function tasksDialog(win) {
    const [list, create] = tabs(win.body, ['任务列表', '新建任务']);
    const title = field(create, '标题', { required: true }); const prompt = field(create, '提示词', { multiline: true, required: true });
    const delay = field(create, '延迟秒数', { type: 'number', value: '0' }); delay.min = '0';
    const cron = field(create, 'Cron（可选，周期调度）');
    const auto = check(create, '任务自动批准（独立权限，需明确确认）');
    note(create, '延迟秒数对应 CLI /loop 的一次性延迟任务，不是周期循环。周期调度使用 Cron；任务自动批准不继承会话权限。');
    const refresh = () => load(win, () => bridge().api('/api/tasks', { silent: true }), data => {
      list.replaceChildren(); const items = rows(data, 'tasks'); if (!items.length) note(list, '没有任务。');
      items.forEach(item => {
        const row = node('div', null, 'ba-window__row'); list.append(row);
        note(row, `${item.title || item.id} · ${item.status} · ${item.cron || ''}`);
        for (const [operation, label] of [['pause', '暂停'], ['resume', '恢复'], ['cancel', '取消']]) action(win, row, label, async () => {
          if (operation === 'cancel' && !await confirm('取消任务', `取消 ${item.title || item.id}？`)) return;
          await post(`/api/tasks/${encodeURIComponent(item.id)}/${operation}`); refresh();
        });
      });
    }, false);
    action(win, create, '创建任务', async () => {
      if (!sessionId()) throw new Error('请先选择会话。');
      if (!title.value.trim() || !prompt.value.trim() || !delay.reportValidity()) throw new Error('请填写标题、提示词与有效时间。');
      if (auto.checked && !await confirm('任务自动批准', '此任务将独立自动批准工具操作，可能写入、删除文件或执行命令。', true)) return;
      await post('/api/tasks', { session_id: sessionId(), title: title.value.trim(), prompt: prompt.value.trim(), delay_seconds: Number(delay.value), cron: cron.value.trim() || null, auto_approve: auto.checked, confirmed: auto.checked });
      title.value = ''; prompt.value = ''; refresh();
    }, true);
    action(win, win.body, '执行到期任务（Pump）', async () => {
      if (await confirm('执行到期任务', '立即触发服务端已到期任务？')) { await post('/api/tasks/pump'); refresh(); }
    }); refresh();
  }
  function memoryDialog(win) {
    const list = node('div', null, 'ba-window__list'); win.body.append(list);
    const key = field(win.body, '记忆键', { required: true });
    const value = field(win.body, '记忆内容', { multiline: true });
    const refresh = () => load(win, () => bridge().api(sessionPath('/memory'), { silent: true }), data => {
      list.replaceChildren(); const items = rows(data, 'memories'); if (!items.length) note(list, '没有持久记忆。');
      items.forEach(item => button(list, item.key, () => { key.value = item.key; value.value = item.value; }));
    });
    action(win, win.body, '保存记忆', async () => {
      if (!key.value.trim()) throw new Error('请输入记忆键。');
      if (await confirm('保存记忆', '更新当前项目的持久记忆？')) { await post(sessionPath('/memory'), { action: 'remember', key: key.value, value: value.value }); refresh(); }
    }, true);
    action(win, win.body, '删除记忆', async () => {
      if (!key.value.trim()) throw new Error('请选择记忆键。');
      if (await confirm('删除记忆', `删除 ${key.value}？`)) { await post(sessionPath('/memory'), { action: 'forget', key: key.value }); value.value = ''; refresh(); }
    }); refresh();
  }
  function pluginsDialog(win) {
    const [installed, market, integrations] = tabs(win.body, ['插件 / 技能', '市场', 'MCP / Hook']);
    load(win, () => bridge().api(sessionPath('/extensions'), { silent: true }), data => {
      const items = rows(data, 'plugins'); if (!items.length) note(installed, '未安装插件。');
      items.forEach(item => note(installed, typeof item === 'string' ? item : JSON.stringify(item)));
      output(integrations, { mcp_configured: data.mcp_configured, hooks_configured: data.hooks_configured, skills: data.skills || [] });
    });
    load(win, () => bridge().api('/api/marketplace', { silent: true }), data => {
      const items = rows(data, 'plugins').length ? rows(data, 'plugins') : rows(data, 'items');
      items.forEach(item => action(win, market, `安装 ${item.name}`, async () => {
        if (!await confirm('安装插件', `安装 ${item.name}？插件可能执行代码。`, true)) return;
        await post('/api/marketplace', { name: item.name }); note(market, `${item.name} 已安装。`);
      })); if (!items.length) note(market, '市场没有可用插件。');
    });
  }
  const titles = { session: '会话', project: '项目', folder: '目录', file: '文件', mode: '工作模式', harness: '执行内核', provider: '供应商', model: '模型', 'add-provider': '新增供应商', permission: '权限', theme: '主题预览', settings: '设置', context: '上下文', tools: '工具', 'tool-detail': '工具详情', 'reasoning-detail': '完整思考过程', git: 'Git', tasks: '任务', memory: '记忆', plugins: '插件与集成', diagnostics: '诊断', help: '帮助', passport: 'PassPort 账号' };
  const aliases = { sessions: 'session', projects: 'project', files: 'folder', permissions: 'permission', models: 'model', providers: 'provider', 'provider-add': 'add-provider', 'tool-details': 'tool-detail', 'run-mode': 'mode', 'git-operations': 'git' };
  function open(name, options = {}) {
    name = aliases[name] || name;
    if (!titles[name]) { bridge().showAlert(`未知工作台窗口：${name}`); return null; }
    const win = dialogs.open(name, { title: titles[name], parent: options.parent });
    if (win.initialized) return win; win.initialized = true;
    win.sessionId = sessionId(); win.sessionScoped = ['mode', 'harness', 'permission', 'context', 'git', 'memory', 'tools', 'plugins', 'diagnostics', 'provider', 'model', 'project', 'folder', 'file'].includes(name);
    try {
      switch (name) {
        case 'session': sessionDialog(win); break;
        case 'project': case 'folder': workspaceDialog(win, options); break;
        case 'file': load(win, async () => {
          let path = options.path || '';
          if (path.startsWith('/')) {
            const workspace = state().session?.workspace_path || state().projection?.session?.workspace_path || (await bridge().api(`/api/workspace${sessionId() ? `?session=${encodeURIComponent(sessionId())}` : ''}`, { silent: true })).path;
            const prefix = `${workspace.replace(/\/$/, '')}/`;
            if (!path.startsWith(prefix)) throw new Error('文件不在当前会话工作区内，请先切换项目。');
            path = path.slice(prefix.length);
          }
          return bridge().api(`/api/workspace/file?path=${encodeURIComponent(path)}${sessionId() ? `&session=${encodeURIComponent(sessionId())}` : ''}`, { silent: true });
        }, data => output(win.body, data.contents || '')); break;
        case 'mode': modeDialog(win); break;
        case 'harness': harnessDialog(win); break;
        case 'provider': providerDialog(win); break;
        case 'model': providerDialog(win, true, options); break;
        case 'add-provider': addProvider(win); break;
        case 'permission': permissionDialog(win); break;
        case 'theme': themeDialog(win); break;
        case 'settings': settingsDialog(win); break;
        case 'context': contextDialog(win); break;
        case 'reasoning-detail':
          output(win.body, options.text || '没有思考过程。');
          break;
        case 'tool-detail':
          output(win.body, options.text || '没有工具输出。');
          if (sessionId()) load(win, () => bridge().api(sessionPath('/context'), { silent: true }), data => {
            const tools = (data.transcript || []).filter(item => item.kind === 'tool' || item.type === 'tool' || item.Tool || item.name);
            if (tools.length) output(win.body, tools);
          });
          break;
        case 'tools': load(win, () => bridge().api(sessionPath('/extensions'), { silent: true }), data => {
          const items = data.tools || data.plugins || []; if (!items.length) note(win.body, '没有可用工具。');
          items.forEach(item => button(win.body, item.name || item.id, () => open('tool-detail', { text: JSON.stringify(item, null, 2) })));
        }); break;
        case 'git': gitDialog(win); break;
        case 'tasks': tasksDialog(win); break;
        case 'memory': memoryDialog(win); break;
        case 'plugins': pluginsDialog(win); break;
        case 'diagnostics': load(win, () => bridge().api(sessionPath('/diagnostics'), { silent: true }), data => output(win.body, data)); break;
        case 'passport': load(win, () => bridge().api('/api/auth/me', { silent: true }), data => note(win.body, data.name || data.username || '尚未连接'), false);
          button(win.body, '连接账号 / 引导', () => { win.close(); bridge().enterOnboarding(); }); break;
        case 'help':
          output(win.body, (window.BloraCommands?.catalog || []).filter(item => !options.query || `${item.name} ${item.about}`.includes(options.query)).map(item => `/${item.name} ${item.hint} · ${item.about}`).join('\n'));
          note(win.body, 'Esc 关闭顶层窗口；Tab 在顶层循环；最小化后 Enter 恢复。双击工具摘要查看详情。Git 复选框可触控或用空格选择。');
          note(win.body, '浏览器关闭页面不会终止服务端。CLI 的退出、终端键位与启动 Web 命令不适用于浏览器。');
          for (const target of ['session', 'project', 'mode', 'harness', 'provider', 'permission', 'settings', 'context', 'tools', 'git', 'tasks', 'memory', 'plugins', 'diagnostics']) button(win.body, titles[target], () => open(target));
           break;
      }
    } catch (err) { error(win, err); }
    return win;
  }
  document.addEventListener('click', event => {
    const entry = event.target.closest('[data-workbench]'); if (!entry) return;
    event.preventDefault(); entry.closest('details')?.removeAttribute('open'); open(entry.dataset.workbench);
  });
  let gitState = { id: null, label: '未选择', updated: 0, busy: false };
  let stateSignature = '';
  function refreshStatus() {
    if (!window.BloraWeb) return;
    const current = state(); const session = current.session || current.projection?.session || {};
    const projection = current.projection || session;
    const id = sessionId(); const harness = projection.harness || session.harness || 'blora';
    const permission = permissions[current.permission || projection.permission_mode || session.permission_mode || bridge().stored('blora-permission', 'default')];
    const labels = { default: '默认确认', plan: '计划', acceptEdits: '自动接受编辑', bypassPermissions: '跳过权限确认' };
    const model = projection.external_model || session.external_model || projection.model || session.model || (id ? '默认模型（未报告）' : '未选择');
    const values = { harness: `内核 · ${id ? harness : '未选择'}`, model: `模型 · ${model}`, permission: `权限 · ${labels[permission] || '默认确认'}`, git: `Git · ${gitState.id === id ? gitState.label : id ? '加载中' : '未选择'}` };
    for (const [key, value] of Object.entries(values)) {
      const entry = document.querySelector(`[data-status="${key}"]`);
      if (entry && entry.textContent !== value) { entry.textContent = value; entry.title = value; entry.setAttribute('aria-label', value); }
    }
    const signature = `${id}:${projection.last_sequence || session.last_sequence || 0}:${current.running}`;
    if (!id || gitState.busy || (signature === stateSignature && Date.now() - gitState.updated < 15000)) return;
    stateSignature = signature; gitState.busy = true;
    bridge().api(`/api/sessions/${encodeURIComponent(id)}/git/status`, { silent: true }).then(data => {
      if (sessionId() !== id) return;
      gitState.id = id;
      gitState.label = `${data.branch || '未命名分支'} · ${data.clean ? '干净' : `+${data.added || 0} −${data.removed || 0}`}${data.ahead ? ` ↑${data.ahead}` : ''}${data.behind ? ` ↓${data.behind}` : ''}`;
    }).catch(() => { if (sessionId() === id) { gitState.id = id; gitState.label = '不可用'; } }).finally(() => { gitState.updated = Date.now(); gitState.busy = false; refreshStatus(); });
  }
  setInterval(refreshStatus, 800);
  window.addEventListener('hashchange', refreshStatus);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) refreshStatus(); });
  refreshStatus();
  window.BloraWorkbench = Object.freeze({ open, requestPermission, confirm });
})();
