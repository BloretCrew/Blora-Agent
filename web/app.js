const state = {
  sessionId: null,
  view: "session",
  source: null,
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
  const response = await fetch(path, {
    headers: { "Content-Type": "application/json", ...(options.headers || {}) },
    ...options,
  });
  if (!response.ok) {
    throw new Error(await response.text());
  }
  if (response.status === 204) {
    return null;
  }
  return response.json();
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
  for (const id of ["session", "tasks", "approvals", "workspace", "settings"]) {
    const el = document.querySelector(`#view-${id}`);
    if (el) {
      el.hidden = id !== view;
    }
  }
}

function renderTranscript(items) {
  thread.replaceChildren();
  if (!items || items.length === 0) {
    thread.append(threadEmpty);
    threadEmpty.hidden = false;
    return;
  }
  threadEmpty.hidden = true;
  for (const item of items) {
    const bubble = document.createElement("blora-chat");
    if (item.kind === "user") {
      bubble.setAttribute("author", "你");
      bubble.setAttribute("avatar", "You");
      bubble.setAttribute("side", "end");
      bubble.setAttribute("avatar-variant", "primary");
    } else if (item.kind === "assistant") {
      bubble.setAttribute("author", "Blora");
      bubble.setAttribute("avatar", "BA");
    } else {
      bubble.setAttribute("author", item.kind);
      bubble.setAttribute("avatar", item.kind.slice(0, 1).toUpperCase());
    }
    bubble.setAttribute("message", item.text);
    thread.append(bubble);
  }
  thread.scrollTop = thread.scrollHeight;
}

function renderSessions(sessions) {
  const previous = document.querySelector("#session-nav");
  const nav = document.createElement("blora-sidebar-nav");
  nav.id = "session-nav";
  nav.setAttribute("label", "导航");
  if (state.view === "session" && state.sessionId) {
    nav.setAttribute("value", state.sessionId);
  } else {
    nav.setAttribute("value", state.view);
  }
  const desk = document.createElement("blora-sidebar-nav-group");
  desk.setAttribute("label", "工作台");
  const pages = [
    ["会话", "session", "#/session"],
    ["任务", "tasks", "#/tasks"],
    ["审批", "approvals", "#/approvals"],
    ["工作区", "workspace", "#/workspace"],
    ["设置", "settings", "#/settings"],
  ];
  for (const [label, value, href] of pages) {
    const link = document.createElement("blora-sidebar-nav-link");
    link.setAttribute("label", label);
    link.setAttribute("value", value);
    link.setAttribute("href", href);
    if (state.view === value) {
      link.setAttribute("current", "");
    }
    desk.append(link);
  }
  const group = document.createElement("blora-sidebar-nav-group");
  group.setAttribute("label", "会话");
  for (const session of sessions || []) {
    const link = document.createElement("blora-sidebar-nav-link");
    link.setAttribute("label", session.title || session.id);
    link.setAttribute("value", session.id);
    link.setAttribute("href", `#/session/${session.id}`);
    if (session.id === state.sessionId && state.view === "session") {
      link.setAttribute("current", "");
    }
    group.append(link);
  }
  nav.append(desk, group);
  previous.replaceWith(nav);
  sessionEmpty.hidden = Boolean(sessions && sessions.length);
}

async function refreshSessions() {
  const sessions = await api("/api/sessions");
  renderSessions(sessions);
  if (!state.sessionId && sessions[0] && state.view === "session") {
    await selectSession(sessions[0].id);
  }
}

async function selectSession(id) {
  state.sessionId = id;
  hideAlert();
  const session = await api(`/api/sessions/${id}`);
  title.textContent = session.title || session.id;
  pathEl.textContent = `${session.workspace_path} · ${session.status} · ${session.input_tokens}/${session.output_tokens} tokens`;
  renderTranscript(session.transcript);
  renderSubagents(session.subagents);
  await refreshApprovals();
  await refreshTasks();
  await refreshSessions();
  if (state.source) {
    state.source.close();
  }
  state.source = new EventSource(`/api/sessions/${id}/events`);
  state.source.onmessage = async () => {
    const latest = await api(`/api/sessions/${id}`);
    renderTranscript(latest.transcript);
    renderSubagents(latest.subagents);
    await refreshApprovals();
    await refreshTasks();
  };
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
  if (value === "tasks" || value === "approvals" || value === "workspace" || value === "settings") {
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
    body: JSON.stringify({ title: "web" }),
  });
  location.hash = `#/session/${session.id}`;
});

document.querySelector("#composer").addEventListener("submit", async (event) => {
  event.preventDefault();
  hideAlert();
  if (!state.sessionId) {
    const session = await api("/api/sessions", {
      method: "POST",
      body: JSON.stringify({ title: "web" }),
    });
    state.sessionId = session.id;
  }
  const box = fieldInput("#prompt") || prompt;
  const text = (box?.value || "").trim();
  if (!text) {
    return;
  }
  box.value = "";
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
  await selectSession(state.sessionId);
});

function renderSubagents(items) {
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
  if (!state.sessionId) {
    approvalList.replaceChildren();
    listOrEmpty(approvalList, approvalEmpty, 0);
    return;
  }
  const rows = await api(`/api/approvals?session=${state.sessionId}`);
  approvalList.replaceChildren();
  listOrEmpty(approvalList, approvalEmpty, rows.length);
  for (const row of rows) {
    approvalList.append(approvalItem(row, refreshApprovals));
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

async function refreshWorkspace() {
  const info = await api("/api/workspace");
  document.querySelector("#workspace-path").textContent = info.path;
  document.querySelector("#workspace-git").textContent =
    [info.git_branch, info.git_status, info.git_log, info.git_diff].join("\n\n");
  document.querySelector("#workspace-files").textContent = info.files;
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
    `环境 provider=${info.provider} model=${info.model} key=${info.has_api_key} mcp=${info.mcp}`;
}

document.querySelector("#compact").addEventListener("click", async () => {
  if (!state.sessionId) {
    return;
  }
  hideAlert();
  await api(`/api/sessions/${state.sessionId}/compact`, { method: "POST", body: "{}" });
  await selectSession(state.sessionId);
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

enhance();
applyRoute().catch((error) => {
  showAlert(error.message);
});
