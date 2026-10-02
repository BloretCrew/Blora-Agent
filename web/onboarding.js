// SPDX-License-Identifier: GPL-3.0-or-later
// Shared lexical state is initialized after the app DOM bindings.
const ONBOARDING_VERSION = "1";
const ONBOARDING_KEY = "blora-web-onboarding-version";
const onboarding = {
  active: false, step: "welcome", returnHash: "#/session", lastHash: "#/session",
  user: null, accountError: "", gateway: null, message: "", busy: false,
  attempt: null, generation: 0, timer: null, expiryTimer: null, accountGeneration: 0, service: null, workspace: null,
};
const ob = (name) => document.querySelector(`#onboarding-${name}`);

function syncPassport() {
  const user = onboarding.user;
  const valid = user?.credentials_valid === true;
  passportLogin.hidden = valid;
  passportLogin.textContent = user ? "重新登录 PassPort" : "PassPort 登录";
  passportUser.hidden = !user;
  passportUser.textContent = user ? `${user.name || user.username || "PassPort 账号"}${valid ? "" : " · 需要重新登录"}` : "";
  passportLogout.hidden = !user;
  const hero = document.querySelector("#hero-title");
  if (hero && !state.sessionId && currentRunMode() !== "imagine") {
    hero.textContent = valid ? `继续，${user.name || user.username}` : "工作区已就绪";
  }
  document.querySelector("#settings-account").textContent = onboarding.accountError ||
    (user ? passportUser.textContent : "尚未连接 PassPort，可使用已配置的供应商。");
  document.querySelector("#settings-login").textContent = valid ? "查看账号" : "连接账号";
}

async function refreshPassport() {
  const generation = ++onboarding.accountGeneration;
  try {
    const user = await api("/api/auth/me", { silent: true });
    if (generation !== onboarding.accountGeneration) return;
    onboarding.user = user;
    onboarding.accountError = "";
  } catch (error) {
    if (generation !== onboarding.accountGeneration) return;
    onboarding.user = null;
    onboarding.accountError = error.status === 401 ? "" : `无法检查账号：${error.message}`;
  }
  syncPassport();
  if (onboarding.active) renderOnboarding();
}

async function checkOnboardingService() {
  onboarding.gateway = null;
  try {
    const info = await api("/api/settings", { silent: true });
    if (typeof info.gateway !== "boolean") throw new Error("服务未返回 gateway 策略");
    onboarding.gateway = info.gateway;
    onboarding.service = info;
    if (!info.gateway || stored("blora-token")) {
      try { onboarding.workspace = (await api("/api/workspace", { silent: true })).path; }
      catch (_) { onboarding.workspace = null; }
    }
  } catch (error) {
    onboarding.message = `无法确认服务策略：${error.message}。请重试检查；暂不提供跳过。`;
  }
  if (onboarding.active) renderOnboarding();
}

function markOnboardingComplete() {
  try {
    localStorage.setItem(ONBOARDING_KEY, ONBOARDING_VERSION);
    return true;
  } catch (_) {
    return false;
  }
}

function cancelDeviceAttempt() {
  onboarding.generation += 1;
  clearTimeout(onboarding.timer);
  clearTimeout(onboarding.expiryTimer);
  const attempt = onboarding.attempt;
  onboarding.attempt = null;
  onboarding.busy = false;
  if (attempt) cancelDeviceId(attempt.attempt_id);
}

function cancelDeviceId(attempt_id) {
  return api("/api/auth/device/cancel", {
    method: "POST", body: JSON.stringify({ attempt_id }), silent: true, keepalive: true,
  }).catch(() => {});
}

function renderOnboarding(focus = false) {
  const step = onboarding.step;
  const valid = onboarding.user?.credentials_valid === true;
  const content = {
    welcome: ["让工作从这里开始", "Blora 在你的工作区中读、改、跑。先连接账号，或保留现有供应商配置，稍后再设置。"],
    account: ["连接你的账号", onboarding.gateway === true ? "此服务启用了网关认证。连接 PassPort 不会替代网关访问凭据。" : "连接 Bloret PassPort 使用 Blora 服务；已有自定义供应商配置不会被更改。"],
    complete: ["准备好了", valid ? `已连接 ${onboarding.user.name || onboarding.user.username}，现在可以开始使用。` : "账号可以稍后连接。使用前仍需配置可用的供应商与凭据。"],
  };
  ob("title").textContent = content[step][0];
  ob("description").textContent = content[step][1];
  document.querySelectorAll("[data-step]").forEach((el) => {
    if (el.dataset.step === step) el.setAttribute("aria-current", "step");
    else el.removeAttribute("aria-current");
  });
  document.querySelector("#view-onboarding").dataset.stage = step;
  ob("welcome").hidden = step !== "welcome";
  ob("complete").hidden = step !== "complete";
  ob("kicker").textContent = step === "welcome" ? "首次使用 · 大约一分钟" : step === "account" ? "账号连接 · 安全授权" : "设置完成 · 开始探索";
  ob("summary-account").textContent = valid ? onboarding.user.name || onboarding.user.username : "稍后连接";
  ob("summary-provider").textContent = stored("blora-provider") || (valid ? "Bloret PassPort" : onboarding.service?.provider_display) || "使用当前配置";
  ob("summary-workspace").textContent = onboarding.workspace || pathEl.textContent || "当前服务工作区";
  ob("account-badge").textContent = valid ? "已连接" : onboarding.attempt ? "等待授权" : onboarding.busy ? "连接中" : "待连接";
  ob("account-badge").dataset.connected = String(valid);
  ob("retry").hidden = !onboarding.attempt;
  ob("status").hidden = !onboarding.message;
  ob("account").hidden = step !== "account";
  ob("account-name").textContent = onboarding.accountError || (onboarding.user ?
    `${onboarding.user.name || onboarding.user.username} · ${valid ? "账号可用" : "凭据已失效，请重新登录"}` : "尚未连接账号");
  ob("device").hidden = !onboarding.attempt;
  ob("login").hidden = valid || Boolean(onboarding.attempt);
  ob("login").disabled = onboarding.busy;
  ob("login").textContent = onboarding.busy ? "正在申请设备码…" : "登录 Bloret PassPort";
  ob("next").hidden = step === "account" && !valid;
  ob("next").textContent = step === "welcome" ? "开始设置" : step === "account" ? "继续" : "开始使用";
  ob("skip").hidden = step !== "account" || onboarding.gateway !== false || valid;
  ob("back").hidden = step === "welcome";
  ob("status").textContent = onboarding.message;
  if (focus) ob("title").focus();
  enhance();
}

function enterOnboarding(step = "welcome") {
  if (!onboarding.active) {
    onboarding.returnHash = location.hash === "#/onboarding" ? onboarding.lastHash : location.hash || "#/session";
    onboarding.active = true;
  }
  cancelDeviceAttempt();
  onboarding.step = step;
  onboarding.message = "";
  document.querySelector("blora-sidebar-layout").hidden = true;
  document.querySelector("#view-onboarding").hidden = false;
  location.hash = "#/onboarding";
  renderOnboarding(true);
  refreshPassport();
  checkOnboardingService();
}

function leaveOnboarding() {
  cancelDeviceAttempt();
  onboarding.active = false;
  document.querySelector("blora-sidebar-layout").hidden = false;
  document.querySelector("#view-onboarding").hidden = true;
}

function returnFromOnboarding() {
  const hash = onboarding.returnHash;
  leaveOnboarding();
  location.hash = hash;
}

function failDeviceAttempt(message) {
  cancelDeviceAttempt();
  onboarding.message = message;
  if (onboarding.active) renderOnboarding();
}

async function startDeviceAttempt() {
  cancelDeviceAttempt();
  const generation = onboarding.generation;
  onboarding.busy = true;
  onboarding.message = "正在申请设备码…";
  renderOnboarding();
  try {
    const attempt = await api("/api/auth/device", { silent: true });
    if (generation !== onboarding.generation || !onboarding.active) {
      if (attempt.attempt_id) cancelDeviceId(attempt.attempt_id);
      return;
    }
    const url = new URL(attempt.verification_uri);
    if (!attempt.attempt_id || !attempt.user_code || url.protocol !== "https:" ||
        !Number.isFinite(Number(attempt.expires_in)) || Number(attempt.expires_in) <= 0) {
      if (attempt.attempt_id) cancelDeviceId(attempt.attempt_id);
      throw new Error("服务返回了无效的设备授权信息");
    }
    attempt.interval = Math.max(1, Number(attempt.interval) || 5);
    attempt.expiresAt = Date.now() + Number(attempt.expires_in) * 1000;
    onboarding.attempt = attempt;
    onboarding.busy = false;
    ob("code").value = attempt.user_code;
    ob("link").href = url.href;
    ob("expiry").textContent = `设备码有效期 ${Math.ceil(Number(attempt.expires_in) / 60)} 分钟`;
    onboarding.message = "等待授权，请打开授权页面并输入设备码。";
    onboarding.expiryTimer = setTimeout(() => {
      if (generation === onboarding.generation) failDeviceAttempt("设备码已过期，请重新登录。");
    }, Number(attempt.expires_in) * 1000);
    renderOnboarding();
    scheduleDevicePoll(generation);
  } catch (error) {
    if (generation === onboarding.generation) failDeviceAttempt(`无法开始登录：${error.message}。请重试。`);
  }
}

function scheduleDevicePoll(generation) {
  const attempt = onboarding.attempt;
  if (!attempt) return;
  onboarding.timer = setTimeout(() => pollDeviceAttempt(generation), attempt.interval * 1000);
}

async function pollDeviceAttempt(generation) {
  const attempt = onboarding.attempt;
  if (!attempt || generation !== onboarding.generation) return;
  if (Date.now() >= attempt.expiresAt) {
    failDeviceAttempt("设备码已过期，请重新登录。");
    return;
  }
  try {
    const result = await api("/api/auth/device/poll", {
      method: "POST", body: JSON.stringify({ attempt_id: attempt.attempt_id }), silent: true,
    });
    if (generation !== onboarding.generation || !onboarding.active) return;
    if (result.status === "authenticated") {
      clearTimeout(onboarding.expiryTimer);
      onboarding.attempt = null;
      onboarding.generation += 1;
      onboarding.message = "授权成功，正在确认账号凭据…";
      await refreshPassport();
      if (!onboarding.active || generation + 1 !== onboarding.generation) return;
      onboarding.message = onboarding.user?.credentials_valid === true ? "账号已连接，点击继续。" :
        "授权已完成，但尚未确认可用凭据，请重新检查账号。";
      syncPassport();
      renderOnboarding();
      refreshSessions().catch((error) => showAlert(error.message));
      return;
    }
    if (result.status !== "pending" && result.status !== "slow_down") {
      throw new Error(result.status === "expired" ? "设备码已过期" :
        result.status === "denied" || result.status === "access_denied" ? "授权被拒绝" : "未知授权状态");
    }
    const interval = Number(result.interval);
    attempt.interval = result.status === "slow_down" ? Math.max(attempt.interval + 5, interval || 0) :
      Math.max(attempt.interval, interval || 0);
    onboarding.message = result.status === "slow_down" ? "授权服务繁忙，已放慢检查频率。" : "等待授权…";
    renderOnboarding();
    scheduleDevicePoll(generation);
  } catch (error) {
    if (generation === onboarding.generation) failDeviceAttempt(`登录未完成：${error.message}。请重新登录。`);
  }
}

passportLogin.addEventListener("click", (event) => {
  event.preventDefault();
  enterOnboarding("account");
});
document.querySelector("#settings-login").addEventListener("click", () => enterOnboarding("account"));
document.querySelector("#settings-onboarding").addEventListener("click", () => enterOnboarding());
ob("login").addEventListener("click", startDeviceAttempt);
ob("retry").addEventListener("click", startDeviceAttempt);
ob("check").addEventListener("click", async () => {
  onboarding.message = "";
  await Promise.all([refreshPassport(), checkOnboardingService()]);
});
ob("next").addEventListener("click", () => {
  if (onboarding.step === "welcome") onboarding.step = "account";
  else if (onboarding.step === "account") {
    if (onboarding.user?.credentials_valid !== true) return;
    cancelDeviceAttempt();
    onboarding.step = "complete";
  } else {
    const saved = markOnboardingComplete();
    returnFromOnboarding();
    if (!saved) showNotice("浏览器无法保存引导记录，下次访问可能再次显示引导。", "info", "设置已完成");
    return;
  }
  onboarding.message = "";
  renderOnboarding(true);
});
ob("skip").addEventListener("click", () => {
  if (onboarding.gateway !== false) return;
  cancelDeviceAttempt();
  onboarding.step = "complete";
  onboarding.message = "";
  renderOnboarding(true);
});
ob("back").addEventListener("click", () => {
  cancelDeviceAttempt();
  onboarding.step = onboarding.step === "complete" ? "account" : "welcome";
  onboarding.message = "";
  renderOnboarding(true);
});
ob("cancel").addEventListener("click", returnFromOnboarding);
ob("copy").addEventListener("click", async () => {
  const attempt = onboarding.attempt;
  if (!attempt) return;
  try {
    await navigator.clipboard.writeText(attempt.user_code);
    if (onboarding.attempt === attempt) onboarding.message = "设备码已复制。";
  } catch (_) {
    if (onboarding.attempt !== attempt) return;
    ob("code").focus();
    ob("code").select();
    onboarding.message = "无法访问剪贴板，请手动复制已选中的设备码。";
  }
  if (onboarding.active) renderOnboarding();
});
passportLogout.addEventListener("click", async () => {
  cancelDeviceAttempt();
  try {
    await api("/api/auth/logout", { method: "POST", body: "{}" });
    onboarding.accountGeneration += 1;
    onboarding.user = null;
    onboarding.accountError = "";
    syncPassport();
    await refreshSessions();
    if (!onboarding.active) await applyRoute();
  } catch (error) { showAlert(error.message); }
});
window.addEventListener("pagehide", cancelDeviceAttempt);


async function bootWeb() {
  await refreshPassport();
  if (location.hash === "#/onboarding") {
    enterOnboarding();
  } else if (stored(ONBOARDING_KEY) !== ONBOARDING_VERSION) {
    if (onboarding.user?.credentials_valid === true) {
      markOnboardingComplete();
      await applyRoute();
    } else {
      enterOnboarding();
    }
  } else {
    await applyRoute();
  }
}

bootWeb().catch((error) => showAlert(error.message));

// Cookies change across tabs; refresh every surface that reads account state.
window.addEventListener("focus", () => {
  refreshPassport().then(() => {
    if (!onboarding.active) return applyRoute();
  }).catch((error) => showAlert(error.message));
});
window.addEventListener("storage", (event) => {
  if (event.key === ONBOARDING_KEY || event.key === "blora-token") {
    refreshPassport().then(() => {
      if (event.key === "blora-token" && onboarding.active) return checkOnboardingService();
      if (!onboarding.active) return applyRoute();
    }).catch((error) => showAlert(error.message));
  }
});
