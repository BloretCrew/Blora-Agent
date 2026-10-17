# Local API

`blora web` 提供本地 HTTP API。Web UI 只通过这些接口访问 Runtime。

```text
GET    /api/sessions          session summaries: id, title, workspace_path, mode, status, updated_at, last_sequence
GET    /api/sessions?q=       same summary shape; full transcript stays on GET /api/sessions/:id
POST   /api/sessions
GET    /api/sessions/:id
POST   /api/sessions/:id/run         {"prompt", "permission_mode": "plan|default|acceptEdits|bypassPermissions", ...}
GET    /api/sessions/:id/events
POST   /api/sessions/:id/fork
GET    /api/sessions/:id/export
POST   /api/sessions/:id/archive
POST   /api/sessions/:id/resume
POST   /api/sessions/:id/compact
POST   /api/sessions/:id/cancel
POST   /api/sessions/:id/steer        {"message": "..."} — delivered at the next turn boundary
POST   /api/sessions/:id/input        {"request_id": "...", "answer": "..." | null} — answer or dismiss the open ask_user question; answer is at most 16000 characters
GET    /api/workspace/list?path=&session=  one directory inside the session workspace: {"path", "entries"}; directory names end with /
GET    /api/tasks
POST   /api/tasks
POST   /api/tasks/:id/cancel
POST   /api/tasks/:id/pause
POST   /api/tasks/:id/resume
POST   /api/tasks/pump
GET    /api/approvals
POST   /api/approvals/:id/resolve
GET    /api/workspace
GET    /api/workspace/file?path=
GET    /api/artifacts
GET    /api/settings
GET    /api/usage
GET    /api/plugins
GET    /api/marketplace
POST   /api/marketplace
GET    /api/auth/me
GET    /api/auth/device
POST   /api/auth/device/poll        {"attempt_id": "…"}
POST   /api/auth/device/cancel      {"attempt_id": "…"}
POST   /api/auth/logout
GET    /ws
```

事件流使用 SSE。密钥不会出现在 `/api/settings` 中。

## 首次引导与设备授权

`GET /api/auth/device` 由用户主动登录时调用，返回 `attempt_id`、`user_code`、`verification_uri`、`expires_in`、`interval`，并设置绑定授权浏览器的 HttpOnly Cookie。内部设备凭据保留在服务器。一个浏览器的新授权会替换旧尝试，其他浏览器的尝试保持独立。

`POST /api/auth/device/poll` 执行一次令牌查询，返回 `status: pending | slow_down | authenticated`。等待状态包含下一次查询的 `interval` 秒数；成功包含 `authenticated`、`username`、`name` 并设置登录 Cookie。客户端按间隔调度，服务器同时限制查询频率和并发。拒绝、过期及无效尝试返回错误。

`POST /api/auth/device/cancel` 取消当前浏览器拥有的尝试。客户端离开账号步骤、重试或退出时应取消旧尝试，并忽略旧请求的响应。

登录 Cookie 使用随机会话标识，服务器负责映射到用户；退出立即撤销映射。当前映射保存在服务器内存中，重启 Web 服务后需要重新登录，用户令牌与引导完成记录继续保留。

`GET /api/auth/me` 返回账号信息及 `credentials_valid`，区分账号会话与 AI 凭据可用性。临近过期时尝试用已保存的 refresh token 刷新；无可用凭据时引导重新登录。OOBE 完成标记由终端或浏览器分别保存，网关 API 的访问权限继续由服务端认证决定。

## 工作台 API（前端接入协议）

以下新增接口都经过网关认证。网关模式必须带 `Authorization: Bearer <token>`；本地模式沿用现有 PassPort Cookie（未登录时是本地操作权限，不是远程多租户）。带 `Origin` 的 API/WS 请求必须与 HTTP Host 同源；不再允许跨源 CORS。成功返回 JSON；失败沿用纯文本错误：401 未认证，403 越权/越界，400 参数、运行状态或 Runtime 错误，Axum JSON 解码错误为 422。

所有 `:id` 接口校验会话归属和工作区：工作区必须是启动 `blora web` 指定目录本身或其子目录，解析真实路径后仍必须在该边界内。网关用户不能访问无归属或其他用户的会话。搜索、任务、审批、产物列表过滤不可访问会话；取消、SSE、派生、归档、恢复、导出、压缩及 WS 操作同样检查归属。派生会话继承当前用户归属。网关 `/api/usage` 必须指定 `?session=...`；全局任务 pump 仅本地模式可调用。

### 内核与会话

- `GET /api/harnesses` → `{harnesses:[{id,name,installed,native,provider_selection,imagine,permissions,authentication}]}`。`permissions` 是可请求的 canonical identifier 数组；外部认证为 `external-cli`，模型/配置/认证属于外部 CLI。
- `POST /api/sessions/:id/harness`，请求 `{harness:"blora|claude-code|codex|grok-build|hermes-agent|openclaw|opencode|qwen-code|deepseek-harness"}` → 完整 SessionJson。运行中或 CLI 未安装拒绝。
- `POST /api/sessions/:id/routing`，请求 `{provider:"供应商ID",model:"模型ID"}` → SessionJson。model 可省略默认空字符串；运行中及外部内核会话拒绝。
- SessionJson 新增 `run_status`（string/null）、`harness`、`provider`（string/null）、`model`（string/null）、`external_model`（string/null）、`external_sessions`（`[{harness,external_session_id}]`）。外部内核实际模型以 `external_model` 为准，不能把原生 model 显示为实际模型。`run_status` 严格来自 `projection.runs.last().status`，没有 run 时为 null，不使用会话 status 或临时取消标记代替。值为 `queued|running|waiting_approval|waiting_input|waiting_task|compacting|paused|completed|failed|cancelled`；三种完成态 completed/failed/cancelled 为 terminal，其他非 null 值均视为有未终结 run。
- `POST /api/sessions/:id/run` 实际调用 `Runtime::run_foreground`，按会话内核执行；外部 CLI 不走原生 provider factory。既有请求字段保持兼容，新增 `confirmed:boolean`。显式请求 `auto_approve:true` 或 `permission_mode:"bypassPermissions"`（含 yolo 别名）必须同时 `confirmed:true`。同一会话并发 run 拒绝，异常结束释放运行标记。
- `POST /api/sessions` 的 `workspace` 可为根目录内绝对路径或相对根目录的路径；默认 `.`；必须是存在的目录。项目切换通过创建/选择相应 workspace 的会话完成，不修改历史会话路径。

### 供应商

- `GET /api/providers` → `{providers:[...]}`。首项 PassPort 为 `{id:"blora",name:"Bloret PassPort",available:boolean,models:[{id,name}]}`；保存项为 `{id,name,base_url,format,models:[{id,name}],has_api_key:boolean}`，不含 api_key。
- `GET /api/providers/catalog` → `{providers:[{id,name,api,env,npm,models:[{id,name}]}]}`，复用 models.dev catalog/cache；网络失败返回 400，不伪造空成功。
- `GET /api/providers/:provider/models` → `{models:[{id,name}]}`。优先已保存模型，随后 catalog；PassPort 为内置模型。不会使用密钥对任意 URL 发起浏览器指定的探测请求。
- `POST /api/providers`，请求 `{id,name,api_key,base_url,format,models:[{id,name}]}`。format 为 `openai|anthropic|gemini|responses`；models 可省略。响应 `{provider:{id,name,base_url,format,models,has_api_key}}`。ID 只允许 ASCII 字母/数字/`-`/`_`，长度 1–100，不允许覆盖 blora；name/key 非空，key 最大 16384 字节。base_url 必须 HTTPS 且不能含凭据、query、fragment 或反斜杠。复用 CLI SavedProvider 存储格式，通过唯一临时文件、0600 权限（Unix）、fsync 和原子 rename 保存，不返回/记录 key。由于 CLI 供应商文件是进程用户全局配置，**网关模式禁止新增/覆盖供应商（403）**，防止一个远程用户修改其他用户的全局路由。

### 目录与项目

`GET /api/directories?path=.` 与 `GET /api/projects?path=.` 同协议：
`{root:"绝对根目录",path:"当前绝对目录",parent:"父目录或null",directories:[{name,path,project:boolean}],files:[{name,path,size}]}`。
files 为当前目录下普通文件列表（name 文件名、path 绝对路径、size 字节数），按 name 排序；目录和文件只返回真实路径在根内的项目；不返回外部符号链接目标。`project` 表示存在 `.git` 标记；目录浏览不会自动切换会话。拒绝 `..`、不存在路径、根外绝对路径。

### 会话工作区文件

`GET /api/workspace?session=:id` 和 `GET /api/workspace/file?session=:id&path=相对路径` 在读取前调用 ensure_session 和 session_workspace，以选定会话 workspace 为边界，不再固定读取服务启动目录。响应字段保持原样：snapshot 为 `{path,files,git_status,git_diff,git_log,git_branch,entries}`，file 为 `{path,contents}`。本地模式省略 session 仍读取启动根目录；网关模式省略 session 返回 400。越权会话返回 403；文件相对路径经 Runtime 边界检查。

`GET /api/workspace/list?path=相对路径&session=:id` 只列出这一层目录，供提问对话框里的文件夹和文件选择器使用。`path` 为空时按 `.` 处理。响应 `{path, entries}`。目录名以 `/` 结尾。拒绝绝对路径、`..` 和越出工作区的路径。

### Git

所有 Git 接口以会话 workspace 为上下文，复用 Runtime，序列化服务器内 Git 操作。必须是允许范围内独立 Git 仓库；拒绝根外父仓库、根外 `.git` 符号链接和 `.git` 文件形式的 linked worktree/submodule（避免访问范围外 Git 管理目录）。

| 方法/路径 | 请求 | 返回 |
|---|---|---|
| GET `/api/sessions/:id/git/status` | 无 | `{branch,added,removed,ahead,behind,stashes,clean}` |
| GET `/api/sessions/:id/git/files` | 无 | `{status:"Git status 原始文本",numstat:"原始文本",files:[{path,old_path,status}]}` |
| GET `/api/sessions/:id/git/diff` | 可选 `?file=相对文件路径` | `{diff:"完整 Runtime 差异文本"}` |
| GET `/api/sessions/:id/git/log` | 无 | `{log:"Runtime Git graph/log 原始文本"}` |
| POST `/api/sessions/:id/git/message` | `{provider?:string,model?:string}` | `{message:"生成的提交主题"}` |
| POST `/api/sessions/:id/git/commit` | `{confirmed:true,message:"主题",files:["相对文件路径"]}` | `{output:"Git 完整输出"}` |
| POST `/api/sessions/:id/git/sync` | `{confirmed:true,direction:"pull|push"}` | `{output:"Git 完整输出"}` |

files 使用 Git porcelain v1 `-z --untracked-files=all` 零字节解析，不从人类可读 status 中切割或解析 quoted path。结构化 `path` 是相对会话 workspace 的目标路径；rename/copy 的 `old_path` 是原路径，否则 null；`status` 为两个原始 XY 状态字符（包含空格，如 `"R "`、`" M"`、`"??"`）。中文、空格、引号、换行保留原值；非 UTF-8 文件名返回 400，不有损转码。子目录工作区会去掉仓库根前缀，跨出该工作区的 rename 返回 403。提交重命名时前端应同时选择 path 与 old_path。

message/commit/sync 在会话运行中拒绝。提交必须非空主题和非空文件列表；路径逐项拒绝绝对路径、`.`、`..`、NUL 及 `.git` 组件；Git 使用 literal pathspec 与 `--only`，未选文件不会加入本次提交。生成主题默认使用会话 provider/model，可携带当前用户有效 PassPort token；没有可用真实模型返回错误。同步必须有 upstream；pull 要求干净工作区且使用 `--ff-only`；push 仅推当前 HEAD 到 upstream，不强推。Runtime 错误输出在 400 纯文本 body 中。

### 记忆、上下文、检查点和扩展

- `GET /api/sessions/:id/memory` → `{memories:[{key,value,updated_at}]}`，按会话 workspace 读取 Runtime 记忆。
- `POST /api/sessions/:id/memory`：`{action:"remember",key,value}` → `{saved:true}`；`{action:"forget",key}` → `{forgotten:boolean}`。key 非空、最多 512 字节，value 最多 65536 字节；运行中拒绝。
- `POST /api/sessions/:id/checkpoint`：`{note?:string}` → SessionJson；默认 note 为 manual，复用 canonical checkpoint event，运行中拒绝。
- `GET /api/sessions/:id/context` → `{session_id,workspace_path,last_sequence,input_tokens,output_tokens,permission_mode,harness,provider,model,external_model,transcript,plan,plan_note}`。transcript 为 Runtime 原始 TranscriptItem 序列（包含完整工具记录），而非主会话接口的摘要文本；可用于上下文/搜索/工具详情。
- `GET /api/sessions/:id/extensions` → `{tools:[{name,description,parameters,read_only}],plugins:[{name,description,parameters}],skills:["文件名.md"],mcp_configured:boolean,hooks_configured:boolean}`。tools 为 `ToolRegistry::specs_for(session.mode)` 的真实内置工具目录，Imagine 仅提供 generate_image，其他模式不提供该工具；插件复用 Runtime 加载；不暴露可包含凭据的 command/MCP/Hook 环境值。skills 对齐 CLI 的 `.blora/skills/*.md` 列表。
- `GET /api/sessions/:id/diagnostics` → `{workspace_path,harness,provider,model,external_model,permission_mode,exec,worktree,network,pty,mcp_configured,hooks_configured,gateway}`。诊断不运行 shell，不返回密钥或完整 MCP/Hook 命令。
- 原有 `POST /api/tasks` 新增 `confirmed`；请求 `auto_approve:true` 时必须 `confirmed:true`，独立于会话权限。delay_seconds 验证转换和时间范围，溢出返回 400。

### 会话操作与工具详情

`POST /api/sessions/:id/actions` 请求 `{action:"distill|uninstall|init|users",name?:string,confirmed:true}`。所有动作要求 confirmed:true 和会话访问权限；distill/uninstall/init 为写操作，运行中返回 400。网关模式允许已授权会话的 distill，禁止 uninstall/init/users（403）。

| action | 参数 | 成功返回 |
|---|---|---|
| distill | 无 | `{action:"distill",stored:number,path:"绝对 memory.md 路径"}` |
| uninstall | `name` 必填，1–100 个 ASCII 字母/数字/`-`/`_` | `{action:"uninstall",name,uninstalled:true}`（插件不存在也成功） |
| init | 无 | `{action:"init",path:"绝对 rules.md 路径",created:boolean}`（已存在不覆盖，created:false） |
| users | 无 | `{action:"users",users:[{id,name,created_at}]}` |

写入/卸载前逐组件检查 `.blora/memory.md`、`.blora/plugins/:name.json`、`.blora/rules.md`，拒绝符号链接（包括悬空链接）和越界路径；distill 验证后复用 Runtime，init 使用 CLI 同样的规则初始内容并 create_new 防止覆盖。users 只投影 id/name/created_at，不返回 token/hash、PassPort token、refresh token、邮箱或其他认证字段。

`GET /api/sessions/:id/tool-details` → `{session_id,tools:[TranscriptItem::Tool]}`。tools 按原 transcript 顺序保留完整字段（Serde 默认外部标记格式 `{Tool:{name,status,event_id,arguments,output,call_id}}`，与 context transcript 的 Tool 项完全一致），未有工具时为空数组，不折叠、不截断。不执行任何工具；鉴权/会话/workspace 规则与 context 一致。

静态资源 `/dialogs.js`、`/workbench.js`、`/commands.js` 使用 Rust `include_str!` 嵌入，与原有 app.js 同样返回 JavaScript MIME。

### 当前边界

主要 API 已实现。已提供安全记忆 distill、插件卸载、规则初始化和用户安全列表；尚未新增会话级插件安装、MCP 工具动态枚举/Hook 执行 API。扩展接口提供按 mode 的内置工具、安全插件/技能清单与配置状态。既有 Marketplace 读取保持不变；全局插件安装在网关模式禁止，防止远程用户修改共享配置。外部 CLI 登录、真实模型调用、真实远端同步需单独实测；浏览器 UI 验收由前端任务负责。

### 共享工作区部署边界

网关身份校验隔离会话记录。一个服务实例的工作区是所有获授权用户共享的文件根目录，允许这些用户创建根内项目会话并操作文件和 Git；此部署用于受信任的同项目协作。需要文件系统级用户隔离时，请为每个用户运行独立服务实例并配置独立工作区及凭据。该网关不提供多租户文件系统隔离。

Git message 请求必须提供非空 `files`，生成仅读取所选文件的完整差异；未选文件不会进入模型请求。
