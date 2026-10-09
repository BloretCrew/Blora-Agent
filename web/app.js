const state = {
  sessionId: null,
  view: "session",
  source: null,
  query: "",
  running: false,
  streaming: null,
  stick: true,
  slashIndex: 0,
  session: null,
  history: [],
  historyIndex: -1,
  draft: "",
  transcriptQuery: "",
  selectionVersion: 0,
};

const SLASH = [
  ["help", "列出命令"],
  ["new", "新建会话"],
  ["compact", "压缩当前会话"],
  ["fork", "派生当前会话"],
  ["cancel", "停止当前运行"],
  ["archive", "归档当前会话"],
  ["resume", "恢复当前会话"],
  ["export", "导出事件日志"],
  ["yes", "权限设为全部允许"],
  ["no", "权限设为询问"],
  ["plan", "权限设为计划"],
  ["model", "设置模型，例如 /model gpt-4o-mini"],
  ["provider", "设置供应商，例如 /provider openai"],
  ["tasks", "打开任务"],
  ["approvals", "打开审批"],
  ["settings", "打开设置"],
  ["login", "连接 PassPort 账号"],
  ["onboarding", "重新打开首次使用引导"],
  ["timeline", "打开时间线"],
  ["artifacts", "打开产物"],
  ["git", "打开工作区 Git"],
  ["usage", "显示用量"],
  ["mode", "新建 code、work、agent 或 imagine 会话"],
  ["imagine", "新建 Imagine 生图会话"],
  ["install", "安装市场插件"],
  ["read", "读取工作区文件"],
  ["steer", "向运行中的回合插话"],
  ["theme", "打开主题设置"],
];

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
function enhance() {
  if (typeof Blora !== "undefined" && typeof Blora.enhanceButtons === "function") {
    Blora.enhanceButtons(document);
  }
}

function showAlert(message) {
  showNotice(message, "danger", "出错了");
}

function showNotice(message, variant, titleText) {
  if (!pageAlert) {
    return;
  }
  pageAlert.hidden = false;
  pageAlert.setAttribute("variant", variant);
  pageAlert.setAttribute("title", titleText);
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
      const error = new Error(await response.text());
      error.status = response.status;
      throw error;
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
  const compactions = Number(session.compaction_count || 0);
  usage.textContent = `${formatTokens(session.input_tokens)} 入 / ${formatTokens(session.output_tokens)} 出`;
  usage.title = "";
  delete usage.dataset.warn;
  if (compactions > 0) {
    usage.textContent += ` · 已压缩 ${compactions} 次`;
    usage.title = "较早的对话已被摘要；压缩次数越多，细节越可能丢失";
    if (compactions >= 3) {
      usage.dataset.warn = "true";
      usage.title += "。建议为无关的新任务开启新会话";
    }
  }
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
  root.dataset.kind = kind;
  root.hidden = (kind === "tool" || kind === "tools") && stored("blora-hide-tools") === "on";
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
  const hideRoles = stored("blora-message-roles", "on") === "off";
  avatar.hidden = hideRoles;
  meta.hidden = hideRoles;
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
  } else if (kind === "reasoning") {
    author.textContent = "思考过程";
    avatar.textContent = "…";
    avatar.dataset.variant = "neutral";
  } else if (kind.startsWith("question")) {
    author.textContent = "Blora 提问";
    avatar.textContent = "?";
    avatar.dataset.variant = "warning";
  } else {
    author.textContent = kind;
    avatar.textContent = (kind || "?").slice(0, 1).toUpperCase();
    avatar.dataset.variant = "neutral";
  }
  return { root, body };
}

function setBubbleText(body, kind, text) {
  if (kind === "reasoning") {
    // Folded by default; the model's reasoning is for inspection only.
    const details = document.createElement("details");
    details.className = "ba-reasoning";
    details.open = stored("blora-show-reasoning") === "on";
    const summary = document.createElement("summary");
    summary.textContent = `思考过程 · ${text.replace(/\s/g, "").length} 字`;
    summary.title = "双击查看完整思考过程";
    summary.addEventListener("dblclick", (event) => {
      event.preventDefault();
      event.stopPropagation();
      const openDetails = () => window.BloraWorkbench?.open("reasoning-detail", { text });
      if (window.BloraWorkbench) openDetails();
      else window.setTimeout(openDetails, 0);
    });
    const pre = document.createElement("div");
    pre.className = "ba-reasoning__body";
    pre.textContent = text;
    details.append(summary, pre);
    body.replaceChildren(details);
    return details;
  }
  if (kind.startsWith("question")) {
    const [question, answer] = String(text).split("\n→ ");
    const ask = document.createElement("p");
    ask.className = "ba-question__text";
    ask.textContent = question;
    body.replaceChildren(ask);
    if (answer !== undefined) {
      const reply = document.createElement("p");
      reply.className = "ba-question__answer";
      reply.textContent = `→ ${answer}`;
      body.append(reply);
    } else if (kind === "question.dismissed") {
      const reply = document.createElement("p");
      reply.className = "ba-question__answer";
      reply.textContent = "已跳过";
      body.append(reply);
    }
    return body;
  }
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
  if (kind === "tool" || kind === "tools") {
    body.title = "双击查看工具详情";
    body.ondblclick = () => window.BloraWorkbench?.open("tool-detail", { text, sessionId: state.sessionId });
  }
  appendGeneratedImages(body, text);
  return body;
}

function appendGeneratedImages(body, text) {
  const pattern = /\.blora\/images\/([A-Za-z0-9_-]+)\/([a-fA-F0-9]{64}\.(?:png|jpe?g|webp))/g;
  const gallery = document.createElement("div");
  gallery.className = "ba-generated";
  let match = pattern.exec(text);
  while (match) {
    if (state.sessionId && match[1] === state.sessionId) {
      const image = document.createElement("img");
      image.alt = "生成的图片";
      image.src = `/api/sessions/${encodeURIComponent(state.sessionId)}/images/${encodeURIComponent(match[2])}`;
      gallery.append(image);
    }
    match = pattern.exec(text);
  }
  if (gallery.childElementCount) {
    body.append(gallery);
  }
}

function renderTranscript(items) {
  if (currentRunMode() === "imagine") {
    renderGallery(items);
    return;
  }
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
    if (state.transcriptQuery && !String(item.text || "").toLowerCase().includes(state.transcriptQuery.toLowerCase())) continue;
    const kind = item.kind || "system";
    const bubble = bubbleParts(kind);
    setBubbleText(bubble.body, kind, item.text || "");
    thread.append(bubble.root);
  }
  state.stick = stick;
  followTail(stick);
}

// Card for an open ask_user question: option buttons, a free-form answer,
// and skip. Answers go to POST /api/sessions/:id/input.
function renderQuestion(session) {
  const host = document.querySelector("#session-question");
  if (!host) return;
  const pending = session && session.pending_input;
  if (!pending) {
    host.hidden = true;
    host.replaceChildren();
    delete host.dataset.request;
    return;
  }
  if (host.dataset.request === pending.request_id && !host.hidden) return;
  host.dataset.request = pending.request_id;
  host.hidden = false;
  host.replaceChildren();
  const sessionId = state.sessionId;
  const heading = document.createElement("p");
  heading.className = "ba-question__label";
  heading.textContent = "Blora 需要你的决定";
  const text = document.createElement("p");
  text.className = "ba-question__text";
  text.textContent = pending.question;
  const options = document.createElement("div");
  options.className = "ba-question__options";
  const form = document.createElement("form");
  form.className = "ba-question__form";
  const input = document.createElement("input");
  input.className = "blora-input";
  input.placeholder = "输入回答…";
  input.setAttribute("aria-label", "回答");
  input.maxLength = 8000;
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "blora-button";
  submit.dataset.variant = "primary";
  submit.dataset.size = "sm";
  submit.textContent = "回答";
  const skip = document.createElement("button");
  skip.type = "button";
  skip.className = "blora-button";
  skip.dataset.variant = "ghost";
  skip.dataset.size = "sm";
  skip.textContent = "跳过";
  const controls = [submit, skip, input];
  const send = async (answer) => {
    controls.forEach((control) => { control.disabled = true; });
    options.querySelectorAll("button").forEach((button) => { button.disabled = true; });
    try {
      await api(`/api/sessions/${encodeURIComponent(sessionId)}/input`, {
        method: "POST",
        body: JSON.stringify({ request_id: pending.request_id, answer }),
      });
      if (state.sessionId === sessionId) {
        host.hidden = true;
        host.replaceChildren();
        delete host.dataset.request;
      }
    } catch (error) {
      controls.forEach((control) => { control.disabled = false; });
      options.querySelectorAll("button").forEach((button) => { button.disabled = false; });
      showAlert(error.message);
    }
  };
  (pending.options || []).forEach((option, index) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "blora-button";
    button.dataset.variant = index === 0 ? "primary" : "ghost";
    button.dataset.size = "sm";
    button.textContent = option;
    button.addEventListener("click", () => send(option));
    options.append(button);
  });
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const answer = input.value.trim();
    if (answer) send(answer);
  });
  skip.addEventListener("click", () => send(null));
  form.append(input, submit, skip);
  host.append(heading, text);
  if (options.childElementCount) host.append(options);
  host.append(form);
  if (!(pending.options || []).length) input.focus();
}

function renderGallery(items) {
  const board = imagineBoard(items);
  const empty = board.cards.length === 0 && board.errors.length === 0;
  setSessionCanvas(empty);
  thread.replaceChildren();
  if (empty) {
    followTail(true);
    return;
  }
  const gallery = document.createElement("div");
  gallery.className = "ba-gallery";
  for (const message of board.errors) {
    const alert = document.createElement("blora-alert");
    alert.setAttribute("variant", "danger");
    alert.textContent = message;
    gallery.append(alert);
  }
  for (const card of board.cards) {
    const figure = document.createElement("figure");
    figure.className = "ba-card";
    if (state.sessionId && card.session === state.sessionId) {
      const image = document.createElement("img");
      image.alt = card.prompt || "生成的图片";
      image.src = `/api/sessions/${encodeURIComponent(state.sessionId)}/images/${encodeURIComponent(card.name)}`;
      figure.append(image);
    }
    const caption = document.createElement("figcaption");
    caption.textContent = card.prompt || card.path;
    figure.append(caption);
    gallery.append(figure);
  }
  thread.append(gallery);
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
  const previousId = state.sessionId;
  const version = ++state.selectionVersion;
  if (state.source) { state.source.close(); state.source = null; }
  state.sessionId = id;
  state.stick = true;
  hideAlert();
  const session = await api(`/api/sessions/${id}`);
  if (version !== state.selectionVersion || state.sessionId !== id) return;
  state.session = session;
  setRunning(["running", "waiting_approval", "waiting_input"].includes(session.run_status));
  window.dispatchEvent(new CustomEvent("blora-session", { detail: session }));
  title.textContent = sessionTitle(session);
  pathEl.textContent = `${session.workspace_path} · ${session.mode} · ${session.status}`;
  if (previousId !== id && state.permission === "yolo") applyPermission("ask", false);
  if (session.permission_mode) {
    applyPermission(["bypassPermissions", "yolo"].includes(session.permission_mode)
      ? previousId === id && state.permission === "yolo" ? "yolo" : "ask"
      : session.permission_mode, false);
  }
  if (session.mode) {
    applyRunMode(session.mode, false);
  }
  renderUsage(session);
  renderPlan(session);
  renderQuestion(session);
  renderTranscript(session.transcript);
  renderSubagents(session.subagents);
  await refreshApprovals();
  await refreshTasks();
  await refreshSessions();
  if (version !== state.selectionVersion || state.sessionId !== id) return;
  if (state.source) state.source.close();
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
    setRunning(["running", "waiting_approval", "waiting_input"].includes(data.status));
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
    kind === "assistant.reasoning" ||
    kind.startsWith("input.") ||
    kind === "skill.changed" ||
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
    if (id !== state.sessionId || (state.session && latest.last_sequence < state.session.last_sequence)) return;
    state.session = latest;
    setRunning(["running", "waiting_approval", "waiting_input"].includes(latest.run_status));
    window.dispatchEvent(new CustomEvent("blora-session", { detail: latest }));
    pathEl.textContent = `${latest.workspace_path} · ${latest.mode} · ${latest.status}`;
    renderUsage(latest);
    renderPlan(latest);
    renderQuestion(latest);
    renderTranscript(latest.transcript);
    renderSubagents(latest.subagents);
  }
}

function appendBubble(kind, text) {
  if (currentRunMode() === "imagine") {
    return;
  }
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
  if (!text || currentRunMode() === "imagine") {
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
  if (location.hash === "#/onboarding") {
    if (!onboarding.active) enterOnboarding();
    return;
  }
  if (onboarding.active) leaveOnboarding();
  document.querySelector("blora-sidebar-layout").hidden = false;
  onboarding.lastHash = location.hash || "#/session";
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
    body: JSON.stringify({ mode: currentRunMode() }),
  });
  location.hash = `#/session/${session.id}`;
});

document.querySelector("#composer").addEventListener("submit", async (event) => {
  event.preventDefault();
  hideAlert();
  const box = fieldInput("#prompt") || prompt;
  const text = (box?.value || "").trim();
  if (!text) {
    return;
  }
  state.history.push(text);
  state.historyIndex = -1;
  state.draft = "";
  box.value = "";
  box.style.height = "";
  hideSlash();
  state.stick = true;
  if (text.startsWith("/")) {
    try { await runSlash(text); } catch (error) { showAlert(error.message); }
    return;
  }
  if (!state.sessionId) {
    const session = await api("/api/sessions", {
      method: "POST", body: JSON.stringify({ mode: currentRunMode() }),
    });
    await selectSession(session.id);
    location.hash = `#/session/${session.id}`;
  }
  const runSessionId = state.sessionId;
  // While a run is in flight the composer becomes a steering channel: the
  // message is queued and delivered at the next model-turn boundary.
  if (state.running) {
    await api(`/api/sessions/${state.sessionId}/steer`, {
      method: "POST",
      body: JSON.stringify({ message: text }),
    });
    showNotice("消息已排队，将在下一轮送达", "info", "插话已排队");
    return;
  }
  setRunning(true);
  try {
    await api(`/api/sessions/${runSessionId}/run`, {
      method: "POST",
      body: JSON.stringify({
        prompt: imaginePrompt(text),
        permission_mode: currentPermission(),
        auto_approve: currentPermission() === "yolo",
        confirmed: currentPermission() === "yolo",
        provider: state.session?.provider || stored("blora-provider"),
        model: state.session?.model || stored("blora-model"),
        worktree: stored("blora-worktree") === "on" || isChecked(document.querySelector("#pref-worktree")),
      }),
    });
  } catch (error) {
    showAlert(error.message);
    if (state.sessionId === runSessionId && !box.value) box.value = text;
  } finally {
    if (state.sessionId === runSessionId) setRunning(false);
  }
  if (state.sessionId === runSessionId) await selectSession(runSessionId);
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
    send.textContent = running ? "排队发送" : currentRunMode() === "imagine" ? "生成" : "发送";
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
    if (state.view === "approvals") await refreshApprovalPage();
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
    await onDone();
    if (state.view === "approvals") await refreshApprovalPage();
    await refreshApprovals();
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
  const id = state.sessionId;
  const version = state.selectionVersion;
  const rows = await api(`/api/approvals?session=${id}`);
  if (id !== state.sessionId || version !== state.selectionVersion) return;
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
  const context = state.sessionId ? `?session=${encodeURIComponent(state.sessionId)}` : "";
  const id = state.sessionId;
  const info = await api(`/api/workspace${context}`);
  if (id !== state.sessionId) return;
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
        const file = await api(`/api/workspace/file?path=${encodeURIComponent(name)}${state.sessionId ? `&session=${encodeURIComponent(state.sessionId)}` : ""}`);
        document.querySelector("#workspace-file").textContent = file.contents || "";
      });
    }
    list.append(item);
  }
  enhance();
}

async function refreshSettings() {
  await refreshPassport();
  const info = await api("/api/settings");
  const provider = fieldInput("#pref-provider") || document.querySelector("#pref-provider");
  const model = fieldInput("#pref-model") || document.querySelector("#pref-model");
  if (provider) provider.value = state.session?.provider || stored("blora-provider", info.provider);
  if (model) model.value = state.session?.model || stored("blora-model", info.model);
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
      prompt: ((fieldInput("#task-prompt") || document.querySelector("#task-prompt"))?.value || titleInput?.value || "").trim(),
      delay_seconds: numberValue(delayInput),
      cron: (cronInput?.value || "").trim() || null,
      auto_approve: false,
    }),
  });
  if (titleInput) {
    titleInput.value = "";
  }
  const taskPrompt = fieldInput("#task-prompt") || document.querySelector("#task-prompt");
  if (taskPrompt) {
    taskPrompt.value = "";
  }
  await refreshTasks();
});

const prefProvider = fieldInput("#pref-provider") || document.querySelector("#pref-provider");
const prefModel = fieldInput("#pref-model") || document.querySelector("#pref-model");
if (prefProvider) {
  prefProvider.addEventListener("change", () => runSlash(`/provider ${prefProvider.value}`).catch(error => showAlert(error.message)));
}
if (prefModel) {
  prefModel.addEventListener("change", () => runSlash(`/model ${prefModel.value}`).catch(error => showAlert(error.message)));
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
  promptBox.addEventListener("input", () => {
    growPrompt();
    state.historyIndex = -1;
    state.slashIndex = 0;
    renderSlash(promptBox.value);
  });
  promptBox.addEventListener("keydown", (event) => {
    const menu = document.querySelector("#slash-menu");
    const open = menu && !menu.hidden;
    if (open && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      event.preventDefault();
      const count = menu.querySelectorAll(".ba-slash__item").length;
      if (!count) {
        return;
      }
      state.slashIndex = (state.slashIndex + (event.key === "ArrowDown" ? 1 : -1) + count) % count;
      renderSlash(promptBox.value);
      return;
    }
    if (!open && (event.key === "ArrowUp" || event.key === "ArrowDown") && !event.shiftKey && !event.isComposing && (state.historyIndex >= 0 || !promptBox.value.includes("\n"))) {
      if (state.history.length) {
        event.preventDefault();
        if (state.historyIndex < 0) { state.draft = promptBox.value; state.historyIndex = state.history.length; }
        state.historyIndex = Math.max(0, Math.min(state.history.length, state.historyIndex + (event.key === "ArrowUp" ? -1 : 1)));
        promptBox.value = state.historyIndex === state.history.length ? state.draft : state.history[state.historyIndex];
        growPrompt();
      }
      return;
    }
    if (open && (event.key === "Tab" || event.key === "Enter")) {
      event.preventDefault();
      const picked = slashMatches(promptBox.value)[state.slashIndex];
      if (picked) {
        promptBox.value = `/${picked[0]} `;
        hideSlash();
        if (event.key === "Enter" && !picked[2]?.startsWith("<")) document.querySelector("#composer").requestSubmit();
      }
      return;
    }
    if (open && event.key === "Escape") {
      event.preventDefault();
      hideSlash();
      return;
    }
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      document.querySelector("#composer").requestSubmit();
    }
  });
}

function imaginePrompt(text) {
  if (currentRunMode() !== "imagine" || text.startsWith("/")) {
    return text;
  }
  const aspect = document.querySelector("#imagine-aspect")?.value || "1:1";
  const count = document.querySelector("#imagine-count")?.value || "1";
  return `${text}\n画幅 ${aspect}，张数 ${count}`;
}

function currentPermission() {
  const select = document.querySelector("#permission-mode");
  return select?.value || stored("blora-permission", "ask");
}

let syncingPermission = false;

const RUN_MODES = ["code", "work", "agent", "imagine"];

function currentRunMode() {
  const select = document.querySelector("#run-mode");
  return RUN_MODES.includes(select?.value) ? select.value : stored("blora-run-mode", "code");
}

const CODE_CHIPS = [
  ["列出当前工作区的文件结构", "列出工作区"],
  ["总结 git 状态和最近提交", "查看 git 状态"],
  ["阅读 README 并说明这个项目做什么", "阅读 README"],
  ["找出可以安全改进的测试缺口", "检查测试"],
];

const IMAGINE_CHIPS = [
  ["一张珊瑚配色的桌面壁纸，留出中间的空白", "珊瑚壁纸"],
  ["一张安静的书桌照片，暖光，没有文字", "书桌"],
  ["一张几何海报，只有色块和细线", "几何海报"],
];

function paintStudio(mode) {
  const imagine = mode === "imagine";
  const view = document.querySelector("#view-session");
  if (view) {
    view.classList.toggle("ba-session--imagine", imagine);
  }
  document.querySelectorAll(".ba-imagine-only").forEach((node) => {
    node.hidden = !imagine;
  });
  const send = document.querySelector("#send");
  if (send) {
    send.textContent = imagine ? "生成" : "发送";
  }
  const box = fieldInput("#prompt") || prompt;
  if (box) {
    box.placeholder = imagine
      ? "描述要生成的画面。输入 / 查看命令"
      : "要在这个工作区做什么？输入 / 查看命令";
  }
  const kicker = document.querySelector(".ba-hero__kicker");
  const hero = document.querySelector("#hero-title");
  const lede = document.querySelector(".ba-hero__lede");
  const chips = document.querySelector("#hero-chips");
  if (!kicker || !hero || !lede || !chips) {
    return;
  }
  if (imagine) {
    kicker.textContent = "Imagine";
    if (!state.sessionId || hero.dataset.studio !== "code-named") {
      hero.textContent = "描述一张图";
    }
    lede.textContent = "图片保存在工作区 .blora/images/。画幅和张数写在下面。";
  } else {
    kicker.textContent = "Code · Work · Agent";
    if (hero.dataset.studio !== "code-named") {
      hero.textContent = "工作区已就绪";
    }
    lede.textContent = "在仓库里读、改、跑。会话、工具和审批都留在本地。";
  }
  chips.replaceChildren();
  for (const [promptText, label] of imagine ? IMAGINE_CHIPS : CODE_CHIPS) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "ba-chip";
    button.dataset.prompt = promptText;
    button.textContent = label;
    chips.append(button);
  }
}

function applyRunMode(mode, persist = true) {
  const next = RUN_MODES.includes(mode) ? mode : "code";
  const select = document.querySelector("#run-mode");
  if (select) {
    select.value = next;
  }
  paintStudio(next);
  if (persist) {
    store("blora-run-mode", next);
  }
}

function applyPermission(mode, persist = true) {
  const aliases = { default: "ask", acceptEdits: "auto-edit", bypassPermissions: "yolo" };
  mode = aliases[mode] || mode;
  const allowed = ["ask", "plan", "auto-edit", "yolo"];
  const next = allowed.includes(mode) ? mode : "ask";
  state.permission = next;
  const select = document.querySelector("#permission-mode");
  if (select) {
    select.value = next;
  }
  if (autoApprove) {
    const value = next === "yolo" ? "on" : "off";
    if (autoApprove.getAttribute("value") !== value) {
      syncingPermission = true;
      autoApprove.setAttribute("value", value);
      syncingPermission = false;
    }
  }
  if (persist) {
    store("blora-permission", next);
  }
}

function slashToken(text) {
  const match = String(text || "").match(/^\/([^\s]*)$/);
  return match ? match[1] : null;
}

function hideSlash() {
  const menu = document.querySelector("#slash-menu");
  if (menu) {
    menu.hidden = true;
    menu.replaceChildren();
  }
}

function slashMatches(text) {
  const token = slashToken(text) || "";
  if (window.BloraCommands) return window.BloraCommands.matches(`/${token}`).map(c => [c.name, c.about, c.hint]);
  return SLASH.filter(row => row[0].startsWith(token));
}

function renderSlash(text) {
  const menu = document.querySelector("#slash-menu");
  const token = slashToken(text);
  if (!menu || token === null) {
    hideSlash();
    return;
  }
  const hits = slashMatches(text);
  if (!hits.length) {
    hideSlash();
    return;
  }
  state.slashIndex = Math.min(state.slashIndex, hits.length - 1);
  menu.hidden = false;
  menu.replaceChildren();
  hits.forEach((row, index) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "ba-slash__item";
    button.setAttribute("role", "option");
    button.setAttribute("aria-selected", index === state.slashIndex ? "true" : "false");
    const name = document.createElement("div");
    name.className = "ba-slash__name";
    name.textContent = `/${row[0]}`;
    const about = document.createElement("div");
    about.className = "ba-slash__about";
    about.textContent = row[1];
    button.append(name, about);
    button.addEventListener("mousedown", (event) => {
      event.preventDefault();
      const box = fieldInput("#prompt") || prompt;
      if (box) {
        box.value = `/${row[0]} `;
      }
      hideSlash();
      box?.focus();
    });
    menu.append(button);
  });
}

async function runSlash(raw) {
  if (window.BloraCommands && await window.BloraCommands.execute(raw, window.BloraWeb)) return;
  const parts = raw.trim().slice(1).split(/\s+/);
  const token = (parts.shift() || "").toLowerCase();
  const name = window.BloraCommands?.resolve(token)?.name || token;
  const rest = parts.join(" ");
  const go = (view) => {
    location.hash = `#/${view}`;
  };
  if (name === "help" || name === "?") {
    showNotice(SLASH.map((row) => `/${row[0]} ${row[1]}`).join(" · "), "info", "命令");
    return;
  }
  if (name === "login" || name === "onboarding") {
    enterOnboarding(name === "login" ? "account" : "welcome");
    return;
  }
  if (name === "new") {
    const session = await api("/api/sessions", { method: "POST", body: "{}" });
    location.hash = `#/session/${session.id}`;
    return;
  }
  if (name === "imagine") {
    const session = await api("/api/sessions", {
      method: "POST",
      body: JSON.stringify({ mode: "imagine", title: "imagine" }),
    });
    location.hash = `#/session/${session.id}`;
    return;
  }
  if (name === "mode" && rest) {
    const session = await api("/api/sessions", {
      method: "POST",
      body: JSON.stringify({ mode: rest, title: rest }),
    });
    location.hash = `#/session/${session.id}`;
    return;
  }
  if (name === "yes" || name === "yolo") {
    if (await window.BloraWorkbench?.requestPermission("yolo")) applyPermission("yolo");
    return;
  }
  if (name === "no") {
    applyPermission("ask");
    return;
  }
  if (name === "plan") {
    applyPermission("plan");
    return;
  }
  if (name === "model") {
    if (rest) {
      store("blora-model", rest);
      const model = fieldInput("#pref-model");
      if (model) model.value = rest;
    }
    showNotice(`模型 ${stored("blora-model") || "默认"}`, "info", "模型");
    return;
  }
  if (name === "provider") {
    if (rest) {
      store("blora-provider", rest);
      const provider = fieldInput("#pref-provider");
      if (provider) provider.value = rest;
    }
    showNotice(`供应商 ${stored("blora-provider") || "默认"}`, "info", "供应商");
    return;
  }
  if (name === "theme" || name === "settings") {
    go("settings");
    return;
  }
  if (["tasks", "approvals", "timeline", "artifacts"].includes(name)) {
    go(name);
    return;
  }
  if (name === "git" || name === "files") {
    go("workspace");
    return;
  }
  if (name === "usage") {
    const query = state.sessionId ? `?session=${state.sessionId}` : "";
    const usage = await api(`/api/usage${query}`);
    showNotice(`输入 ${usage.input_tokens || 0} · 输出 ${usage.output_tokens || 0}`, "info", "用量");
    return;
  }
  if (name === "read" && rest) {
    go("workspace");
    const file = await api(`/api/workspace/file?path=${encodeURIComponent(rest)}${state.sessionId ? `&session=${encodeURIComponent(state.sessionId)}` : ""}`);
    document.querySelector("#workspace-file").textContent = file.contents || "";
    return;
  }
  if (name === "install" && rest) {
    await api("/api/marketplace", { method: "POST", body: JSON.stringify({ name: rest }) });
    showNotice(`已安装 ${rest}`, "info", "插件");
    return;
  }
  if (!state.sessionId && ["compact", "fork", "cancel", "archive", "resume", "export", "steer"].includes(name)) {
    showAlert("先打开一个会话");
    return;
  }
  if (name === "compact") {
    await api(`/api/sessions/${state.sessionId}/compact`, { method: "POST", body: "{}" });
    await selectSession(state.sessionId);
    return;
  }
  if (name === "fork") {
    const child = await api(`/api/sessions/${state.sessionId}/fork`, { method: "POST", body: "{}" });
    location.hash = `#/session/${child.id}`;
    return;
  }
  if (name === "cancel" || name === "stop") {
    await api(`/api/sessions/${state.sessionId}/cancel`, { method: "POST", body: "{}" });
    return;
  }
  if (name === "archive") {
    await api(`/api/sessions/${state.sessionId}/archive`, { method: "POST", body: "{}" });
    await selectSession(state.sessionId);
    return;
  }
  if (name === "resume") {
    await api(`/api/sessions/${state.sessionId}/resume`, { method: "POST", body: "{}" });
    await selectSession(state.sessionId);
    return;
  }
  if (name === "export") {
    await downloadExport(state.sessionId);
    return;
  }
  if (name === "steer") {
    if (!rest) {
      showAlert("用法：/steer 要补充的话");
      return;
    }
    await api(`/api/sessions/${state.sessionId}/steer`, {
      method: "POST",
      body: JSON.stringify({ message: rest }),
    });
    return;
  }
  showAlert(`未知命令 /${name}。输入 /help 查看。`);
}

async function downloadExport(id) {
  const response = await fetch(`/api/sessions/${id}/export`, {
    headers: stored("blora-token") ? { authorization: `Bearer ${stored("blora-token")}` } : {},
  });
  if (!response.ok) {
    throw new Error(await response.text());
  }
  const blob = await response.blob();
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = `${id}.jsonl`;
  link.click();
  URL.revokeObjectURL(link.href);
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

document.querySelector("#hero-chips")?.addEventListener("click", (event) => {
  const chip = event.target.closest(".ba-chip");
  const box = fieldInput("#prompt") || prompt;
  if (!chip || !box) {
    return;
  }
  box.value = chip.getAttribute("data-prompt") || "";
  box.focus();
});

const savedPermission = stored("blora-permission", "ask");
applyPermission(["yolo", "bypassPermissions"].includes(savedPermission) ? "ask" : savedPermission, false);
const runModeSelect = document.querySelector("#run-mode");
if (runModeSelect) {
  applyRunMode(stored("blora-run-mode", "code"), false);
  runModeSelect.addEventListener("change", async () => {
    applyRunMode(runModeSelect.value);
    if (!state.sessionId) {
      return;
    }
    if (state.running) {
      applyRunMode(state.session?.mode || "code", false);
      showAlert("运行中不能切换工作模式");
      return;
    }
    try {
      await api(`/api/sessions/${state.sessionId}/mode`, {
        method: "POST",
        body: JSON.stringify({ mode: currentRunMode() }),
      });
      await selectSession(state.sessionId);
    } catch (error) {
      applyRunMode(state.session?.mode || "code", false);
      showAlert(error.message);
    }
  });
}

const permissionSelect = document.querySelector("#permission-mode");
if (permissionSelect) {
  permissionSelect.addEventListener("change", async () => {
    const next = permissionSelect.value;
    applyPermission(state.permission || "ask", false);
    if (window.BloraWorkbench && await window.BloraWorkbench.requestPermission(next)) applyPermission(next);
    else if (!["yolo", "bypassPermissions"].includes(next)) applyPermission(next);
  });
}
if (autoApprove) {
  autoApprove.addEventListener("blora-change", () => {
    if (syncingPermission) {
      return;
    }
    const next = isChecked(autoApprove) ? "yolo" : "ask";
    applyPermission(state.permission || "ask", false);
    window.BloraWorkbench?.requestPermission(next).then(allowed => { if (allowed) applyPermission(next); });
  });
}
for (const [id, action] of [
  ["#archive", "archive"],
  ["#resume", "resume"],
]) {
  document.querySelector(id)?.addEventListener("click", async () => {
    if (!state.sessionId) {
      return;
    }
    hideAlert();
    await api(`/api/sessions/${state.sessionId}/${action}`, { method: "POST", body: "{}" });
    await selectSession(state.sessionId);
  });
}
document.querySelector("#export-session")?.addEventListener("click", async () => {
  if (!state.sessionId) {
    return;
  }
  hideAlert();
  await downloadExport(state.sessionId);
});


enhance();

window.BloraWeb = {
  api, getState: () => state, stored, store, applyPermission, applyRunMode,
  refreshSessions, selectSession, runSlash, showNotice, showAlert, refreshSettings,
  enterOnboarding: (...args) => enterOnboarding(...args),
  refreshPassport: (...args) => refreshPassport(...args),
  setRunning, renderUsage,
  renderTranscript: () => renderTranscript(state.session?.transcript || []),
  setSearch: query => { state.transcriptQuery = query; renderTranscript(state.session?.transcript || []); },
};
document.addEventListener("keydown", event => {
  if (window.BloraDialogs?.top?.()) return;
  if (event.ctrlKey && event.key.toLowerCase() === "p" && !promptBox.value) {
    event.preventDefault(); promptBox.value = "/"; promptBox.focus(); renderSlash("/");
  }
});
