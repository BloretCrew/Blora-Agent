// SPDX-License-Identifier: GPL-3.0-or-later
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { test } = require('node:test');
const source = fs.readFileSync('web/app.js', 'utf8');
function extract(name, next) { return source.slice(source.indexOf(`async function ${name}(`), source.indexOf(next, source.indexOf(`async function ${name}(`))); }
function deferred() { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; }
test('late session selection cannot close the current event stream; bypass requires fresh session authorization', async () => {
  const delayed = deferred(); const streams = [];
  const state = { sessionId: 'A', permission: 'yolo', selectionVersion: 0 };
  const context = vm.createContext({ state, api: async path => ({ id: path.split('/').pop(), permission_mode: null, mode: 'code', run_status: null }),
    hideAlert() {}, setRunning() {}, window: { dispatchEvent() {} }, CustomEvent: function () {},
    title: {}, pathEl: {}, sessionTitle: s => s.id, applyPermission: mode => { state.permission = mode; }, applyRunMode() {},
    renderUsage() {}, renderPlan() {}, renderQuestion() {}, renderTranscript() {}, renderSubagents() {}, refreshTasks: async () => {}, refreshSessions: async () => {},
    refreshApprovals: () => state.sessionId === 'A' ? delayed.promise : Promise.resolve(), showAlert() {},
    EventSource: function (url) { this.url = url; this.closed = false; this.close = () => { this.closed = true; }; streams.push(this); }, handleSessionEvent() {},
  });
  vm.runInContext(extract('selectSession', '\n// Apply one live event'), context);
  const old = context.selectSession('A');
  await new Promise(r => setImmediate(r));
  await context.selectSession('B');
  const current = state.source;
  assert.equal(state.permission, 'ask');
  delayed.resolve(); await old;
  assert.equal(state.source, current); assert.equal(current.closed, false); assert.equal(streams.length, 1);
});
test('late approvals cannot overwrite the currently selected session', async () => {
  const response = deferred(); const rendered = [];
  const state = { sessionId: 'A', selectionVersion: 1 };
  const context = vm.createContext({ state, document: { querySelector: () => ({ replaceChildren() {}, append() {}, hidden: true }) },
    api: () => response.promise, approvalList: { replaceChildren: () => rendered.push('clear'), append: row => rendered.push(row) },
    approvalEmpty: {}, listOrEmpty() {}, approvalItem: row => row.id, enhance() {},
  });
  vm.runInContext(extract('refreshApprovals', '\nasync function refreshApprovalPage'), context);
  const old = context.refreshApprovals(); state.sessionId = 'B'; state.selectionVersion++;
  response.resolve([{ id: 'approval-A', session_id: 'A' }]); await old;
  assert.deepEqual(rendered, []);
});
test('question card answers the session it was shown for, even after switching', async () => {
  const response = deferred(); const calls = [];
  const nodes = [];
  function element(tag) {
    const node = { tag, children: [], dataset: {}, hidden: false, disabled: false, value: '', listeners: {},
      append(...items) { this.children.push(...items); }, replaceChildren(...items) { this.children = items; },
      addEventListener(type, fn) { this.listeners[type] = fn; }, setAttribute() {}, focus() {},
      querySelectorAll() { return this.children.filter(child => child.tag === 'button'); } };
    nodes.push(node); return node;
  }
  const host = element('div'); host.hidden = true;
  const state = { sessionId: 'A' };
  const context = vm.createContext({ state, document: { querySelector: selector => selector === '#session-question' ? host : null, createElement: element },
    api: (url, options) => { calls.push({ url, body: JSON.parse(options.body) }); return response.promise; }, showAlert() {} });
  vm.runInContext(source.slice(source.indexOf('function renderQuestion('), source.indexOf('\nfunction renderGallery(')), context);
  context.renderQuestion({ pending_input: { request_id: 'ask_A', question: 'Q?', options: ['Yes', 'No'] } });
  assert.equal(host.hidden, false);
  const yes = nodes.find(node => node.tag === 'button' && node.textContent === 'Yes');
  yes.listeners.click();
  assert.deepEqual(calls, [{ url: '/api/sessions/A/input', body: { request_id: 'ask_A', answer: 'Yes' } }]);
  state.sessionId = 'B';
  context.renderQuestion({ pending_input: { request_id: 'ask_B', question: 'Other?', options: [] } });
  response.resolve({}); await new Promise(r => setImmediate(r));
  assert.equal(host.hidden, false, 'late answer for A does not hide B\'s question');
  assert.equal(host.dataset.request, 'ask_B');
  context.renderQuestion({ pending_input: null });
  assert.equal(host.hidden, true);
});
