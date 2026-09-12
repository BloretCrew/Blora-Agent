const state = {
  sessionId: null,
  source: null,
};

const sessionList = document.querySelector("#session-list");
const thread = document.querySelector("#thread");
const title = document.querySelector("#session-title");
const pathEl = document.querySelector("#session-path");
const prompt = document.querySelector("#prompt");
const autoApprove = document.querySelector("#auto-approve");
const taskList = document.querySelector("#task-list");
const subagents = document.querySelector("#subagents");
const approvalList = document.querySelector("#approval-list");

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
  for (const item of items || []) {
    const article = document.createElement("article");
    article.className = "ba-bubble";
    article.dataset.kind = item.kind;
    article.textContent = item.text;
    thread.append(article);
  }
  thread.scrollTop = thread.scrollHeight;
}

function renderSessions(sessions) {
  sessionList.replaceChildren();
  for (const session of sessions) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = session.title || session.id;
    if (session.id === state.sessionId) {
      button.setAttribute("aria-current", "true");
    }
    button.addEventListener("click", () => selectSession(session.id));
    sessionList.append(button);
  }
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

document.querySelector("#new-session").addEventListener("click", async () => {
  const session = await api("/api/sessions", {
    method: "POST",
    body: JSON.stringify({ title: "web" }),
  });
  await refreshSessions();
  await selectSession(session.id);
});

document.querySelector("#composer").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!state.sessionId) {
    const session = await api("/api/sessions", {
      method: "POST",
      body: JSON.stringify({ title: "web" }),
    });
    state.sessionId = session.id;
  }
  const text = prompt.value.trim();
  if (!text) {
    return;
  }
  prompt.value = "";
  await api(`/api/sessions/${state.sessionId}/run`, {
    method: "POST",
    body: JSON.stringify({
      prompt: text,
      auto_approve: autoApprove.checked,
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
    const row = document.createElement("p");
    row.textContent = `子代理 ${item.role} · ${item.status}`;
    subagents.append(row);
  }
}

async function refreshTasks() {
  const query = state.sessionId ? `?session=${state.sessionId}` : "";
  const tasks = await api(`/api/tasks${query}`);
  taskList.replaceChildren();
  for (const task of tasks) {
    const row = document.createElement("p");
    row.textContent = `${task.title} · ${task.status}`;
    taskList.append(row);
  }
}

async function refreshApprovals() {
  if (!state.sessionId) {
    approvalList.replaceChildren();
    return;
  }
  const rows = await api(`/api/approvals?session=${state.sessionId}`);
  approvalList.replaceChildren();
  for (const row of rows) {
    const wrap = document.createElement("div");
    const text = document.createElement("p");
    text.textContent = row.summary;
    const allow = document.createElement("button");
    allow.type = "button";
    allow.className = "blora-button";
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
    deny.textContent = "拒绝";
    deny.addEventListener("click", async () => {
      await api(`/api/approvals/${row.id}/resolve`, {
        method: "POST",
        body: JSON.stringify({ allow: false }),
      });
      await refreshApprovals();
    });
    wrap.append(text, allow, deny);
    approvalList.append(wrap);
  }
}

document.querySelector("#compact").addEventListener("click", async () => {
  if (!state.sessionId) {
    return;
  }
  await api(`/api/sessions/${state.sessionId}/compact`, { method: "POST", body: "{}" });
  await selectSession(state.sessionId);
});

document.querySelector("#task-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!state.sessionId) {
    return;
  }
  const titleInput = document.querySelector("#task-title");
  const delayInput = document.querySelector("#task-delay");
  await api("/api/tasks", {
    method: "POST",
    body: JSON.stringify({
      session_id: state.sessionId,
      title: titleInput.value.trim(),
      prompt: titleInput.value.trim(),
      delay_seconds: Number(delayInput.value || 0),
      auto_approve: autoApprove.checked,
    }),
  });
  titleInput.value = "";
  await refreshTasks();
});

refreshSessions().catch((error) => {
  thread.textContent = error.message;
});
