# Web 操作与 CLI 功能对照

## 交付范围与实现状态

`web/commands.js` 提供 `window.BloraCommands`，目录对齐 `crates/blora-tui/src/slash.rs` 的全部 79 个命令及别名，说明为中文。目录收录不等于功能已经实现。下面区分模块执行、复用原 app、依赖 Workbench 与尚未提供的功能；没有声称整体验收或浏览器测试通过。

命令模块与 app 的加载、执行桥接及以 `/` 开头的补全输入集成已完成；窗口与服务端由主任务集成。本次修订仅修改命令模块、本说明，并新增 `scripts/web-commands.test.cjs`，没有修改 Rust、UI 或 app。主任务负责浏览器验收，本子任务不把接口集成完成等同于浏览器测试通过。

## 集成方式

1. 已由主任务接入 `/commands.js` 的静态加载和命令桥接；窗口依赖 dialogs/workbench。
2. app 补全已对齐 `BloraCommands.matches(input)`，input 以 `/` 开头；每项为 `{name, aliases, hint, about, group}`。裸 `/` 保持 TUI 目录顺序；非空输入按名称或别名的精确、前缀、包含、子序列匹配排序，同分按名称长度和字典序排序。`resolve(name)` 返回规范目录项或 `null`，不用于模糊执行。
3. 原 `runSlash` 已接入 `await BloraCommands.execute(raw, bridge)`；返回 `true` 就结束，返回 `false` 继续原核心逻辑。执行失败显示 `showAlert` 并返回 `true`，不将失败的操作二次交给旧逻辑。
4. **旧核心必须先规范化别名**：例如 `/events`、`/interject` 应由 `resolve(token).name` 转成 `/timeline`、`/steer` 再进入旧分支。模块对复用命令返回 `false`，不会偷偷递归调用 `runSlash`。

bridge 必需函数：`api(path, options)`、`getState()`、`stored(key, fallback)`、`store(key, value)`、`applyPermission(mode)`、`applyRunMode(mode)`、`refreshSessions()`、`selectSession(id)`、`showNotice(text, variant, title)`、`showAlert(text)`、`enterOnboarding(step)`。`api` 接受 JSON 字符串 body，负责鉴权、状态检查和解析响应。

状态至少包含 `sessionId`、`running`；权限优先读取 `state.permission`，其次 `permissionMode` / `permission_mode`，否则使用持久化偏好。偏好值不能证明绕过权限已经获批。可选 `setSearch(query)` 用于本地记录筛选；不存在时要求 Workbench 的 search 窗口。

权限调用可选 `bridge.requestPermission(canonicalMode)`，否则调用 `window.BloraWorkbench.requestPermission(canonicalMode)`；须返回布尔值。确认 `true` 后才由模块调用 `applyPermission`，取消不改设置。不提供确认入口时拒绝改变权限，不退回旧 `/yes` 的直接绕过逻辑。支持 canonical `default / plan / acceptEdits / bypassPermissions`，兼容 `ask / auto-edit / yolo`。

窗口统一调用 `window.BloraWorkbench.open(kind, {bridge, args, query, ...})`。`/help query` 调用 help 窗口，由 UI 展示并筛选中文命令目录，不再用页面 Notice 堆叠目录。已读取的 kind 对应为 session、folder、mode、harness、model、provider、permission、theme、settings、context、tools、git、tasks、help。参数设置在命令模块直接应用，不依赖设置窗口消费 key/value。可选 `renderTranscript` 更新记录，缺省时重载当前会话；可选 `refreshPassport` 用于退出后的账号刷新，否则点击现有 `#passport-logout` 调用 UI 退出流程。

## 功能对照

下列行均包含对应目录的别名；别名清单以模块 catalog 为准。

| CLI 命令 | Web 入口与实际程度 | 服务端 / 依赖及限制 |
| --- | --- | --- |
| help | 打开 UI help 对话框，传入 query | UI 显示中文命令目录，无服务端请求 |
| login、onboarding | 模块打开账号 / 首次引导 | enterOnboarding；真实授权未验证 |
| logout | 模块退出当前浏览器账号 | 现有 passport-logout 点击流程，或 POST /api/auth/logout 后 refreshPassport；不是 CLI 清除全部本地用户 |
| keymap | 浏览器适配说明 | 键盘绑定仍由 app / dialogs 实现，不能照搬终端退出键 |
| theme | 无参委托窗口；参数直接保存并应用主题或配色 | 六种配色；auto/dark/light/plain。plain 在 Web 映射 light，不等同无 ANSI 终端模式 |
| settings | 无参委托窗口；参数直接执行 | permission/auto-approve 经确认；model/provider 走 routing；theme/palette、scheme/color-scheme、message-roles、hide-tools/tools、worktree 写 store 并应用/重绘；未知键报错 |
| harness | 委托 harness 窗口，运行中拒绝 | /api/harnesses 与会话 harness API；真实外部 CLI 未验证 |
| new、code、work、agent、imagine | 模块按模式新建并选择会话 | POST /api/sessions；不是修改现有会话模式 |
| mode | 参数按指定模式新建；无参委托窗口 | code / work / agent / imagine；窗口的现有会话模式切换是单独行为 |
| plan | 模块创建 mode=agent、title=plan，会话可提交 prompt | /api/sessions、/:id/run；不再把权限设为 plan。绕过权限确认后附 confirmed=true；运行/流式状态的浏览器验收由主任务执行 |
| sessions | 委托 session 窗口 | 窗口提供查询及选择；初始化参数需联调 |
| find | 模块按查询列出匹配会话 | GET /api/sessions?q=；服务端 search_sessions 搜索标题和记录 |
| goto、next、prev、reload | 模块刷新、检索与选择会话 | /api/sessions；next / prev 不循环；运行中拒绝切换 |
| status、id | 模块显示状态 / ID | /api/sessions/:id |
| context | 委托 context 窗口 | 新 /api/sessions/:id/context；工作台响应字段需联调 |
| compact、fork、archive、resume、export | 返回 false，复用原 app | 原会话 API；export 使用浏览器下载。compact note 不新增持久化语义 |
| cancel、steer | 返回 false，复用原 app | /:id/cancel、/:id/steer；别名必须在旧分支前规范化 |
| checkpoint | 模块写手动恢复检查点，运行中拒绝 | POST /api/sessions/:id/checkpoint，{"note":"manual"}；仅在仍为原会话时重载 |
| search、clear | 使用可选 setSearch 筛选 / 清除 | 缺少 setSearch 时需 search 窗口；不是远端全文搜索 |
| tools | 模块切换 blora-hide-tools 并重新渲染记录 | 可选 renderTranscript；不存在时重载当前会话，专用工具详情由 Workbench 提供 |
| copy | 模块复制最后助手回复 | 会话 transcript；需要安全上下文与浏览器剪贴板授权 |
| yes、no | 模块风险确认后设置权限；yes on/off 与无参切换 | 请求 canonical 权限；取消保持原值，不绕过确认 |
| permissions | 委托 permission 窗口 | 网络与隔离为服务端能力，不能用浏览器偏好改变 |
| approvals、timeline、artifacts、usage | 返回 false，复用原 app 页面 / 请求 | 原审批、事件、产物、用量 API |
| tasks | 委托 tasks 窗口 | 原 GET /api/tasks；页面仍需自行刷新 |
| task、cron、loop | 模块排队任务 | POST /api/tasks；session_id/title/prompt/delay_seconds/cron/auto_approve/confirmed；cron 五字段由服务端验证；loop 是单次延迟，不是周期任务 |
| pause、unpause、cancel-task、pump | 模块执行任务生命周期 | /api/tasks/:id/pause、resume、cancel 和 /api/tasks/pump；操作后窗口刷新需联调 |
| memory、recall | 模块列出 / 按键筛选会话工作区记忆 | GET /api/sessions/:id/memory 返回 memories；不是浏览器 localStorage 记忆 |
| remember、forget | 模块保存 / 删除，运行中拒绝 | 同路径 POST action/key/value；forget 先 Workbench.confirm，确认后附 confirmed=true，取消不删除；检查 forgotten 返回值 |
| distill | 风险确认后从会话提炼记忆 | POST /api/sessions/:id/actions，action=distill、confirmed=true，显示 output |
| plugins、marketplace | 模块显示 API 的 JSON 结果 | /api/plugins、/api/marketplace；列表展示未做专用样式 |
| install | 返回 false，复用原 app | POST /api/marketplace |
| uninstall | 风险确认后卸载指定插件 | actions 请求 action=uninstall、name、confirmed=true，显示 output |
| skills、mcp、hooks | 模块显示技能及配置标志 | GET /api/sessions/:id/extensions；不返回真实 MCP 命令 / Hook 路径，与 CLI 原始环境值不同 |
| agents | 模块展示会话子代理 | 会话 JSON subagents |
| git | 委托 Git 工作台 | 新会话 Git 协议由工作台实现；提交、选择文件、说明生成、push/pull、Git 图未在命令模块实现或验收 |
| diff、log | 模块展示原工作区 diff / log | GET /api/workspace?session=当前ID；无会话时使用默认工作区；需新服务端 session query 支持 |
| files | 委托 folder 窗口 | 工作台目录浏览协议与新服务端 /api/directories 尚需核对 |
| read | 返回 false，复用原 app | GET /api/workspace/file?path=；默认工作区只读 |
| pwd | 模块显示当前会话工作区，无会话时显示默认工作区 | GET /api/workspace?session=当前ID |
| doctor、exec、worktree | 模块显示诊断 / 特定字段 | GET /api/sessions/:id/diagnostics；服务端脱敏元数据，不读浏览器进程环境 |
| init、users | actions 操作并显示 output | init 写规则前风险确认；users 只读不弹危险确认；body 为 action、confirmed=true |
| web、quit | 模块给出浏览器适配说明 | 不启动进程、不关闭标签页、不停止服务或后台任务 |

## 权限与后台任务

后台任务协议只保存 auto_approve，不继承 plan / acceptEdits。普通权限下创建的后台任务 auto_approve=false。当前偏好为 bypassPermissions 时，每次排队须重新得到 requestPermission=true 才能发送 auto_approve=true、confirmed=true；取消或无确认入口不创建任务。不能只凭 localStorage 的旧值自动批准。

模式会话创建与权限是独立操作；`/plan prompt` 不隐式修改权限。当前为绕过权限时，提交 prompt 同样需要显式确认；取消后 plan 会话仍存在，但不发送 prompt。

## 协议与确认边界

已有会话 memory、checkpoint、routing、extensions、diagnostics 协议由服务端工作台实现；本轮新增命令按主任务约定调用 `POST /api/sessions/:id/actions`，body 为 `{action,name?,confirmed:true}`，显示响应 `output`。distill、uninstall、init 会写工作区，先调用 `BloraWorkbench.confirm(title,text,true)`；users 是只读操作，不显示危险确认。缺少 confirm 导出时写操作失败关闭，不发送请求。forget 使用同样的确认边界。

任务请求 auto_approve=true 必须同时 confirmed=true；/plan prompt 绕过权限运行也发送 confirmed=true。确认取消均不执行对应请求；/plan 已创建的会话仍保留。

## 验证记录与外部依赖限度

本子任务已运行 `node --test scripts/web-commands.test.cjs`：15 项独立 Node 单测通过，覆盖全部 79 项 CLI 目录、别名、hint/group/顺序、以 `/` 开头的模糊匹配、核心回退、帮助窗口、权限优先级与取消、任务确认字段、参数设置和重绘、退出刷新、记忆删除、actions 危险确认、缺少确认入口、运行中限制、plan prompt 及模式/路由/本地筛选。测试用 VM 与 Mock bridge，不代表真实 HTTP 或浏览器验证。

集成已完成，主任务负责运行桌面与手机浏览器验收、服务端构建与 API 回归；具体结果应以主任务报告为准。本子任务没有执行 Playwright、Rust 构建、真实 PassPort 授权、线上模型、外部 CLI 登录、真实远端 Git 推送。浏览器剪贴板仍受安全上下文与授权限制，外部内核继续由自身认证/模型配置/权限边界控制。
