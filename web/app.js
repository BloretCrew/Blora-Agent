const state = {
  sessionId: null,
  view: "session",
  source: null,
  query: "",
  running: false,
  streaming: null,
  stick: true,
};

const sessionEmpty = document.querySelector("#session-empty");
const thread = document.querySelector("#thread");
const threadEmpty = document.querySelector("#thread-empty");
const title = document.querySelector("#session-title");
const pathEl = document.querySelector("#session-path");
const prompt = document.querySelector("#prompt");
const autoApprove = document.querySelector("#auto-approve");
const taskList = document.querySelector("#task-list");
const taskEmpty = document.querySelector("#task-empty");
const subagents = document.querySelector("#subagents");
const approvalList = document.querySelector("#approval-list");
const approvalEmpty = document.querySelector("#approval-empty");
const pageAlert = document.querySelector("#page-alert");
const passportLogin = document.querySelector("#passport-login");
const passportUser = document.querySelector("#passport-user");
const passportLogout = document.querySelector("#passport-logout");
let passportDevice = null;

async function refreshPassport() {
  if (!passportLogin || !passportUser) return;
  try {
    const user = await api("/api/auth/me");
    passportLogin.hidden = true;
    passportUser.hidden = false;
    passportUser.textContent = user.name || user.username;
    const hero = document.querySelector("#hero-title");
    if (hero && !state.sessionId) {
      hero.textContent = `继续，${user.name || user.username}`;
    }
    if (passportLogout) passportLogout.hidden = false;
  } catch (_) {
    passportLogin.hidden = false;
    passportUser.hidden = true;
    if (passportLogout) passportLogout.hidden = true;
    const payload = await fetch("/api/auth/device").then((response) => response.json());
    passportDevice = payload;
    passportLogin.href = payload.verification_uri || "/auth/start";
    passportLogin.textContent = "PassPort 登录";
    if (payload.user_code) {
      passportLogin.title = `设备码 ${payload.user_code}`;
    }
    pollPassportDevice();
  }
}

if (passportLogout) {
  passportLogout.addEventListener("click", async () => {
    await api("/api/auth/logout", { method: "POST", body: "{}" });
    passportDevice = null;
    await refreshPassport();
  });
}

async function pollPassportDevice() {
  if (!passportDevice) return;
  try {
    const result = await api("/api/auth/device/poll", {
      method: "POST",
      body: "{}",
      silent: true,
    });
    passportDevice = null;
    passportLogin.hidden = true;
    passportUser.hidden = false;
    passportUser.textContent = `已登录 · ${result.name || result.username}`;
    if (passportLogout) passportLogout.hidden = false;
  } catch (error) {
    const message = String(error.message || "");
    if (message.includes("authorization_pending") || message.includes("authorization pending")) {
      setTimeout(pollPassportDevice, Math.max(1000, Number(passportDevice.interval || 5) * 1000));
    } else if (message.includes("slow_down")) {
      passportDevice.interval = Number(passportDevice.interval || 5) + 5;
      setTimeout(pollPassportDevice, passportDevice.interval * 1000);
    }
  }
}

function enhance() {
  if (typeof Blora !== "undefined" && typeof Blora.enhanceButtons === "function") {
    Blora.enhanceButtons(document);
  }
}

function showAlert(message) {
  if (!pageAlert) {
    return;
  }
  pageAlert.hidden = false;
  pageAlert.setAttribute("variant", "danger");
  pageAlert.setAttribute("title", "出错了");
  pageAlert.setAttribute("description", message);
}

function hideAlert() {
  if (pageAlert) {
    pageAlert.hidden = true;
  }
}

function isChecked(el) {
  if (!el) {
    return false;
  }
  if (typeof el.checked === "boolean") {
    return el.checked;
  }
  return el.hasAttribute("checked");
}

function numberValue(el) {
  if (!el) {
    return 0;
  }
  const parsed = Number(el.value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function fieldInput(id) {
  const host = document.querySelector(id);
  if (!host) {
    return null;
  }
  if (host.matches("input, textarea")) {
    return host;
  }
  return host.querySelector("input, textarea");
}

function stored(key, fallback = "") {
  try {
    return localStorage.getItem(key) || fallback;
  } catch (err) {
    return fallback;
  }
}

function store(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch (err) {
    /* ignore quota */
  }
}

async function api(path, options = {}) {
  const silent = Boolean(options.silent);
  const fetchOptions = { ...options };
  delete fetchOptions.silent;
  const loading = document.querySelector("#page-loading");
  if (loading && !silent) {
    loading.hidden = false;
  }
  try {
    const headers = { "Content-Type": "application/json", ...(fetchOptions.headers || {}) };
    const token = stored("blora-token");
    if (token) {
      headers.Authorization = `Bearer ${token}`;
    }
    const response = await fetch(path, {
      ...fetchOptions,
      headers,
    });
    if (!response.ok) {
      throw new Error(await response.text());
    }
    if (response.status === 204) {
      return null;
    }
    return response.json();
  } finally {
    if (loading && !silent) {
      loading.hidden = true;
    }
  }
}

function parseHash() {
  const raw = (location.hash || "#/session").replace(/^#/, "");
  const parts = raw.split("/").filter(Boolean);
  if (parts[0] === "session" && parts[1]) {
    return { view: "session", sessionId: parts[1] };
  }
  if (parts[0] === "tasks") {
    return { view: "tasks" };
  }
  if (parts[0] === "approvals") {
    return { view: "approvals" };
  }
  if (parts[0] === "timeline") {
    return { view: "timeline" };
  }
  if (parts[0] === "artifacts") {
    return { view: "artifacts" };
  }
  if (parts[0] === "workspace") {
    return { view: "workspace" };
  }
  if (parts[0] === "settings") {
    return { view: "settings" };
  }
  return { view: "session", sessionId: parts[1] || state.sessionId };
}

function setView(view) {
  state.view = view;
  for (const id of ["session", "tasks", "approvals", "timeline", "artifacts", "workspace", "settings"]) {
    const el = document.querySelector(`#view-${id}`);
    if (el) {
      el.hidden = id !== view;
    }
  }
  document.querySelectorAll(".ba-rail__link").forEach((link) => {
    const current = (link.getAttribute("href") || "") === `#/${view}`;
    if (current) {
      link.setAttribute("aria-current", "page");
    } else {
      link.removeAttribute("aria-current");
    }
  });
}

function pinnedToBottom(el) {
  if (!el) {
    return true;
  }
  return el.scrollHeight - el.scrollTop - el.clientHeight < el.clientHeight;
}

function setJumpVisible(visible) {
  const jump = document.querySelector("#jump-latest");
  if (jump) {
    jump.hidden = !visible;
  }
}

function followTail(force) {
  if (!thread) {
    return;
  }
  if (force || state.stick) {
    thread.scrollTop = thread.scrollHeight;
    state.stick = true;
    setJumpVisible(false);
    return;
  }
  setJumpVisible(thread.scrollHeight > thread.clientHeight);
}

function setSessionCanvas(empty) {
  const view = document.querySelector("#view-session");
  if (view) {
    view.classList.toggle("ba-session--empty", empty);
  }
  const actions = document.querySelector("#session-actions");
  if (actions) {
    actions.hidden = !state.sessionId;
  }
}

function sessionTitle(session) {
  return (session && session.title) || "未命名会话";
}

function formatTokens(value) {
  const count = Number(value) || 0;
  if (count >= 10000) {
    return `${Math.round(count / 1000)}k`;
  }
  if (count >= 1000) {
    return `${(count / 1000).toFixed(1)}k`;
  }
  return String(count);
}

function renderUsage(session) {
  const usage = document.querySelector("#session-usage");
  if (!usage) {
    return;
  }
  if (!session) {
    usage.textContent = "";
    return;
  }
  usage.textContent = `${formatTokens(session.input_tokens)} 入 / ${formatTokens(session.output_tokens)} 出`;
}

function renderPlan(session) {
  const host = document.querySelector("#session-plan");
  if (!host) {
    return;
  }
  const steps = (session && session.plan) || [];
  host.replaceChildren();
  if (!steps.length) {
    host.hidden = true;
    return;
  }
  host.hidden = false;
  const widget = document.createElement("blora-steps");
  widget.setAttribute("clickable", "false");
  for (const step of steps) {
    const item = document.createElement("blora-step");
    const state = step.status === "done" ? "done" : step.status === "in_progress" ? "active" : "pending";
    item.setAttribute("title", step.title || "");
    item.setAttribute("state", state);
    item.textContent = step.title || "";
    widget.append(item);
  }
  host.append(widget);
}

function bubbleParts(kind) {
  const root = document.createElement("article");
  root.className = "blora-chat";
  if (kind === "user") {
    root.classList.add("blora-chat--end");
  }
  const avatar = document.createElement("span");
  avatar.className = "blora-avatar blora-chat__avatar";
  avatar.dataset.size = "sm";
  const content = document.createElement("div");
  content.className = "blora-chat__content";
  const meta = document.createElement("div");
  meta.className = "blora-chat__meta";
  const author = document.createElement("span");
  const body = document.createElement("div");
  body.className = "blora-chat__bubble ba-bubble";
  meta.append(author);
  content.append(meta, body);
  root.append(avatar, content);
  if (kind === "user") {
    author.textContent = "你";
    avatar.textContent = "You";
    avatar.dataset.variant = "primary";
  } else if (kind === "assistant") {
    author.textContent = "Blora";
    avatar.textContent = "BA";
    avatar.dataset.variant = "info";
  } else if (kind === "routing") {
    author.textContent = "路由";
    avatar.textContent = "⇄";
    avatar.dataset.variant = "neutral";
  } else if (kind === "tools" || kind === "tool") {
    author.textContent = "工具";
    avatar.textContent = "⌘";
    avatar.dataset.variant = "neutral";
  } else {
    author.textContent = kind;
    avatar.textContent = (kind || "?").slice(0, 1).toUpperCase();
    avatar.dataset.variant = "neutral";
  }
  return { root, body };
}

function setBubbleText(body, kind, text) {
  if (kind === "user" || kind === "assistant") {
    let node = body.querySelector("blora-markdown");
    if (!node) {
      body.replaceChildren();
      node = document.createElement("blora-markdown");
      body.append(node);
    }
    node.setAttribute("source", text);
    return node;
  }
  body.replaceChildren(document.createTextNode(text));
  return body;
}

function renderTranscript(items) {
  const stick = state.stick || pinnedToBottom(thread);
  thread.replaceChildren();
  const empty = !items || items.length === 0;
  setSessionCanvas(empty);
  if (empty) {
    thread.append(threadEmpty);
    threadEmpty.hidden = true;
    followTail(true);
    return;
  }
  threadEmpty.hidden = true;
  for (const item of items) {
    const kind = item.kind || "system";
    const bubble = bubbleParts(kind);
    setBubbleText(bubble.body, kind, item.text || "");
    thread.append(bubble.root);
  }
  state.stick = stick;
  followTail(stick);
}

function renderSessions(sessions) {
  const group = document.querySelector("#session-list");
  const nav = document.querySelector("#session-nav");
  if (!group) {
    return;
  }
  group.replaceChildren();
  if (nav) {
    nav.setAttribute("label", "会话");
    if (state.view === "session" && state.sessionId) {
      nav.setAttribute("value", state.sessionId);
    }
  }
  for (const session of sessions || []) {
    const link = document.createElement("blora-sidebar-nav-link");
    link.setAttribute("label", session.title || "未命名会话");
    link.setAttribute("value", session.id);
    link.setAttribute("href", `#/session/${session.id}`);
    if (session.id === state.sessionId && state.view === "session") {
      link.setAttribute("current", "");
    }
    group.append(link);
  }
  sessionEmpty.hidden = Boolean(sessions && sessions.length);
}

async function refreshSessions() {
  const query = state.query ? `?q=${encodeURIComponent(state.query)}` : "";
  const sessions = await api(`/api/sessions${query}`);
  renderSessions(sessions);
  if (!state.sessionId && state.view === "session") {
    title.textContent = "Blora";
    pathEl.textContent = "";
    setSessionCanvas(true);
  }
}

async function selectSession(id) {
  state.sessionId = id;
  state.stick = true;
  hideAlert();
  const session = await api(`/api/sessions/${id}`);
  title.textContent = sessionTitle(session);
  pathEl.textContent = `${session.workspace_path} · ${session.mode} · ${session.status}`;
  renderUsage(session);
  renderPlan(session);
  renderTranscript(session.transcript);
  renderSubagents(session.subagents);
  await refreshApprovals();
  await refreshTasks();
  await refreshSessions();
  if (state.source) {
    state.source.close();
  }
  state.streaming = null;
  state.source = new EventSource(`/api/sessions/${id}/events`);
  state.source.onmessage = (message) => {
    let data = null;
    try {
      data = JSON.parse(message.data);
    } catch (_) {
      data = null;
    }
    handleSessionEvent(id, data).catch((error) => showAlert(error.message));
  };
}

// Apply one live event without refetching the whole session where possible.
// Assistant deltas append to an in-progress bubble; structural events refetch
// only the panel they affect.
async function handleSessionEvent(id, data) {
  if (!data || id !== state.sessionId) {
    return;
  }
  const kind = data.type || "";
  const payload = data.payload || {};
  if (data.status) {
    pathEl.textContent = pathEl.textContent.replace(/ · [a-z_]+ · /, ` · ${data.status} · `);
  }
  if (kind === "assistant.delta") {
    appendStreamingText(payload.text || "");
    return;
  }
  if (kind === "user.input") {
    appendBubble("user", payload.text || "");
    return;
  }
  if (kind === "tool.requested") {
    appendBubble("tool", `${payload.tool || "tool"} …`);
    return;
  }
  if (kind.startsWith("approval.")) {
    await refreshApprovals();
    return;
  }
  if (kind.startsWith("task.")) {
    await refreshTasks();
    return;
  }
  if (
    kind === "assistant.message.completed" ||
    kind === "tool.completed" ||
    kind === "tool.failed" ||
    kind === "routing.changed" ||
    kind.startsWith("run.") ||
    kind.startsWith("subagent.") ||
    kind.startsWith("context.") ||
    kind === "snapshot"
  ) {
    state.streaming = null;
    const latest = await api(`/api/sessions/${id}`);
    pathEl.textContent = `${latest.workspace_path} · ${latest.mode} · ${latest.status}`;
    renderUsage(latest);
    renderPlan(latest);
    renderTranscript(latest.transcript);
    renderSubagents(latest.subagents);
  }
}

function appendBubble(kind, text) {
  setSessionCanvas(false);
  threadEmpty.hidden = true;
  if (threadEmpty.parentElement === thread) {
    threadEmpty.remove();
  }
  const bubble = bubbleParts(kind);
  const node = setBubbleText(bubble.body, kind, text);
  thread.append(bubble.root);
  followTail(false);
  return node;
}

function appendStreamingText(text) {
  if (!text) {
    return;
  }
  if (!state.streaming) {
    state.streaming = appendBubble("assistant", "");
  }
  const current = state.streaming.getAttribute("source") || "";
  state.streaming.setAttribute("source", current + text);
  followTail(false);
}

async function applyRoute() {
  const parsed = parseHash();
  setView(parsed.view);
  if (parsed.sessionId && parsed.sessionId !== state.sessionId) {
    await selectSession(parsed.sessionId);
  }
  if (parsed.view === "tasks") {
    await refreshTaskPage();
  }
  if (parsed.view === "approvals") {
    await refreshApprovalPage();
  }
  if (parsed.view === "timeline") {
    await refreshTimeline();
  }
  if (parsed.view === "artifacts") {
    await refreshArtifacts();
  }
  if (parsed.view === "workspace") {
    await refreshWorkspace();
  }
  if (parsed.view === "settings") {
    await refreshSettings();
  }
  await refreshSessions();
}

document.addEventListener("blora-change", (event) => {
  if (event.target?.id !== "session-nav") {
    return;
  }
  const value = event.detail?.value;
  if (!value) {
    return;
  }
  if (
    value === "tasks" ||
    value === "approvals" ||
    value === "workspace" ||
    value === "settings" ||
    value === "timeline" ||
    value === "artifacts"
  ) {
    location.hash = `#/${value}`;
    return;
  }
  if (value === "session") {
    location.hash = state.sessionId ? `#/session/${state.sessionId}` : "#/session";
    return;
  }
  if (value !== state.sessionId) {
    location.hash = `#/session/${value}`;
  }
});

window.addEventListener("hashchange", () => {
  applyRoute().catch((error) => showAlert(error.message));
});

document.querySelector("#new-session").addEventListener("click", async () => {
  hideAlert();
  const session = await api("/api/sessions", {
    method: "POST",
    body: JSON.stringify({}),
  });
  location.hash = `#/session/${session.id}`;
});

document.querySelector("#composer").addEventListener("submit", async (event) => {
  event.preventDefault();
  hideAlert();
  if (!state.sessionId) {
    const session = await api("/api/sessions", {
      method: "POST",
      body: JSON.stringify({}),
    });
    state.sessionId = session.id;
  }
  const box = fieldInput("#prompt") || prompt;
  const text = (box?.value || "").trim();
  if (!text) {
    return;
  }
  box.value = "";
  box.style.height = "";
  state.stick = true;
  // While a run is in flight the composer becomes a steering channel: the
  // message is queued and delivered at the next model-turn boundary.
  if (state.running) {
    await api(`/api/sessions/${state.sessionId}/steer`, {
      method: "POST",
      body: JSON.stringify({ message: text }),
    });
    return;
  }
  setRunning(true);
  try {
    await api(`/api/sessions/${state.sessionId}/run`, {
      method: "POST",
      body: JSON.stringify({
        prompt: text,
        auto_approve: isChecked(autoApprove),
        provider: stored("blora-provider"),
        model: stored("blora-model"),
        worktree: isChecked(document.querySelector("#pref-worktree")),
      }),
    });
  } finally {
    setRunning(false);
  }
  await selectSession(state.sessionId);
});

let workOrbHandle = null;

function ensureWorkOrb() {
  const canvas = document.querySelector("#work-orb");
  const api = window.BloraThinkingOrb;
  if (!canvas || !api) {
    return null;
  }
  if (!workOrbHandle) {
    workOrbHandle = api.attachThinkingOrb(canvas, { state: "working", size: 20 });
  }
  return { canvas, handle: workOrbHandle };
}

window.addEventListener("blora-orb-ready", () => setWorkOrb(state.running));

function setWorkOrb(running) {
  const orb = ensureWorkOrb();
  if (!orb) {
    return;
  }
  orb.canvas.hidden = !running;
  orb.canvas.setAttribute("aria-hidden", running ? "false" : "true");
  orb.handle.setPaused(!running);
}

function setRunning(running) {
  state.running = running;
  const send = document.querySelector("#send");
  if (send) {
    send.textContent = running ? "插话" : "发送";
  }
  const box = fieldInput("#prompt") || prompt;
  if (box) {
    box.placeholder = running ? "运行中，输入会在下一轮送达" : "要在这个工作区做什么？";
  }
  setWorkOrb(running);
}

function renderSubagents(items) {
  if (!subagents) {
    return;
  }
  if (!items || items.length === 0) {
    subagents.hidden = true;
    subagents.replaceChildren();
    return;
  }
  subagents.hidden = false;
  subagents.replaceChildren();
  for (const item of items) {
    const tag = document.createElement("span");
    tag.className = "blora-tag";
    tag.dataset.variant = item.status === "completed" ? "success" : "primary";
    tag.textContent = `子代理 ${item.role} · ${item.status}`;
    subagents.append(tag);
  }
}

function listOrEmpty(list, empty, count) {
  empty.hidden = count > 0;
  list.hidden = count === 0;
}

function taskItem(task, actions) {
  const item = document.createElement("div");
  item.className = "blora-list__item";
  const meta = document.createElement("div");
  meta.className = "blora-list__meta";
  const name = document.createElement("div");
  name.className = "blora-list__title";
  name.textContent = task.title;
  const desc = document.createElement("div");
  desc.className = "blora-list__desc";
  desc.textContent = `${task.status} · ${task.attempt}${task.cron ? ` · ${task.cron}` : ""}`;
  meta.append(name, desc);
  const badge = document.createElement("span");
  badge.className = "blora-badge";
  badge.dataset.shape = "pill";
  badge.dataset.variant = task.status === "completed" ? "success" : "neutral";
  badge.textContent = task.status;
  item.append(meta, badge);
  if (actions) {
    item.append(actions);
  }
  return item;
}

async function refreshTasks() {
  const query = state.sessionId ? `?session=${state.sessionId}` : "";
  const tasks = await api(`/api/tasks${query}`);
  taskList.replaceChildren();
  listOrEmpty(taskList, taskEmpty, tasks.length);
  for (const task of tasks) {
    taskList.append(taskItem(task));
  }
}

async function refreshTaskPage() {
  const list = document.querySelector("#task-page-list");
  const empty = document.querySelector("#task-page-empty");
  const tasks = await api("/api/tasks");
  list.replaceChildren();
  listOrEmpty(list, empty, tasks.length);
  for (const task of tasks) {
    const actions = document.createElement("div");
    actions.className = "ba-toolbar__actions";
    const pause = document.createElement("button");
    pause.type = "button";
    pause.className = "blora-button";
    pause.dataset.variant = "ghost";
    pause.dataset.size = "sm";
    pause.textContent = "暂停";
    pause.addEventListener("click", async () => {
      await api(`/api/tasks/${task.id}/pause`, { method: "POST", body: "{}" });
      await refreshTaskPage();
    });
    const resume = document.createElement("button");
    resume.type = "button";
    resume.className = "blora-button";
    resume.dataset.variant = "ghost";
    resume.dataset.size = "sm";
    resume.textContent = "恢复";
    resume.addEventListener("click", async () => {
      await api(`/api/tasks/${task.id}/resume`, { method: "POST", body: "{}" });
      await refreshTaskPage();
    });
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.className = "blora-button";
    cancel.dataset.variant = "ghost";
    cancel.dataset.size = "sm";
    cancel.textContent = "取消";
    cancel.addEventListener("click", async () => {
      await api(`/api/tasks/${task.id}/cancel`, { method: "POST", body: "{}" });
      await refreshTaskPage();
    });
    actions.append(pause, resume, cancel);
    list.append(taskItem(task, actions));
  }
  enhance();
}

function approvalItem(row, onDone) {
  const item = document.createElement("div");
  item.className = "blora-list__item";
  const meta = document.createElement("div");
  meta.className = "blora-list__meta";
  const name = document.createElement("div");
  name.className = "blora-list__title";
  name.textContent = row.summary;
  meta.append(name);
  const allow = document.createElement("button");
  allow.type = "button";
  allow.className = "blora-button";
  allow.dataset.variant = "primary";
  allow.dataset.size = "sm";
  allow.textContent = "允许";
  allow.addEventListener("click", async () => {
    await api(`/api/approvals/${row.id}/resolve`, {
      method: "POST",
      body: JSON.stringify({ allow: true }),
    });
    await onDone();
  });
  const deny = document.createElement("button");
  deny.type = "button";
  deny.className = "blora-button";
  deny.dataset.variant = "ghost";
  deny.dataset.size = "sm";
  deny.textContent = "拒绝";
  deny.addEventListener("click", async () => {
    await api(`/api/approvals/${row.id}/resolve`, {
      method: "POST",
      body: JSON.stringify({ allow: false }),
    });
    await onDone();
  });
  item.append(meta, allow, deny);
  return item;
}

async function refreshApprovals() {
  const strip = document.querySelector("#session-approvals");
  if (!state.sessionId) {
    approvalList.replaceChildren();
    listOrEmpty(approvalList, approvalEmpty, 0);
    if (strip) {
      strip.replaceChildren();
      strip.hidden = true;
    }
    return;
  }
  const rows = await api(`/api/approvals?session=${state.sessionId}`);
  approvalList.replaceChildren();
  listOrEmpty(approvalList, approvalEmpty, rows.length);
  if (strip) {
    strip.replaceChildren();
    strip.hidden = rows.length === 0;
  }
  for (const row of rows) {
    const item = approvalItem(row, refreshApprovals);
    approvalList.append(item);
    if (strip) {
      strip.append(approvalItem(row, refreshApprovals));
    }
  }
  enhance();
}

async function refreshApprovalPage() {
  const list = document.querySelector("#approval-page-list");
  const empty = document.querySelector("#approval-page-empty");
  const rows = await api("/api/approvals");
  list.replaceChildren();
  listOrEmpty(list, empty, rows.length);
  for (const row of rows) {
    list.append(approvalItem(row, refreshApprovalPage));
  }
  enhance();
}

async function refreshTimeline() {
  const list = document.querySelector("#timeline-list");
  const empty = document.querySelector("#timeline-empty");
  list.replaceChildren();
  if (!state.sessionId) {
    listOrEmpty(list, empty, 0);
    return;
  }
  const events = await api(`/api/sessions/${state.sessionId}/export`);
  const rows = Array.isArray(events) ? events : [];
  listOrEmpty(list, empty, rows.length);
  for (const event of rows) {
    const item = document.createElement("div");
    item.className = "blora-list__item";
    const meta = document.createElement("div");
    meta.className = "blora-list__meta";
    const name = document.createElement("div");
    name.className = "blora-list__title";
    name.textContent = event.type || "event";
    const desc = document.createElement("div");
    desc.className = "blora-list__desc";
    desc.textContent = `${event.sequence || ""} · ${event.timestamp || ""}`;
    meta.append(name, desc);
    item.append(meta);
    list.append(item);
  }
}

async function refreshArtifacts() {
  const list = document.querySelector("#artifact-list");
  const empty = document.querySelector("#artifact-empty");
  const query = state.sessionId ? `?session=${state.sessionId}` : "";
  const payload = await api(`/api/artifacts${query}`);
  const rows = payload.artifacts || [];
  list.replaceChildren();
  listOrEmpty(list, empty, rows.length);
  for (const row of rows) {
    const item = document.createElement("div");
    item.className = "blora-list__item";
    const meta = document.createElement("div");
    meta.className = "blora-list__meta";
    const name = document.createElement("div");
    name.className = "blora-list__title";
    name.textContent = `${row.kind} · ${row.id}`;
    const desc = document.createElement("div");
    desc.className = "blora-list__desc";
    desc.textContent = row.content || row.path || "";
    meta.append(name, desc);
    item.append(meta);
    list.append(item);
  }
}

async function refreshWorkspace() {
  const info = await api("/api/workspace");
  document.querySelector("#workspace-path").textContent = info.path;
  document.querySelector("#workspace-git").textContent =
    [info.git_branch, info.git_status, info.git_log, info.git_diff].join("\n\n");
  const list = document.querySelector("#workspace-file-list");
  list.replaceChildren();
  for (const name of info.entries || []) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "blora-button";
    item.dataset.variant = "ghost";
    item.dataset.size = "sm";
    item.textContent = name;
    if (!name.endsWith("/")) {
      item.addEventListener("click", async () => {
        const file = await api(`/api/workspace/file?path=${encodeURIComponent(name)}`);
        document.querySelector("#workspace-file").textContent = file.contents || "";
      });
    }
    list.append(item);
  }
  enhance();
}

async function refreshSettings() {
  const info = await api("/api/settings");
  const provider = fieldInput("#pref-provider") || document.querySelector("#pref-provider");
  const model = fieldInput("#pref-model") || document.querySelector("#pref-model");
  if (provider && !provider.value) {
    provider.value = stored("blora-provider", info.provider);
  }
  if (model && !model.value) {
    model.value = stored("blora-model", info.model);
  }
  document.querySelector("#settings-meta").textContent =
    `供应商 ${info.provider_display || info.provider} · 模型 ${info.model} · exec=${info.exec} gateway=${info.gateway} key=${info.has_api_key} mcp=${info.mcp} plugins=${(info.plugins || []).join(",") || "-"}`;
  const tokenInput = fieldInput("#pref-token") || document.querySelector("#pref-token");
  if (tokenInput && !tokenInput.value) {
    tokenInput.value = stored("blora-token");
  }
  const market = await api("/api/marketplace");
  const list = document.querySelector("#market-list");
  if (list) {
    list.replaceChildren();
    for (const plugin of market.plugins || []) {
      const item = document.createElement("div");
      item.className = "blora-list__item";
      const meta = document.createElement("div");
      meta.className = "blora-list__meta";
      const name = document.createElement("div");
      name.className = "blora-list__title";
      name.textContent = plugin.name;
      const desc = document.createElement("div");
      desc.className = "blora-list__desc";
      desc.textContent = plugin.description || "";
      meta.append(name, desc);
      const install = document.createElement("button");
      install.type = "button";
      install.className = "blora-button";
      install.dataset.variant = "secondary";
      install.dataset.size = "sm";
      install.textContent = "安装";
      install.addEventListener("click", async () => {
        await api("/api/marketplace", {
          method: "POST",
          body: JSON.stringify({ name: plugin.name }),
        });
        await refreshSettings();
      });
      item.append(meta, install);
      list.append(item);
    }
    enhance();
  }
}

document.querySelector("#compact").addEventListener("click", async () => {
  if (!state.sessionId) {
    return;
  }
  hideAlert();
  await api(`/api/sessions/${state.sessionId}/compact`, { method: "POST", body: "{}" });
  await selectSession(state.sessionId);
});

document.querySelector("#cancel-run").addEventListener("click", async () => {
  if (!state.sessionId) {
    return;
  }
  hideAlert();
  try {
    await api(`/api/sessions/${state.sessionId}/cancel`, { method: "POST", body: "{}" });
  } catch (error) {
    showAlert(error.message);
  }
});

document.querySelector("#fork").addEventListener("click", async () => {
  if (!state.sessionId) {
    return;
  }
  hideAlert();
  const child = await api(`/api/sessions/${state.sessionId}/fork`, { method: "POST", body: "{}" });
  location.hash = `#/session/${child.id}`;
});

document.querySelector("#task-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!state.sessionId) {
    return;
  }
  const titleInput = fieldInput("#task-title") || document.querySelector("#task-title");
  const delayInput = document.querySelector("#task-delay");
  const cronInput = fieldInput("#task-cron") || document.querySelector("#task-cron");
  await api("/api/tasks", {
    method: "POST",
    body: JSON.stringify({
      session_id: state.sessionId,
      title: (titleInput?.value || "").trim(),
      prompt: (titleInput?.value || "").trim(),
      delay_seconds: numberValue(delayInput),
      cron: (cronInput?.value || "").trim() || null,
      auto_approve: isChecked(autoApprove),
    }),
  });
  if (titleInput) {
    titleInput.value = "";
  }
  await refreshTasks();
});

const prefProvider = fieldInput("#pref-provider") || document.querySelector("#pref-provider");
const prefModel = fieldInput("#pref-model") || document.querySelector("#pref-model");
if (prefProvider) {
  prefProvider.addEventListener("change", () => store("blora-provider", prefProvider.value));
}
if (prefModel) {
  prefModel.addEventListener("change", () => store("blora-model", prefModel.value));
}
const prefToken = fieldInput("#pref-token") || document.querySelector("#pref-token");
if (prefToken) {
  prefToken.addEventListener("change", () => store("blora-token", prefToken.value));
}
const searchBox = fieldInput("#session-search") || document.querySelector("#session-search");
let searchTimer = 0;
if (searchBox) {
  searchBox.addEventListener("input", () => {
    window.clearTimeout(searchTimer);
    searchTimer = window.setTimeout(async () => {
      state.query = (searchBox.value || "").trim();
      try {
        await refreshSessions();
      } catch (error) {
        showAlert(error.message);
      }
    }, 200);
  });
}

const promptBox = fieldInput("#prompt") || prompt;
if (promptBox) {
  const growPrompt = () => {
    promptBox.style.height = "auto";
    const limit = 12 * 16;
    promptBox.style.height = `${Math.min(promptBox.scrollHeight, limit)}px`;
  };
  promptBox.addEventListener("input", growPrompt);
  promptBox.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      document.querySelector("#composer").requestSubmit();
    }
  });
}

if (thread) {
  thread.addEventListener("scroll", () => {
    state.stick = pinnedToBottom(thread);
    setJumpVisible(!state.stick && thread.scrollHeight > thread.clientHeight + 8);
  });
}

const jumpLatest = document.querySelector("#jump-latest");
if (jumpLatest) {
  jumpLatest.addEventListener("click", () => {
    state.stick = true;
    followTail(true);
  });
}

document.querySelectorAll(".ba-chip").forEach((chip) => {
  chip.addEventListener("click", () => {
    const box = fieldInput("#prompt") || prompt;
    if (!box) {
      return;
    }
    box.value = chip.getAttribute("data-prompt") || "";
    box.focus();
  });
});

enhance();
refreshPassport();
applyRoute().catch((error) => {
  showAlert(error.message);
});
