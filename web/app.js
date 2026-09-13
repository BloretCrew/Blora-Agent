const state = {
  sessionId: null,
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
  const value = el.value;
  const parsed = Number(value);
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
  nav.setAttribute("label", "会话");
  if (state.sessionId) {
    nav.setAttribute("value", state.sessionId);
  }
  const group = document.createElement("blora-sidebar-nav-group");
  group.setAttribute("label", "会话");
  for (const session of sessions || []) {
    const link = document.createElement("blora-sidebar-nav-link");
    link.setAttribute("label", session.title || session.id);
    link.setAttribute("value", session.id);
    link.setAttribute("href", `#session/${session.id}`);
    if (session.id === state.sessionId) {
      link.setAttribute("current", "");
    }
    group.append(link);
  }
  nav.append(group);
  previous.replaceWith(nav);
  sessionEmpty.hidden = Boolean(sessions && sessions.length);
}

async function refreshSessions() {
  const sessions = await api("/api/sessions");
  renderSessions(sessions);
  if (!state.sessionId && sessions[0]) {
    await selectSession(sessions[0].id);
  }
}

async function selectSession(id) {
  state.sessionId = id;
  hideAlert();
  const session = await api(`/api/sessions/${id}`);
  title.textContent = session.title || session.id;
  pathEl.textContent = session.workspace_path;
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
  };
}

document.addEventListener("blora-change", (event) => {
  if (event.target?.id !== "session-nav") {
    return;
  }
  const value = event.detail?.value;
  if (value && value !== state.sessionId) {
    selectSession(value);
  }
});

document.querySelector("#new-session").addEventListener("click", async () => {
  hideAlert();
  const session = await api("/api/sessions", {
    method: "POST",
    body: JSON.stringify({ title: "web" }),
  });
  await refreshSessions();
  await selectSession(session.id);
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

async function refreshTasks() {
  const query = state.sessionId ? `?session=${state.sessionId}` : "";
  const tasks = await api(`/api/tasks${query}`);
  taskList.replaceChildren();
  listOrEmpty(taskList, taskEmpty, tasks.length);
  for (const task of tasks) {
    const item = document.createElement("div");
    item.className = "blora-list__item";
    const meta = document.createElement("div");
    meta.className = "blora-list__meta";
    const name = document.createElement("div");
    name.className = "blora-list__title";
    name.textContent = task.title;
    const desc = document.createElement("div");
    desc.className = "blora-list__desc";
    desc.textContent = `${task.status} · ${task.attempt}`;
    meta.append(name, desc);
    const badge = document.createElement("span");
    badge.className = "blora-badge";
    badge.dataset.shape = "pill";
    badge.dataset.variant = task.status === "completed" ? "success" : "neutral";
    badge.textContent = task.status;
    item.append(meta, badge);
    taskList.append(item);
  }
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
      await refreshApprovals();
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
      await refreshApprovals();
    });
    item.append(meta, allow, deny);
    approvalList.append(item);
  }
  enhance();
}

document.querySelector("#compact").addEventListener("click", async () => {
  if (!state.sessionId) {
    return;
  }
  hideAlert();
  await api(`/api/sessions/${state.sessionId}/compact`, { method: "POST", body: "{}" });
  await selectSession(state.sessionId);
});

document.querySelector("#task-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!state.sessionId) {
    return;
  }
  const titleInput = fieldInput("#task-title") || document.querySelector("#task-title");
  const delayInput = document.querySelector("#task-delay");
  await api("/api/tasks", {
    method: "POST",
    body: JSON.stringify({
      session_id: state.sessionId,
      title: (titleInput?.value || "").trim(),
      prompt: (titleInput?.value || "").trim(),
      delay_seconds: numberValue(delayInput),
      auto_approve: isChecked(autoApprove),
    }),
  });
  if (titleInput) {
    titleInput.value = "";
  }
  await refreshTasks();
});

enhance();
refreshSessions().catch((error) => {
  showAlert(error.message);
});
