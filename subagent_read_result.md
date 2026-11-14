# 子代理（Subagent）功能视检记录

- 视检日期：2026-11-15
- 基线提交：`8a3eae1 feat(runtime): 完善子代理并行、取消、进度与自定义角色`
- 视检方式：只读代码走查 + 定向/全量测试，未改动任何源码
- 说明：以下行号对应该提交时的快照；后续改动可能使行号漂移

---

## 一、结论摘要

子代理不是独立进程，而是**同一进程内的递归 Run + scoped 线程**：

- 每次 `delegate` 创建一个独立**子会话**（`parent_session_id` 指向父会话），子会话有独立事件流、检查点与 token 统计。
- 子代理本质是在当前线程内同步调用 `Runtime::run(child_session, …)`；同一批多个 `delegate` 用 `std::thread::scope` 并行。
- 父会话只收到 `subagent.*` 摘要事件和最终 `<subagent>…</subagent>` 工具结果，不接收子会话完整事件流。
- 深度上限 2、同一父会话并发 3、每个子代理独立取消令牌；取消子代理不会取消父运行，父取消会传导到所有子代理。

---

## 二、功能地图（代码位置）

| 层 | 位置 | 内容 |
|---|---|---|
| 标识类型 | `crates/blora-types/src/id.rs:63` | `AgentId`，前缀 `agt_` |
| 取消语义 | `crates/blora-types/src/cancel.rs` | `CancelToken::child()`：子观察父，取消子不影响父 |
| 存储记录 | `crates/blora-storage/src/tasks.rs:39` | `AgentRecord`：id、父子会话、role、depth、status、budget_turns、summary、时间 |
| 表结构 | `crates/blora-storage/src/schema.sql:85` | `agents` 表；索引 `(parent_session_id, status)` |
| 事件 | `crates/blora-events/src/payload.rs:231-268` | `SubagentSpawned/Message/Completed/Failed` |
| 事件类型/actor | `crates/blora-events/src/payload.rs:533-536, 595-598, 640-643` | `subagent.*` 映射 `Actor::Subagent` |
| 投影 | `crates/blora-session/src/projection.rs:101, 594-690` | `SubagentView`、status/summary 合并、System 摘要 |
| 工具声明 | `crates/blora-tools/src/lib.rs:202-266` | `delegate`（可写）、`handoff`（只读） |
| 核心实现 | `crates/blora-runtime/src/agents.rs` | 角色解析、spawn、取消、插话、handoff、进度转发 |
| 调度 | `crates/blora-runtime/src/coordinator.rs:1451-1574, 1697-1708` | 批量并行、工具分发、`runtime_handled_tool` |
| API | `crates/blora-server/src/lib.rs:244-245, 1222-1258` | `/api/agents/{id}/steer|cancel` |
| 会话 JSON | `crates/blora-server/src/lib.rs:1835-1845` | `subagents[]`（id/role/status/summary/child_session_id） |
| TUI | `crates/blora-tui/src/lib.rs:5789-5832` | `/agents`、`/steer-agent`、`/cancel-agent` |
| Web UI | `web/app.js:1167-1231` | 子代理面板，运行中显示“插话/取消” |
| Web 命令 | `web/commands.js:71-73, 302-324` | `/agents`、`/steer-agent`、`/cancel-agent` |
| CLI | `crates/blora-cli/src/main.rs:663-671` | 仅 `session show` 打印子代理行 |
| 文档 | `docs/architecture/agent-loop.md:44` 等 | 已同步（见下节） |

已同步的文档：

- `docs/architecture/agent-loop.md:44`（子代理章节）
- `docs/architecture/overview.md:30`
- `docs/architecture/execution-and-permissions.md:21`
- `docs/architecture/modes.md:19, 32`
- `docs/operations/tui.md:151`
- `docs/operations/web.md:64-66`
- `docs/protocols/local-api.md:19-20`
- `docs/protocols/openapi.yaml:107-136`
- `README.md:20`

---

## 三、运行流程

### 3.1 工具分发

- `delegate` / `handoff` 属于 runtime 自行处理的工具，`runtime_handled_tool` 在 `coordinator.rs:2204-2215` 列出，注册表只提供 schema。
- `dispatch_batch`（`coordinator.rs:1451`）判断：
  - 整批都是 `delegate`、数量 >1、且未设置 `BLORA_HOOKS_DIR` → 并行分支（`:1473-1545`）；
  - 否则顺序执行，`dispatch_tool` 内 `delegate` 调 `spawn_subagent`（`:1697-1707`）。
- 并行的每个调用由 `scope.spawn` 起一个 OS 线程，结果按调用顺序填入 `precomputed`，再顺序落库，保证事件顺序确定。

### 3.2 `spawn_subagent`（`agents.rs:46-239`）

1. 校验 prompt 非空；`normalize_role` 校验角色名（字母数字 / `-` / `_`，≤64，非法回退 `worker`）。
2. `session_depth()` 沿 `sessions.parent_session_id` 上溯，深度超过 `MAX_SUBAGENT_DEPTH=2` 直接 Policy 错误。
3. 读取父会话，解析角色规范：
   - 内置：`research` / `review` / `plan` 只读（预算 6/6/4），`code` 可写（预算 8）；
   - 自定义：`.blora/agents/{role}.md` 或 `.blora/agents/{role}/AGENT.md`，再查 `$BLORA_HOME/agents/...`；frontmatter 支持 `readonly`、`budget`（1–40）；
   - 都没有 → 可写 `worker`，预算 `SUBAGENT_TURNS=6`。
4. 创建子会话：继承父 workspace；模式继承（Imagine 降级为 Code）；标题 `subagent:{role}`；`parent_session_id=父会话`。
5. `try_insert_running_agent(..., MAX_RUNNING_CHILDREN=3)` 在 store 互斥锁内“计数 + 插入”，避免并行 spawn 竞态；超限则归档子会话并返回 Policy 错误。
6. `cancel.child()` 创建独立取消令牌，登记到 `Runtime.subagent_cancels`；`SubagentCancelGuard` 在退出（含 panic）时清理令牌和 handoff 缓存。
7. 向**父会话**发 `subagent.spawned`（含 role、child_session_id、prompt、depth）。
8. 构造子 `RunOptions`：继承 model/mock/provider/interactive，`can_ask=false`，`worktree=false`，预算即 `max_turns`；只读角色强制 `PermissionMode::Plan`。
9. 生成子代理任务说明 `subagent_brief`（角色职责、预算、交付格式、能否再委派）。
10. 同步执行 `self.run(child_session, …)`（递归入口）。
11. 结束后读取子会话投影：取最后一条 assistant 文本；若子代理调用过 `handoff`，handoff 摘要优先；统计 token 与 turns。
12. 按结果更新 `agents` 行并发送事件：
    - 取消 → `cancelled` + `subagent.failed{error:"cancelled"}`；
    - 成功 → `completed` + `subagent.completed{summary,tokens,turns}`；
    - 失败 → `failed` + `subagent.failed{error}`。
13. 无论成功/失败/取消，都返回 **Ok 字符串** `<subagent role=... status=... turns=... tokens=...>…</subagent>`，父 run 不会被工具错误炸掉。

### 3.3 子代理运行期间

- 子会话每完成一条 assistant 消息，`coordinator.rs:1155` 调 `relay_subagent_progress`；`agents.rs:317-342` 找到 `agent_for_child_session`，若状态仍为 `running`，把前 400 字符作为 `subagent.message` 写到**父会话**，Web/TUI 可看到进度。
- `handoff`（`agents.rs:282-314`）把 `summary + files` 存入 `Runtime.subagent_handoffs`（按子会话键），返回“停止调用工具”的提示；父侧汇总时优先使用。
- 子代理内部还可以再 `delegate`（depth 2）；`subagent_brief` 在达到上限时提示“不要再派子代理”，但硬约束仍是 `spawn_subagent` 的深度检查。

### 3.4 取消与插话

- `cancel_subagent`（`agents.rs:242-266`）：只对 `running` 生效；取子令牌 `cancel()`；把 agents 行改 `cancelled`；发 `subagent.failed{cancelled}`。子运行在 provider 调用、重试退避、审批等待处检查令牌。
- `steer_subagent`（`agents.rs:270-278`）：要求 `running`，调用 `queue_steer(child_session)`；子 run 在下一轮边界把消息转成 `user.input`。
- 父取消：子令牌观察父令牌，父 run 取消会传导到子；取消单个子不会影响父。
- 清理：`SubagentCancelGuard::drop` 移除 `subagent_cancels` / `subagent_handoffs`。

### 3.5 对外入口

| 界面 | 行为 |
|---|---|
| 模型 | `delegate`、`handoff` |
| TUI | `/agents` 列出直接子代理；`/steer-agent <id> <msg>`；`/cancel-agent <id>` |
| Web | 子代理面板（运行中显示“插话/取消”按钮）；命令模块同上三命令 |
| Web API | `POST /api/agents/{id}/steer`、`POST /api/agents/{id}/cancel`；会话 JSON 带 `subagents[]` |
| CLI | 仅 `session show` 打印子代理行；无独立 agent 子命令 |
| 事件流 | SSE 按 `event.session_id` 过滤；子代理事件写在父会话，所以父页面能实时收到 |
| 持久化 | 事件是投影真相；`agents` 表是运行控制/查询副本；`subagent_cancels`、`subagent_handoffs` 仅在内存 |

---

## 四、数据模型与不变量

### 4.1 状态机

`agents.status` 取值：`running` → `completed` | `failed` | `cancelled`。

- 插入时固定 `running`（`agents.rs:84-101`）。
- 终态由 `update_agent` 写入（`agents.rs:181-183, 201-202, 222-223, 255-256`）。
- 投影侧状态来自事件（`projection.rs:594-690`），与表行分开维护；两者可能短暂不一致（见问题 5）。

### 4.2 事件载荷

| 事件 | 载荷字段 | 写入会话 |
|---|---|---|
| `subagent.spawned` | `agent_id`, `role`, `child_session_id?`, `prompt?`, `depth` | 父会话 |
| `subagent.message` | `agent_id`, `text`（≤400 字符） | 父会话 |
| `subagent.completed` | `agent_id`, `summary?`, `input_tokens`, `output_tokens`, `turns` | 父会话 |
| `subagent.failed` | `agent_id`, `error`（`cancelled` 表示取消） | 父会话 |

- 所有 `subagent.*` 事件默认 `Visibility::User`、`Actor::Subagent`。
- 未知事件类型不会破坏回放（`decode_payload` 返回 `None`）。

### 4.3 关键不变量（现状）

1. 深度 ≤ 2：由 `session_depth` + `spawn_subagent` 检查，绕过路径未知。
2. 直接子代理并发 ≤ 3：由 `try_insert_running_agent` 计数保证（仅统计 `status='running'`）。
3. 子代理不向用户提问：`can_ask=false`，`ask_user` 返回 `NO_USER` 文本。
4. 只读角色无法写：`read_only=true` + `PermissionMode::Plan` 双重限制（工具过滤 + `dispatch_tool` 拒绝）。
5. 取消子不影响父：令牌父子关系单向。
6. 父报告优先 handoff：`subagent_handoffs` 在汇总时优先于最后一条 assistant 文本。

---

## 五、测试验证结果

已运行（未改动代码）：

| 测试 | 结果 |
|---|---|
| `cargo test -p blora-runtime subagent` | ✅ 2/2（取消中断、handoff 优先） |
| `parallel_delegate_batch_runs_two_children` | ✅ |
| `agent_mode_spawns_code_child_with_usage_rollup` | ✅ |
| `subagent_handoff_is_the_parent_report` | ✅ |
| `blora-session` 全量 | ✅ 13/13 |
| `blora-types` 全量 | ✅ 4/4 |
| `blora-tools` 全量 | ✅ 7/7 |
| `blora-storage` 全量 | ✅ 17/17 |
| `blora-runtime` 全量 | ⚠️ 72/73，`shell_cwd_persists_across_calls` 失败 |
| `blora-server` 全量 | ⚠️ 14/20，6 个静态资源/workbench 测试失败 |
| `node scripts/web-commands.test.cjs` | ✅ 17/17（含 `/steer-agent`、`/cancel-agent`） |
| `node scripts/web-state.test.cjs` | ⚠️ 2/3，`window is not defined`（Node 环境问题） |

补充说明：

- `shell_cwd_persists_across_calls` 的失败与 `to_do.md` 中“待修复”记录一致，断言点是 shell cwd 投影，与子代理逻辑无直接关系。
- `blora-server` 的 6 个失败均为 CSS/静态资源与 workbench 路径类测试（`blora_css_imports_are_on_disk`、`serves_foundation_imports_and_addons`、`serves_nested_component_css`、`http_serves_css_and_sessions`、`workbench_session_workspace_and_actions_protocol`、`workbench::git_files_are_relative_to_nested_workspace`），与 `8a3eae1` 改动无直接关系，属于该快照的基线状态。
- 子代理相关测试均通过，说明主链路（spawn、并行、handoff、取消、投影）在 mock/单元层面是可靠的。

---

## 六、发现的问题（按严重度排序）

### 问题 1（高）：可写子代理的审批在 TUI/Web 会话内不可见，可能卡死整个父 run

**现象**

- 子 `RunOptions` 继承父 `interactive`（`agents.rs:130`）。父权限为默认 `Ask` 时，可写角色（`code` / 自定义 / `worker`）执行写文件或 shell 会触发 `ApprovalRequired`，进入 `await_approval` 阻塞（`coordinator.rs:1887-1895`）。
- 审批记录写入**子会话**，但：
  - TUI 右栏只查主会话和侧边对话（`right_panel.rs:395-413`）；
  - TUI `/approvals` 只查当前会话（`lib.rs:5544`）；
  - Web 会话内审批只查当前会话（`web/app.js:1373`）；
  - Web 全局审批页虽列全部（`web/app.js:1394`），但 `ensure_session` 在 `require_auth=true` 时会把无 owner 的子会话判为 forbidden（`server/src/lib.rs:1902-1920`），而子会话创建时不会继承 `user_id`（仅 fork 会话会设置，`server/src/lib.rs:1403`）。

**影响**

- 在 TUI 默认 `Ask` 权限下，父线程阻塞在 `spawn_subagent` 的 join，子代理永久等待审批，只能取消整个父 run 或使用 Web 全局审批页（且本地未开认证时）解除。
- 子代理面板显示 `running`，但用户没有入口批准，体验上是“卡死”。

**触发条件**

- TUI 或 Web；父会话权限为 `Ask` / `AutoEdit`（写入或变更命令需要审批）；使用可写角色且确实调用了写工具或非只读 shell。

### 问题 2（中）：崩溃/重启后 `running` 子代理行无人回收

**现象**

- `work.rs:111-143` 有 `recover_stale_tasks`，但 agents 表没有对应恢复逻辑。
- 进程退出时未完成的子代理会留下 `status='running'` 行；重启后：
  - 永久计入“并发 3”上限，阻塞新的 `delegate`；
  - 投影里一直显示 running；
  - 只能人工 `/cancel-agent`（`cancel_subagent` 即使没有内存令牌也会把行改为 cancelled）。
- 若子代理在崩溃前已产生终态 run，也不会补发 `subagent.failed/completed`。

**影响**

- 长时间使用后可能“无法再委派”，且需要人工清理。

### 问题 3（中）：父取消无法立即打断子代理里的前台 shell

**现象**

- `execute_shell`（`coordinator.rs:2059-2112`）不接收 `CancelToken`，前台 shell 默认超时上限 600 秒，PTY 无超时。
- 取消只能打断 provider 调用、重试 sleep、审批等待；若子代理正在执行长 shell，父线程仍要等 scoped thread join 完成。

**影响**

- `cancel_subagent` 的“立即中断”在这一场景下有边界；用户感知为取消后仍然卡住。

### 问题 4（低）：取消判定可能把已成功的子代理标成 cancelled

**现象**

- `agents.rs:147-148` 在 join 之后才用 `child_cancel.is_cancelled()` 判断。若父在子代理刚完成、尚未汇总时取消，子令牌因观察父令牌而变为 cancelled，成功结果会被报告为 `status="cancelled"`。

**影响**

- 结果状态误报；子代理实际产出被丢弃（summary 仍取 handoff/最后文本，但状态显示取消）。

### 问题 5（低）：并发上限失败路径的归档不写事件

**现象**

- `agents.rs:99` 直接调用 `store.archive_session`（纯 DB UPDATE，`sqlite.rs:750-760`），不产生 `session.archived` 事件。
- 投影只根据事件改状态（`projection.rs:336-340`），因此该子会话在会话列表中显示 archived，但通过会话 JSON/投影看仍是 active。

**影响**

- 同一会话在不同视图状态不一致；依赖事件的消费者（复刻、导出）看不到归档。

### 问题 6（低）：子会话作为普通会话出现在列表中

**现象**

- `list_sessions` 无 `parent_session_id` 过滤（`sqlite.rs:389-431`）；Web 直接渲染全部（`app.js:822-846`），TUI 侧栏只按 workspace 过滤（`view.rs:1132-1139`）。
- 完成后的子会话仍为 active，会持续占用会话列表，标题为 `subagent:{role}`。

**影响**

- 列表噪声；用户可能误入子会话并继续对话。也可能是有意保留以便查看上下文，但文档没有说明。

### 问题 7（低）：其他观察

- `/agents` 与 `list_agents` 只列**直接子代理**；根会话看不到孙子代理（depth 2）的进度，其 `subagent.*` 事件写在其直接父会话。
- `insert_agent`、`running_child_count` 目前未使用（storage 层死代码）。
- CLI 没有独立的子代理控制命令；能力仅在 TUI/Web/API。
- 混合批量（`delegate` + 其他工具）会退化为顺序执行；只有“整批全是 delegate”才并行，与文档一致。
- 自定义角色名校验可阻止路径穿越；frontmatter 只解析简单 `key: value`，未知键忽略；非法名回退 `worker`。
- `delegate` 声明为可写工具，因此不会进入只读工具的并发批次。

---

## 七、关键代码索引

```
crates/blora-types/src/id.rs                       AgentId (agt_)
crates/blora-types/src/cancel.rs                   CancelToken 父子语义
crates/blora-events/src/payload.rs                 subagent.* 载荷与类型映射
crates/blora-session/src/projection.rs             子代理投影与 System 摘要
crates/blora-storage/src/schema.sql                agents 表与索引
crates/blora-storage/src/tasks.rs                  AgentRecord
crates/blora-storage/src/sqlite.rs                 try_insert_running_agent / update_agent / list_agents
crates/blora-tools/src/lib.rs                      delegate / handoff 工具声明
crates/blora-runtime/src/agents.rs                 子代理核心实现
crates/blora-runtime/src/coordinator.rs            dispatch_batch / dispatch_tool / relay_subagent_progress
crates/blora-server/src/lib.rs                     /api/agents/* 与会话 JSON subagents
crates/blora-tui/src/lib.rs                        /agents /steer-agent /cancel-agent
crates/blora-tui/src/right_panel.rs                审批面板（主会话 + 侧边对话）
web/app.js                                         子代理面板与审批轮询
web/commands.js                                    Web 命令模块
```
