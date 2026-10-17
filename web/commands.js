// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

(() => {
  "use strict";

  const catalog = Object.freeze([
    {"name": "help", "aliases": ["?"], "hint": "[query]", "about": "列出或筛选命令", "group": "system"},
    {"name": "login", "aliases": [], "hint": "", "about": "连接 Bloret PassPort", "group": "system"},
    {"name": "onboarding", "aliases": [], "hint": "", "about": "重新打开首次使用引导", "group": "system"},
    {"name": "logout", "aliases": [], "hint": "", "about": "退出当前浏览器的 PassPort 账号", "group": "system"},
    {"name": "keymap", "aliases": ["keys", "shortcuts"], "hint": "", "about": "查看浏览器快捷键适配说明", "group": "system"},
    {"name": "theme", "aliases": ["appearance"], "hint": "[coral|indigo|…|dark|light]", "about": "打开主题选择或预览指定主题", "group": "system"},
    {"name": "settings", "aliases": [], "hint": "[key=value]", "about": "打开设置或修改 key=value 设置", "group": "system"},
    {"name": "harness", "aliases": [], "hint": "", "about": "选择前台执行内核", "group": "session"},
    {"name": "new", "aliases": [], "hint": "", "about": "新建代码会话", "group": "session"},
    {"name": "sessions", "aliases": ["ls-sessions"], "hint": "", "about": "查找和选择会话", "group": "session"},
    {"name": "goto", "aliases": ["open", "switch"], "hint": "<id|title>", "about": "按 ID 或标题切换会话", "group": "session"},
    {"name": "status", "aliases": ["info", "session-info"], "hint": "", "about": "查看当前会话状态", "group": "session"},
    {"name": "id", "aliases": [], "hint": "", "about": "显示当前会话 ID", "group": "session"},
    {"name": "context", "aliases": [], "hint": "", "about": "查看上下文、令牌和工作区", "group": "session"},
    {"name": "compact", "aliases": [], "hint": "[note]", "about": "压缩会话历史", "group": "run"},
    {"name": "checkpoint", "aliases": [], "hint": "", "about": "写入恢复检查点", "group": "run"},
    {"name": "search", "aliases": ["find-transcript"], "hint": "[query]", "about": "筛选当前会话记录", "group": "session"},
    {"name": "find", "aliases": ["grep"], "hint": "[query]", "about": "搜索会话标题和记录", "group": "session"},
    {"name": "clear", "aliases": [], "hint": "", "about": "清除会话记录筛选", "group": "session"},
    {"name": "tools", "aliases": [], "hint": "", "about": "切换工具调用显示", "group": "session"},
    {"name": "reasoning", "aliases": ["thinking"], "hint": "", "about": "展开或折叠模型思考过程", "group": "session"},
    {"name": "copy", "aliases": [], "hint": "", "about": "复制最后一条助手回复", "group": "session"},
    {"name": "cancel", "aliases": ["stop"], "hint": "", "about": "停止当前运行", "group": "run"},
    {"name": "steer", "aliases": ["interject"], "hint": "<message>", "about": "为运行中的回合排队插话", "group": "run"},
    {"name": "yes", "aliases": ["yolo", "always-approve", "auto"], "hint": "[on|off]", "about": "请求绕过权限（须风险确认）或恢复默认", "group": "run"},
    {"name": "no", "aliases": [], "hint": "", "about": "恢复默认权限模式", "group": "run"},
    {"name": "permissions", "aliases": ["policy"], "hint": "", "about": "查看和选择权限、网络及隔离策略", "group": "run"},
    {"name": "approvals", "aliases": [], "hint": "", "about": "查看待处理审批", "group": "run"},
    {"name": "model", "aliases": ["m"], "hint": "[name]", "about": "选择供应商和模型或设置模型覆盖", "group": "run"},
    {"name": "provider", "aliases": [], "hint": "[name]", "about": "选择供应商或设置供应商覆盖", "group": "run"},
    {"name": "mode", "aliases": [], "hint": "[code|work|agent|imagine]", "about": "选择模式或在指定模式新建会话", "group": "session"},
    {"name": "code", "aliases": [], "hint": "", "about": "新建代码会话", "group": "session"},
    {"name": "work", "aliases": [], "hint": "", "about": "新建工作会话", "group": "session"},
    {"name": "agent", "aliases": [], "hint": "", "about": "新建代理会话", "group": "session"},
    {"name": "imagine", "aliases": [], "hint": "", "about": "新建 Imagine 生图会话", "group": "session"},
    {"name": "plan", "aliases": [], "hint": "", "about": "切换只读计划权限模式", "group": "run"},
    {"name": "exec", "aliases": ["sandbox"], "hint": "", "about": "查看服务端执行隔离", "group": "run"},
    {"name": "worktree", "aliases": [], "hint": "", "about": "查看服务端 worktree 标志", "group": "run"},
    {"name": "tasks", "aliases": [], "hint": "", "about": "查看后台任务", "group": "work"},
    {"name": "task", "aliases": [], "hint": "<prompt>", "about": "在当前会话排队后台任务", "group": "work"},
    {"name": "cron", "aliases": [], "hint": "<m h dom mon dow> <prompt>", "about": "创建五字段 cron 周期任务", "group": "work"},
    {"name": "loop", "aliases": [], "hint": "<duration> <prompt>", "about": "创建延迟任务（非周期循环）", "group": "work"},
    {"name": "pause", "aliases": [], "hint": "<task-id>", "about": "暂停后台任务", "group": "work"},
    {"name": "unpause", "aliases": ["resume-task"], "hint": "<task-id>", "about": "恢复暂停的后台任务", "group": "work"},
    {"name": "cancel-task", "aliases": [], "hint": "<task-id>", "about": "取消后台任务", "group": "work"},
    {"name": "pump", "aliases": [], "hint": "", "about": "执行一次到期任务调度", "group": "work"},
    {"name": "fork", "aliases": [], "hint": "", "about": "派生当前会话", "group": "session"},
    {"name": "resume", "aliases": [], "hint": "", "about": "恢复当前会话", "group": "session"},
    {"name": "archive", "aliases": [], "hint": "", "about": "归档当前会话", "group": "session"},
    {"name": "export", "aliases": [], "hint": "", "about": "下载规范事件日志", "group": "session"},
    {"name": "timeline", "aliases": ["events"], "hint": "", "about": "查看规范事件时间线", "group": "session"},
    {"name": "usage", "aliases": [], "hint": "", "about": "查看令牌用量", "group": "session"},
    {"name": "memory", "aliases": ["mem"], "hint": "", "about": "列出工作区记忆", "group": "memory"},
    {"name": "remember", "aliases": [], "hint": "<key> <value>", "about": "保存工作区记忆", "group": "memory"},
    {"name": "recall", "aliases": [], "hint": "[key]", "about": "读取记忆或列出全部记忆", "group": "memory"},
    {"name": "forget", "aliases": [], "hint": "<key>", "about": "删除工作区记忆", "group": "memory"},
    {"name": "distill", "aliases": ["flush"], "hint": "", "about": "从会话提炼记忆", "group": "memory"},
    {"name": "plugins", "aliases": ["plugin"], "hint": "", "about": "列出已安装插件", "group": "plugins"},
    {"name": "marketplace", "aliases": ["market"], "hint": "", "about": "查看插件市场", "group": "plugins"},
    {"name": "install", "aliases": [], "hint": "<name>", "about": "安装市场插件", "group": "plugins"},
    {"name": "uninstall", "aliases": ["remove"], "hint": "<name>", "about": "卸载插件", "group": "plugins"},
    {"name": "skills", "aliases": [], "hint": "", "about": "列出工作区技能", "group": "plugins"},
    {"name": "mcp", "aliases": ["mcps"], "hint": "", "about": "查看服务端 MCP 配置", "group": "plugins"},
    {"name": "agents", "aliases": ["subagents"], "hint": "", "about": "查看当前会话子代理", "group": "work"},
    {"name": "git", "aliases": [], "hint": "", "about": "打开 Git 工作台", "group": "git"},
    {"name": "diff", "aliases": [], "hint": "", "about": "查看 Git 差异", "group": "git"},
    {"name": "log", "aliases": [], "hint": "", "about": "查看 Git 日志", "group": "git"},
    {"name": "files", "aliases": ["ls"], "hint": "", "about": "浏览工作区文件", "group": "git"},
    {"name": "read", "aliases": [], "hint": "<path>", "about": "读取工作区文件", "group": "git"},
    {"name": "artifacts", "aliases": [], "hint": "", "about": "查看会话产物", "group": "session"},
    {"name": "pwd", "aliases": [], "hint": "", "about": "显示工作区路径", "group": "session"},
    {"name": "doctor", "aliases": [], "hint": "", "about": "查看运行环境诊断", "group": "system"},
    {"name": "hooks", "aliases": [], "hint": "", "about": "查看服务端 Hook 配置", "group": "plugins"},
    {"name": "init", "aliases": [], "hint": "", "about": "初始化工作区规则", "group": "system"},
    {"name": "users", "aliases": [], "hint": "", "about": "列出网关用户", "group": "system"},
    {"name": "web", "aliases": [], "hint": "", "about": "说明浏览器与 Web 服务的关系", "group": "system"},
    {"name": "reload", "aliases": [], "hint": "", "about": "刷新会话列表", "group": "session"},
    {"name": "next", "aliases": [], "hint": "", "about": "切换到下一会话", "group": "session"},
    {"name": "prev", "aliases": [], "hint": "", "about": "切换到上一会话", "group": "session"},
    {"name": "quit", "aliases": ["q", "exit"], "hint": "", "about": "说明如何离开浏览器界面", "group": "system"},
  ].map((row) => Object.freeze({ ...row, aliases: Object.freeze(row.aliases) })));

  function resolve(name) {
    const token = String(name || "").trim().replace(/^\/+/, "").toLowerCase();
    return catalog.find((row) => row.name === token || row.aliases.includes(token)) || null;
  }

  function rankName(name, token) {
    if (name === token) return 0;
    if (name.startsWith(token)) return 1;
    if (name.includes(token)) return 2;
    let index = 0;
    for (const char of name) {
      if (char === token[index]) index += 1;
    }
    return index === token.length ? 3 : Infinity;
  }

  function matches(input) {
    const text = String(input || "");
    if (!text.startsWith("/")) return [];
    const token = text.slice(1).trim().split(/\s+/)[0].toLowerCase();
    if (!token) return catalog.slice();
    return catalog.map((row) => ({ row, rank: Math.min(...[row.name, ...row.aliases].map((name) => rankName(name, token))) }))
      .filter((hit) => Number.isFinite(hit.rank))
      .sort((a, b) => a.rank - b.rank || a.row.name.length - b.row.name.length || (a.row.name < b.row.name ? -1 : 1))
      .map((hit) => hit.row);
  }

  const modes = ["code", "work", "agent", "imagine"];
  const permissionAliases = { ask: "default", "auto-edit": "acceptEdits", yolo: "bypassPermissions" };
  const permissions = ["default", "plan", "acceptEdits", "bypassPermissions"];
  const fallback = new Set(["compact", "fork", "export", "archive", "resume", "cancel", "steer", "usage", "read", "install", "approvals", "timeline", "artifacts"]);
  const dialogs = { theme: "theme", settings: "settings", harness: "harness", sessions: "session", context: "context", permissions: "permission", model: "model", provider: "provider", mode: "mode", tasks: "tasks", git: "git", files: "folder", find: "session", search: "search", clear: "search", tools: "tools" };

  async function execute(raw, bridge) {
    const parsed = String(raw || "").trim().match(/^\/([^\s]+)(?:\s+([\s\S]*))?$/);
    if (!parsed) return false;
    const command = resolve(parsed[1]);
    if (!command) return false;
    const name = command.name;
    const args = (parsed[2] || "").trim();
    if (fallback.has(name)) return false;
    const state = bridge.getState() || {};
    const notice = (text, title = "命令") => bridge.showNotice(String(text), "info", title);
    const post = (path, body = {}) => bridge.api(path, { method: "POST", body: JSON.stringify(body) });
    const sessionId = () => {
      if (!state.sessionId) throw new Error("先打开一个会话");
      return encodeURIComponent(state.sessionId);
    };
    const setRouting = async (key, value) => {
      if (state.running) throw new Error("运行中不能切换供应商或模型");
      if (state.sessionId) {
        const session = await bridge.api(`/api/sessions/${sessionId()}`);
        await post(`/api/sessions/${sessionId()}/routing`, {
          provider: key === "provider" ? value : session.provider || bridge.stored("blora-provider", ""),
          model: key === "model" ? value : session.model || bridge.stored("blora-model", ""),
        });
      }
      bridge.store(`blora-${key}`, value);
      if (state.sessionId && bridge.getState()?.sessionId === state.sessionId) await bridge.selectSession(state.sessionId);
      notice(`${key}=${value}（仅用于 Blora 原生运行）`, "Blora 原生设置");
    };
    const currentPermission = () => {
      const value = state.permission || state.permissionMode || state.permission_mode || bridge.stored("blora-permission", "default");
      return permissionAliases[value] || value;
    };
    const open = async (kind, options = {}) => {
      if (typeof window.BloraWorkbench?.open !== "function") {
        throw new Error(`/${name} 需要工作台对话框模块，当前尚未接入`);
      }
      await window.BloraWorkbench.open(kind, { bridge, args, ...options });
    };
    const requestPermission = async (value) => {
      const next = permissionAliases[value] || value;
      if (!permissions.includes(next)) throw new Error("权限模式：default、plan、acceptEdits、bypassPermissions");
      // Stored preferences are not proof of an explicit bypass authorization.
      if (typeof bridge.requestPermission === "function") {
        return await bridge.requestPermission(next);
      } else if (typeof window.BloraWorkbench?.requestPermission === "function") {
        return await window.BloraWorkbench.requestPermission(next, bridge);
      } else {
        throw new Error("权限确认入口 requestPermission 尚未接入；未改变权限");
      }
    };
    const confirm = async (title, text) => {
      if (typeof window.BloraWorkbench?.confirm !== "function") throw new Error("风险确认入口尚未接入；未执行操作");
      return await window.BloraWorkbench.confirm(title, text, true) === true;
    };
    const render = async () => {
      if (typeof bridge.renderTranscript === "function") await bridge.renderTranscript();
      else if (state.sessionId) await bridge.selectSession(state.sessionId);
    };
    const booleanValue = (value) => {
      const token = value.toLowerCase();
      if (["on", "true", "1", "yes", "是", "开启"].includes(token)) return true;
      if (["off", "false", "0", "no", "否", "关闭"].includes(token)) return false;
      throw new Error("布尔设置使用 on 或 off");
    };
    const applySetting = async (key, value) => {
      if (["theme", "palette"].includes(key)) {
        const aliases = { 珊瑚: "coral", 靛蓝: "indigo", 石墨: "graphite", 单色: "mono", 电路: "circuit", 暮色: "dusk" };
        const palette = aliases[value] || value.toLowerCase();
        if (!["coral", "indigo", "graphite", "mono", "circuit", "dusk"].includes(palette)) throw new Error("可用主题：coral、indigo、graphite、mono、circuit、dusk");
        bridge.store("blora-theme", palette);
        document.documentElement.setAttribute("data-blora-theme", palette);
      } else if (["scheme", "color-scheme"].includes(key)) {
        const scheme = value.toLowerCase();
        if (!["auto", "dark", "light", "plain"].includes(scheme)) throw new Error("可用配色模式：auto、dark、light、plain");
        bridge.store("blora-color-scheme", scheme);
        const resolved = scheme === "auto" ? (window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light") : scheme === "plain" ? "light" : scheme;
        document.documentElement.setAttribute("data-blora-color-scheme", resolved);
        document.documentElement.style.colorScheme = resolved;
      } else if (["message-roles", "hide-tools", "tools", "worktree"].includes(key)) {
        const enabled = booleanValue(value);
        const preference = key === "tools" ? "hide-tools" : key;
        bridge.store(`blora-${preference}`, enabled ? "on" : "off");
        if (preference === "worktree") {
          const control = document.querySelector("#pref-worktree");
          if (control) {
            control.setAttribute("value", enabled ? "on" : "off");
            control.checked = enabled;
          }
        }
      } else throw new Error(`未知设置：${key}`);
      await render();
      notice(`${key}=${value}`, "设置");
    };
    try {
      if (name === "help") {
        await open("help", { query: args });
      } else if (name === "login" || name === "onboarding") {
        await bridge.enterOnboarding(name === "login" ? "account" : "welcome");
      } else if (name === "logout") {
        if (typeof bridge.refreshPassport === "function") {
          await post("/api/auth/logout");
          await bridge.refreshPassport();
          notice("已退出当前浏览器的 PassPort 登录；不会清除其他网关用户", "账号");
        } else {
          const button = document.querySelector("#passport-logout");
          if (!button) throw new Error("账号退出刷新入口尚未接入");
          button.click();
        }
      } else if (name === "quit") {
        notice("请关闭此标签页或窗口离开 Web 界面。关闭浏览器不会停止服务端或取消后台任务；停止运行请用 /cancel。", "浏览器适配");
      } else if (name === "web") {
        notice("当前已在 Web 界面。服务端可由 blora --web 或 blora web --bind 127.0.0.1:8787 启动；浏览器不能启动本地进程。", "浏览器适配");
      } else if (name === "keymap") {
        notice("浏览器保留自身快捷键，不能直接沿用终端 Ctrl+C/退出语义。输入 / 查看命令，点击建议补全；Tab、方向键、Enter 的命令选择由 app 绑定，窗口 Esc/焦点隔离由 Workbench 管理。", "快捷键适配");
      } else if (name === "yes" || name === "no") {
        const value = args.toLowerCase();
        if (name === "yes" && value && !["on", "true", "1", "off", "false", "0", "no"].includes(value)) throw new Error("用法：/yes [on|off]");
        const off = name === "no" || ["off", "false", "0", "no"].includes(value) || (!value && currentPermission() === "bypassPermissions");
        const next = off ? "default" : "bypassPermissions";
        if (await requestPermission(next) === true) bridge.applyPermission(next);
      } else if (name === "settings" && args) {
        const pair = args.match(/^([^=]+)=([\s\S]+)$/);
        if (!pair) throw new Error("用法：/settings key=value");
        const key = pair[1].trim().toLowerCase();
        const value = pair[2].trim();
        if (!value) throw new Error("设置值不能为空");
        if (["permission", "permission-mode"].includes(key)) {
          const next = permissionAliases[value.trim()] || value.trim();
          if (await requestPermission(next) === true) bridge.applyPermission(next);
        }
        else if (["auto-approve", "auto"].includes(key)) {
          const next = booleanValue(value) ? "bypassPermissions" : "default";
          if (await requestPermission(next) === true) bridge.applyPermission(next);
        } else if (["model", "provider"].includes(key)) {
          await setRouting(key, value.trim());
        } else await applySetting(key, value);
      } else if (name === "theme" && args) {
        await applySetting(["auto", "dark", "light", "plain"].includes(args.toLowerCase()) ? "scheme" : "theme", args);
      } else if (["model", "provider"].includes(name) && args) {
        await setRouting(name, args);
      } else if (name === "plan") {
        if (args) throw new Error("/plan 不接受参数");
        const next = currentPermission() === "plan" ? "default" : "plan";
        if (await requestPermission(next) === true) bridge.applyPermission(next);
        notice(next === "plan" ? "已切换到计划权限模式（只读）" : "已恢复默认权限模式", "权限模式");
      } else if (["new", "code", "work", "agent", "imagine"].includes(name) || (name === "mode" && args)) {
        if (state.running) throw new Error("请先停止当前运行，再创建或切换工作会话");
        const mode = name === "new" ? "code" : name === "mode" ? args : name;
        if (!modes.includes(mode)) throw new Error("模式应为 code、work、agent 或 imagine");
        if (args && name !== "mode") throw new Error(`/${name} 不接受参数`);
        const session = await post("/api/sessions", { mode, title: mode });
        bridge.applyRunMode(mode);
        await bridge.refreshSessions();
        await bridge.selectSession(session.id);
        window.location.hash = `#/session/${encodeURIComponent(session.id)}`;
      } else if (["goto", "next", "prev", "reload"].includes(name)) {
        if (name !== "reload" && state.running) throw new Error("运行中不能切换会话");
        await bridge.refreshSessions();
        if (name === "reload") notice("会话列表已刷新");
        else {
          const rows = await bridge.api("/api/sessions");
          let selected;
          if (name === "goto") {
            if (!args) throw new Error("用法：/goto <id|title>");
            const query = args.toLowerCase();
            selected = rows.find((row) => row.id === args) || rows.find((row) => `${row.id}\n${row.title || ""}`.toLowerCase().includes(query));
          } else {
            const index = rows.findIndex((row) => row.id === state.sessionId);
            selected = rows[Math.max(0, Math.min(rows.length - 1, index + (name === "next" ? 1 : -1)))];
          }
          if (!selected) throw new Error("没有匹配的会话");
          await bridge.selectSession(selected.id);
          window.location.hash = `#/session/${encodeURIComponent(selected.id)}`;
        }
      } else if (name === "id") {
        sessionId();
        notice(state.sessionId, "会话 ID");
      } else if (name === "status" || name === "agents" || name === "copy") {
        const session = await bridge.api(`/api/sessions/${sessionId()}`);
        if (name === "status") notice(`${session.id}\n模式：${session.mode}\n状态：${session.status}\n序号：${session.last_sequence}`, "会话状态");
        else if (name === "agents") notice((session.subagents || []).map((row) => `${row.id} ${row.role} ${row.status}\n${row.summary || ""}`).join("\n") || "暂无子代理", "子代理");
        else {
          const reply = (session.transcript || []).slice().reverse().find((row) => row.kind === "assistant");
          if (!reply) throw new Error("暂无助手回复");
          if (!window.navigator?.clipboard?.writeText) throw new Error("浏览器剪贴板不可用，请在安全上下文中使用或手动复制");
          await window.navigator.clipboard.writeText(reply.text);
          notice("已复制最后一条助手回复");
        }
      } else if (["task", "cron", "loop"].includes(name)) {
        sessionId();
        let prompt = args;
        let cron = null;
        let delaySeconds = 0;
        if (name === "cron") {
          const parts = args.split(/\s+/);
          if (parts.length < 6) throw new Error("用法：/cron <m h dom mon dow> <prompt>");
          cron = parts.slice(0, 5).join(" ");
          prompt = parts.slice(5).join(" ");
        } else if (name === "loop") {
          const match = args.match(/^(\d+)([smhd])\s+([\s\S]+)$/);
          if (!match) throw new Error("用法：/loop <30s|5m|1h|2d> <prompt>");
          delaySeconds = Number(match[1]) * { s: 1, m: 60, h: 3600, d: 86400 }[match[2]];
          if (!Number.isSafeInteger(delaySeconds) || delaySeconds <= 0 || delaySeconds > 2147483647) throw new Error("延迟须为正数且不超过 2147483647 秒");
          prompt = match[3].trim();
        }
        if (!prompt) throw new Error(`用法：/${name} ${command.hint}`);
        // Background tasks persist only a legacy auto-approve flag, not plan/acceptEdits.
        const bypass = currentPermission() === "bypassPermissions";
        if (bypass && await requestPermission("bypassPermissions") !== true) {
          notice("未创建任务：后台自动批准需要明确确认；取消不排队。", "后台权限");
        } else {
          const task = await post("/api/tasks", { session_id: state.sessionId, title: Array.from(prompt).slice(0, 40).join(""), prompt, delay_seconds: delaySeconds, cron, auto_approve: bypass, confirmed: bypass });
          notice(`已排队 ${task.id}；后台自动批准${bypass ? "开启" : "关闭"}，不继承 plan / acceptEdits`, "后台任务");
        }
      } else if (["pause", "unpause", "cancel-task"].includes(name)) {
        if (!args || /\s/.test(args)) throw new Error(`用法：/${name} <task-id>`);
        const action = { pause: "pause", unpause: "resume", "cancel-task": "cancel" }[name];
        await post(`/api/tasks/${encodeURIComponent(args)}/${action}`);
        notice(`任务 ${args} 已${{ pause: "暂停", resume: "恢复", cancel: "取消" }[action]}`);
      } else if (name === "pump") {
        const result = await post("/api/tasks/pump");
        notice(`已调度 ${(result.pumped || []).length} 个到期任务`);
      } else if (name === "diff" || name === "log" || name === "pwd") {
        const workspace = await bridge.api(`/api/workspace${state.sessionId ? `?session=${encodeURIComponent(state.sessionId)}` : ""}`);
        notice(name === "pwd" ? workspace.path : workspace[name === "diff" ? "git_diff" : "git_log"] || "暂无记录", command.about);
      } else if (name === "plugins" || name === "marketplace") {
        const result = await bridge.api(name === "plugins" ? "/api/plugins" : "/api/marketplace");
        notice(JSON.stringify(result, null, 2), command.about);
      } else if (["memory", "remember", "recall", "forget"].includes(name)) {
        const path = `/api/sessions/${sessionId()}/memory`;
        if (["remember", "forget"].includes(name) && state.running) throw new Error("运行中不能修改工作区记忆");
        if (name === "remember") {
          const pair = args.match(/^(\S+)\s+([\s\S]+)$/);
          if (!pair) throw new Error("用法：/remember <key> <value>");
          await post(path, { action: "remember", key: pair[1], value: pair[2] });
          notice(`已保存记忆 ${pair[1]}`);
        } else if (name === "forget") {
          if (!args) throw new Error("用法：/forget <key>");
          if (!await confirm("删除工作区记忆", `确认删除记忆 ${args}？此操作不可撤销。`)) return true;
          const result = await post(path, { action: "forget", key: args, confirmed: true });
          notice(result.forgotten ? `已删除记忆 ${args}` : `没有记忆 ${args}`);
        } else {
          const result = await bridge.api(path);
          const rows = (result.memories || []).filter((row) => name !== "recall" || !args || row.key === args);
          notice(rows.map((row) => `${row.key}=${row.value}`).join("\n") || "没有匹配的记忆", "工作区记忆");
        }
      } else if (["distill", "uninstall", "init", "users"].includes(name)) {
        const path = `/api/sessions/${sessionId()}/actions`;
        if (name !== "users" && state.running) throw new Error("运行中不能修改工作区");
        if (name === "uninstall" && !args) throw new Error("用法：/uninstall <name>");
        if (name !== "uninstall" && args) throw new Error(`/${name} 不接受参数`);
        if (name !== "users" && !await confirm(command.about, name === "uninstall" ? `确认卸载插件 ${args}？` : name === "distill" ? "确认从当前会话提炼并保存工作区记忆？" : "确认在当前工作区初始化 .blora/rules.md？")) return true;
        const result = await post(path, { action: name, ...(name === "uninstall" ? { name: args } : {}), confirmed: true });
        notice(typeof result.output === "string" ? result.output : JSON.stringify(result.output ?? result, null, 2), command.about);
      } else if (name === "checkpoint") {
        if (state.running) throw new Error("运行中不能写入手动检查点");
        const id = state.sessionId;
        await post(`/api/sessions/${sessionId()}/checkpoint`, { note: "manual" });
        if (bridge.getState()?.sessionId === id) await bridge.selectSession(id);
        notice("已写入恢复检查点");
      } else if (["skills", "mcp", "hooks"].includes(name)) {
        const result = await bridge.api(`/api/sessions/${sessionId()}/extensions`);
        notice(name === "skills" ? (result.skills || []).join("\n") || "暂无工作区技能" : `${name}：${result[`${name}_configured`] ? "已配置" : "未配置"}（不暴露服务端命令或目录）`, command.about);
      } else if (["doctor", "exec", "worktree"].includes(name)) {
        const result = await bridge.api(`/api/sessions/${sessionId()}/diagnostics`);
        notice(name === "doctor" ? JSON.stringify(result, null, 2) : `${name}=${result[name]}`, command.about);
      } else if (name === "find") {
        if (!args) throw new Error("用法：/find <query>");
        const rows = await bridge.api(`/api/sessions?q=${encodeURIComponent(args)}`);
        notice(rows.map((row) => `${row.id} ${row.mode} ${row.title || ""}`).join("\n") || "没有匹配的会话", "会话搜索");
      } else if (name === "reasoning") {
        const open = bridge.stored("blora-show-reasoning") !== "on";
        bridge.store("blora-show-reasoning", open ? "on" : "off");
        if (typeof bridge.renderTranscript === "function") bridge.renderTranscript();
        notice(open ? "已展开思考过程" : "已折叠思考过程");
      } else if (name === "tools") {
        const hidden = bridge.stored("blora-hide-tools") !== "on";
        bridge.store("blora-hide-tools", hidden ? "on" : "off");
        if (typeof bridge.renderTranscript === "function") bridge.renderTranscript();
        else if (state.sessionId) await bridge.selectSession(state.sessionId);
        notice(hidden ? "已隐藏工具摘要" : "已显示工具摘要");
      } else if (name === "search" || name === "clear") {
        if (typeof bridge.setSearch !== "function") await open("search", { query: name === "clear" ? "" : args });
        else bridge.setSearch(name === "clear" ? "" : args);
      } else if (dialogs[name]) {
        if (name === "harness" && state.running) throw new Error("运行中不能切换执行内核");
        await open(dialogs[name], { query: args, clear: name === "clear" });
      } else {
        notice(`/${name} 已收录；当前已确认的 Local API 尚未提供此操作，未执行。请使用 CLI，或在新增服务端协议明确后接入。`, "待接入功能");
      }
      return true;
    } catch (error) {
      bridge.showAlert(error instanceof Error ? error.message : String(error));
      return true;
    }
  }

  window.BloraCommands = Object.freeze({ catalog, resolve, matches, execute });
})();
