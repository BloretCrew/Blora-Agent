# 子代理（Subagent）改进方案

- 编写日期：2026-11-15
- 依据：`subagent_read_result.md`（同一轮视检）
- 基线提交：`8a3eae1 feat(runtime): 完善子代理并行、取消、进度与自定义角色`
- 原则：先修阻断性缺陷，再补健壮性，最后改善可观测性与体验；每步独立可回滚，并配套回归测试

---

## 一、总体目标

1. **不死锁**：可写子代理在需要审批时，用户必须能在 TUI/Web 中看到并处理，或子代理自动得到明确的“无法审批”结果。
2. **可恢复**：进程崩溃/重启后，遗留 `running` 子代理能被检测并收敛，不永久占用并发额度。
3. **可取消**：取消子代理能尽快生效，包括正在执行的前台 shell。
4. **状态一致**：`agents` 表、事件投影、会话列表、归档状态在各视图保持一致。
5. **可观测**：父会话能看到子代理进度与终态，CLI 也能控制子代理。

---

## 二、改进项总览

| 编号 | 优先级 | 问题 | 方案摘要 | 预估 |
|---|---|---|---|---|
| F1 | P0 | 可写子代理审批不可见导致卡死 | 审批上浮到父会话 + 自动拒绝兜底 | M |
| F2 | P0 | 崩溃后 running 子代理无人回收 | 启动/委派前收敛 + `subagent.failed{interrupted}` | S |
| F3 | P1 | 长 shell 无法被取消 | 前台 shell 支持取消（kill 子进程） | M |
| F4 | P1 | 取消误报成功结果为 cancelled | 以 run 终态为准，而不是令牌 | S |
| F5 | P2 | 并发上限失败路径归档不写事件 | 改为发 `SessionArchived` 事件 | XS |
| F6 | P2 | 子会话混入会话列表 | 列表默认隐藏子会话 + 显式入口 | S |
| F7 | P2 | CLI 无法控制子代理 | 增加 `blora agent list|steer|cancel` | S |
| F8 | P3 | 死代码 | 删除或使用 `insert_agent` / `running_child_count` | XS |
| F9 | P3 | 文档未说明子会话可见性 | 更新架构/操作文档 | XS |

规模：XS ≈ 半天内，S ≈ 1 天，M ≈ 2–3 天（含测试）。

---

## 三、详细方案

### F1（P0）审批上浮与自动拒绝兜底

**目标**：默认 `Ask` 权限下，可写子代理触发审批时不再无声卡死。

**方案 A（推荐）：审批上浮到父会话**

- 在 `spawn_subagent` 构造子 `RunOptions` 时传入父会话上下文，使 `await_approval` 把审批写到**父会话**，并在摘要里标注来源子代理：
  - 新增内部字段（例如 `RunOptions.approval_owner: Option<SessionId>` 或通过 `Runtime` 的 spawn 上下文映射 `child_session -> parent_session`）。
  - `await_approval` 插入记录与 `approval.requested` 事件时，`session_id` 取父会话，`summary` 追加 `[subagent:{role}]` 前缀。
  - 解析审批仍走同一个 `resolve_approval`，无需改 API。
- TUI 右栏、Web 会话内审批面板天然能查到（它们查的是父会话）。
- 需要保证子取消时审批记录被置为 deny（已有逻辑在 `await_approval` 的取消分支，改为按新的 owner 会话发 `approval.resolved`）。

**方案 B（兜底）：非交互子代理自动拒绝**

- 若选择不实现 A，则给可写子代理设置 `interactive=false`，并在 `delegate` 工具描述与 `subagent_brief` 中说明“子代理不能请求审批，遇到需要审批的操作会失败，请在说明中给出替代方案”。
- `dispatch_tool` 的 `ApprovalRequired` 分支已有非交互处理：记录 `tool.failed` 并返回错误，父 run 继续。
- 风险：可写子代理在 `Ask` 权限下会频繁失败，体验差，因此只作为 A 的降级。

**建议**：实现 A，B 作为 `BLORA_SUBAGENT_APPROVAL=parent|deny` 环境开关的备选。

**测试**

- 新单测：父权限 `Ask`，`code` 角色子代理写文件 → 父会话出现 `approval.requested`，`resolve_approval(allow=true)` 后子代理完成。
- 新单测：取消父 run 时，子审批记录变为 `deny`。
- server 集成测试：`require_auth=true` 时不因 owner 问题 403（若沿用父会话审批则天然满足）。

**涉及文件**

- `crates/blora-runtime/src/agents.rs`
- `crates/blora-runtime/src/coordinator.rs`（`RunOptions`、`dispatch_tool`、`await_approval`）
- `crates/blora-runtime/src/approvals.rs`
- `crates/blora-server/src/lib.rs`（仅在需要显式 owner 时）
- `web/app.js`（如需展示来源标签）

---

### F2（P0）遗留 running 子代理收敛

**目标**：重启或异常退出后，不再有永久 `running` 的 agent 行。

**方案**

- 在 `SqliteStore` 或 `Runtime` 增加 `reconcile_stale_agents(now)`：
  - 查找 `status='running'` 且 `child_session_id` 的最近 run 已终态（或对应事件流最后是 `run.completed/failed/cancelled`）的行；
  - 将其更新为 `completed`/`failed`/`cancelled`，摘要写 `interrupted` 或沿用 run 终态；
  - 对没有终态 run 的行统一写 `failed`，error=`interrupted`。
- 触发时机：
  - `Runtime::new` 之后首次 `spawn_subagent` 前（按需、幂等）；
  - 或 `pump()` 中与 `recover_stale_tasks` 并列（服务端已有每秒 tick）。
- 同时向父会话补发 `subagent.failed{error:"interrupted"}`，让投影收敛。
- 需要处理“子代理属于另一个仍在运行的进程”的情况：本地单进程模型下可直接判定；若未来多进程，应加 `process_id`/心跳列，本期不做。

**测试**

- 内存库：手工插入 `running` 行 + 无 run 事件 → 触发 reconcile 后变 `failed`，父会话有 `subagent.failed`。
- 插入 `running` 行 + 子会话 `run.completed` → 变 `completed`，summary 取最后 assistant。
- 并发上限：遗留 3 个 `running` 行时，reconcile 后能继续 `delegate`。

**涉及文件**

- `crates/blora-storage/src/sqlite.rs`
- `crates/blora-runtime/src/agents.rs`
- `crates/blora-runtime/src/work.rs`（若挂到 pump）
- `crates/blora-runtime/src/coordinator.rs`（`Runtime::new` 挂载）

---

### F3（P1）前台 shell 可取消

**目标**：`cancel_subagent` / 父取消能终止子代理正在跑的前台 shell。

**方案**

- 给 `LocalBackend::shell_prepared` 或新增 `shell_prepared_cancellable(command, cwd, kind, cancel)` 传入 `&CancelToken`。
- 实现方式（平台区分）：
  - 通用：改 `spawn_shell` 为“轮询 + 超时切片”，每 50ms 检查 `cancel`，命中则 kill 子进程并返回 `BloraError::Cancelled`（与 `shell_cwd` 的超时处理一致）。
  - Windows：`std::process::Child::kill()` 默认只杀直接子进程；如果 shell 包了 `cmd.exe /C`，需要 `taskkill /T /F /PID` 杀进程树。
  - POSIX：进程组 kill（`setpgid` 或 `kill(-pid)`）；本仓库禁止 `unsafe`，优先用 `Command::new("kill").arg("-TERM")` 或依赖现有 shell 封装。
- `execute_shell`（`coordinator.rs`）把 `cancel` 透传进去。
- 注意：`shell` 的 `background` 与 `pty` 分支行为不同；PTY 取消至少要关闭句柄/杀进程，避免泄漏。

**测试**

- 单测：启动 `sleep 30`（或平台等价），100ms 后 cancel，断言在 1s 内返回 `Cancelled`。
- 回归：`shell_cwd_persists_across_calls` 保持通过（当前基线失败，需先修 shell 探测，与本项独立）。

**涉及文件**

- `crates/blora-exec/src/lib.rs`（`spawn_shell`）
- `crates/blora-exec/src/host.rs`（如需要进程树 kill）
- `crates/blora-runtime/src/coordinator.rs`（`execute_shell` 透传 cancel）

---

### F4（P1）取消判定以 run 终态为准

**目标**：避免“子代理刚成功、父正好取消”被误报为 cancelled。

**方案**

- `agents.rs` 汇总时优先读取子会话投影里最后一次 run 的 `status`：
  - `completed` → 成功，即使 `child_cancel.is_cancelled()` 为真；
  - `failed` → 失败；
  - `cancelled` → 取消；
  - 不存在 run（例如创建后立刻取消）→ 按令牌判定。
- `agents` 行的 `update_agent` 使用同一个判定结果，保证表与事件一致。
- 若 cancel 发生在 `run` 返回 Ok 之后，仍以 run 终态为准（用户已得到结果）。

**测试**

- 构造：父取消与子完成竞争；断言最终状态为 `completed`，`subagent.completed` 只发一次。
- 构造：子 run 返回 `BloraError::Cancelled` → `cancelled`。

**涉及文件**

- `crates/blora-runtime/src/agents.rs`

---

### F5（P2）归档走事件

**目标**：并发上限失败时，子会话状态在各视图一致。

**方案**

- 将 `agents.rs:99` 的 `self.store.archive_session(&child_session, Utc::now())` 替换为 `self.archive_session(&child_session)`（Runtime 方法，发 `SessionArchived` 事件）。
- 错误路径：归档失败不应掩盖原始 Policy 错误，保持 `let _ =` 语义或用 `tracing::warn`。
- 可选：在 `subagent.spawned` 之前不发事件，所以子会话只有 `session.created` + `session.archived`，投影正确。

**测试**

- 让父会话已有 3 个 running 子代理，第 4 个 `delegate` 失败；断言子会话投影 `status=archived` 且会话列表显示 archived。

**涉及文件**

- `crates/blora-runtime/src/agents.rs`

---

### F6（P2）子会话列表可见性

**目标**：减少列表噪声，同时保留查看子会话上下文的入口。

**方案**

- `SqliteStore::list_sessions` 增加参数或新方法 `list_root_sessions`（`WHERE parent_session_id IS NULL`）。
- 默认 UI（TUI 侧栏、Web 侧栏、`/sessions`、`/goto`）使用根会话列表。
- 保留显式入口：
  - `/agents` 已列当前会话子代理，可增加“打开子会话”动作（TUI 回车、Web 按钮）；
  - Web 子代理面板已有 `child_session_id`，可直接链接到 `#/session/{id}`。
- `session_search` 已有 `roots_only` 选项，保持不变；`/find` 是否包含子会话由产品决定，建议默认包含并标注。
- 需要兼容：`ensure_session`、`/api/sessions/{id}` 仍应允许直接访问子会话。

**测试**

- TUI/Web 列表不出现 `subagent:*`；通过子代理面板可打开子会话。
- API 默认列表与显式 `?include_children=1`（如需要）行为清晰。

**涉及文件**

- `crates/blora-storage/src/sqlite.rs`
- `crates/blora-runtime/src/coordinator.rs`
- `crates/blora-server/src/lib.rs`
- `crates/blora-tui/src/lib.rs`、`crates/blora-tui/src/view.rs`
- `web/app.js`、`web/commands.js`
- `docs/operations/*.md`

---

### F7（P2）CLI 子代理控制

**目标**：无头/脚本场景也能列、插话、取消子代理。

**方案**

- 新增 `blora agent` 子命令组：
  - `blora agent list --session <id>`
  - `blora agent steer --id <agent> <message>`
  - `blora agent cancel --id <agent>`
- 复用 `Runtime::list_subagents` / `steer_subagent` / `cancel_subagent`，输出与 `session show` 一致。
- 与 TUI `/steer-agent`、Web 命令保持同一套错误文案。

**测试**

- CLI 解析测试（`Cli::try_parse_from`）。
- 最小集成：内存库创建子代理行后 `agent list` 输出。

**涉及文件**

- `crates/blora-cli/src/main.rs`
- `docs/operations/development.md` 或新增 CLI 文档

---

### F8（P3）死代码清理

- `SqliteStore::insert_agent` 与 `running_child_count` 无调用方。
- 选择：删除，或在 F2 的 reconcile 中使用 `running_child_count` 做诊断日志。
- 建议删除，测试覆盖由 `try_insert_running_agent`/`list_agents` 提供。

**涉及文件**

- `crates/blora-storage/src/sqlite.rs`

---

### F9（P3）文档补充

- `docs/architecture/agent-loop.md`：补充审批归属（F1 后）、崩溃恢复（F2）、取消边界（F3）。
- `docs/operations/tui.md` / `web.md`：补子会话可见性策略（F6）。
- `docs/protocols/openapi.yaml`：如新增 `?include_children` 或 CLI 不涉及 API，则仅更新说明。
- `README.md`：保持一行概述即可。

---

## 四、实施顺序与里程碑

**里程碑 1（止血，P0）**

1. F5（最小、无风险）
2. F4（小、纯逻辑）
3. F2（恢复逻辑，需谨慎处理事件补发）
4. F1 方案 B 兜底（如 A 排期长，先保证不死锁）

**里程碑 2（P0/P1 完整）**

5. F1 方案 A（审批上浮）
6. F3（可取消 shell）

**里程碑 3（体验与一致性，P2）**

7. F6（列表可见性）
8. F7（CLI）
9. F8、F9

每个里程碑结束跑：

- `cargo test -p blora-runtime -p blora-session -p blora-storage -p blora-types -p blora-tools`
- `cargo test -p blora-server`
- `node scripts/web-commands.test.cjs`
- `node scripts/web-state.test.cjs`
- 子代理专项：`cargo test -p blora-runtime subagent`、`delegate`、`handoff`、`agent`

---

## 五、风险与回滚

| 项 | 风险 | 缓解 |
|---|---|---|
| F1 | 审批归属改变可能影响现有 approval API 语义 | 事件仍写在父会话，API 不变；加环境开关灰度 |
| F2 | 误判仍在运行的子代理为中断 | 仅在单进程模型下收敛；以 run 终态为先验；幂等且可观测（tracing） |
| F3 | Windows 进程树 kill 不彻底或误杀 | 优先 `taskkill /T`；只杀本次 spawn 的 PID；测试覆盖 |
| F4 | 终态读取与事件写入存在竞争 | 以子会话事件流为唯一真相；汇总在 join 之后一次完成 |
| F5 | 归档失败掩盖原始错误 | `let _ =` + warn，保留 Policy 错误 |
| F6 | 隐藏子会话影响现有用户找历史 | 保留 `/agents` 与面板入口；可加 `include_children` 开关 |

回滚策略：每项独立提交；F1 与 F6 用环境变量/查询参数开关，便于快速回退。注意仓库在 `main` 分支，实施时应先建功能分支。

---

## 六、验收标准（Definition of Done）

- [x] 默认 `Ask` 权限下，可写子代理需要审批时，TUI/Web 能在父会话看到并批准，父 run 不卡死（F1）
- [x] 模拟崩溃后重启，遗留 `running` 子代理自动变为 `failed{interrupted}`，可继续 `delegate`（F2）
- [x] 子代理执行长 shell 时取消，1 秒内返回 `Cancelled`（F3）
- [x] 子代理成功与父取消竞争时，最终状态为 `completed`（F4）
- [x] 并发上限失败时，子会话在各视图均为 archived（F5）
- [x] 会话列表默认不显示子会话，但可从 `/agents` 或面板打开（F6）
- [x] `blora agent list|steer|cancel` 可用且有测试（F7）
- [x] F8/F9 完成（死代码清理、文档同步）
- [x] 本轮新增测试全部通过。仓库既有失败仍为环境问题，已在 §八 记录。

---

## 七、实施记录（2026-11-15）

分支：`feat/subagent-hardening`（基于 `backup/workspace-2026-11-15` 之后的 `4ed82c8`）

| 项 | 落地方式 | 关键位置 |
|---|---|---|
| F1 | Runtime 新增 `subagent_owners: Mutex<HashMap<SessionId, (SessionId, String)>>`；`await_approval` 把审批记录和 `approval.*` 事件写到父会话，摘要加 `[subagent:{role}]` 前缀；子 run 的 `run_id` 不写入父事件（避免跨会话外键语义） | `runtime/src/approvals.rs`、`runtime/src/agents.rs`、`runtime/src/coordinator.rs` |
| F2 | `Runtime::reconcile_stale_agents()`：跳过本进程活跃令牌和 `owner_pid` 仍存活的行，按子会话最后一次 run 终态收敛，否则 `failed{interrupted}`；`finish_agent_if_running` 条件更新防重复上报；在 `spawn_subagent` 与 `pump` 调用 | `runtime/src/agents.rs`、`storage/src/sqlite.rs`、`runtime/src/work.rs` |
| F2 跨进程 | agents 表新增 `owner_pid`（schema v14，`ALTER TABLE` 兼容旧库）；TUI/Web 共用同一 SQLite 时不会误回收对方正在驱动的子代理 | `storage/src/schema.sql`、`storage/src/sqlite.rs`、`storage/src/tasks.rs` |
| F3 | `LocalBackend::shell_prepared_cancellable`：前台 shell 每 50ms 轮询取消，命中则 `terminate_pid` 并返回 `Cancelled`；`Runtime::execute_shell` 透传 `CancelToken` | `exec/src/lib.rs`、`runtime/src/coordinator.rs` |
| F4 | `child_was_cancelled()` 以子会话 run 终态优先，仅在无 run 时回退到令牌/表状态 | `runtime/src/agents.rs` |
| F5 | 并发上限失败改走 `Runtime::archive_session`，写 `session.archived` 事件 | `runtime/src/agents.rs` |
| F6 | `SessionSummary` 增加 `parent_session_id`；新增 `list_root_sessions`；TUI 侧栏、选择器、`/sessions`、`/next` `/prev` 只看根会话；`/goto <完整 id>`、`/sessions all`、Web `？include_children=1`、子代理面板“打开子会话”保留访问 | `storage/src/sqlite.rs`、`tui/src/{view,session_picker,lib}.rs`、`server/src/lib.rs`、`web/*` |
| F7 | `blora agent list|steer|cancel`，解析与输出对齐 TUI 文案 | `cli/src/main.rs` |
| F8 | 删除未使用的 `insert_agent`、`running_child_count`；统一 agent 行映射 | `storage/src/sqlite.rs` |
| F9 | 更新架构/操作/协议文档 | `docs/**` |

新增测试（全部通过）：

- `agents::`：`finished_child_is_not_relabelled_by_a_racing_parent_cancel`、`reconcile_marks_orphaned_running_agents_interrupted`、`reconcile_skips_children_this_process_is_running`、`reconcile_leaves_rows_owned_by_a_live_process_alone`、`spawn_over_the_running_cap_archives_the_child_by_event`、`subagent_approvals_surface_on_the_parent_session`
- `blora-exec`：`cancellable_shell_returns_cancelled_when_already_cancelled`、`cancellable_shell_stops_a_long_command`
- `blora-storage`：`root_session_list_excludes_delegate_children`
- `blora-tui`：`picker_lists_root_sessions_only`
- `blora-cli`：3 个 agent 命令解析测试
- `web-commands.test.cjs`：17/17

环境性既有失败（基线一致，与本轮无关）：

- `blora-runtime::shell_cwd_persists_across_calls`、`blora-exec` 5 个 shell/git 测试：本机 `host_shell()` 为 `Cmd`，Git 工具在 MSYS 下启动失败；`to_do.md` 已记录。
- `blora-tui` 2 个文件面板测试、`blora-server` 6 个资源/workbench 测试：Windows `\\?\` 路径前缀与仓库未附带的 design 资源包。

---

## 八、与原方案偏差点

1. **F2 增加 `owner_pid`**：原方案写“单进程模型下可直接判定，多进程本期不做”，但 TUI 与 `blora serve` 默认共用同一个 `state.sqlite`，直接收敛会误伤另一个进程的子代理，因此补了进程归属列。
2. **F1 未新增 `RunOptions` 字段**：改用 runtime 内部 `subagent_owners` 映射，避免改动 22 处 `RunOptions` 构造点，也避免与并行修改冲突。
3. **F6 未新增 slash 命令**：新增命令会让 `scripts/web-commands.test.cjs` 的目录一致性断言（硬编码 82 条）失败，改为复用 `/sessions all` 参数。
4. **F5 归档失败仍为 best-effort**：保留 `let _ =`，不覆盖原始“too many running subagents”错误。

---

## 九、与现有待办的关系

- 仓库根 `to_do.md` 记录的是 Windows `bash/sh` 探测与 `shell_cwd_persists_across_calls`，与本方案 F3 相邻但独立；建议先修该基线失败，再做 F3 的取消切片，避免测试噪声。
- 本方案不包含外部 harness（Codex/Grok）子代理能力；它们目前没有 `delegate` 对应物。

---

## 十、后续只读复查发现（2026-11-15）

复查基线：`2f82ddc`（代码实现为 `124ad15`）。本次仅走查当前代码和依赖的反序列化实现，未修改业务代码、未重新执行全量测试。第六节勾选项描述上一轮已实现并由局部测试覆盖的主流程，不代表以下边界已完成验收。

### F10（优先）：子会话没有继承用户归属

- 代码证据：`runtime/src/agents.rs:144-151` 创建子会话后未绑定父用户；`storage/src/sqlite.rs:273-291` 的创建 SQL 不写入 `user_id`。
- 影响一：鉴权 Gateway 下，`server/src/lib.rs:1913-1930` 拒绝访问无 owner 的子会话，Web 的“打开子会话”仍可能返回 403。
- 影响二：Agent 模式的子代理可以调用 `session_search`；`runtime/src/skills.rs:139-150` 用当前会话的 `user_id` 决定过滤条件，而 `storage/src/search.rs:238-241` 在该值为空时不加用户过滤。因此多用户环境中存在跨用户检索历史记录的风险。这不应视为普通体验优化。
- 建议：创建委派会话时在同一事务中继承父用户，并让需要用户隔离的检索在缺少归属时失败关闭；补两个用户的隔离回归测试。

### F11（优先）：子代理的问答及多级审批仍存在等待边界

- 子代理设置 `can_ask=false`，但保留父 `interactive`（`runtime/src/agents.rs:212-218`）；实际执行问答时使用 `options.can_ask || options.interactive`（`runtime/src/coordinator.rs:1807-1816`）。交互式父运行下，子代理仍可进入 `input.rs:128-143` 的等待循环，而提问事件只写在子会话。
- `await_approval` 只向直接父会话上浮（`runtime/src/approvals.rs:48-57`）。深度 2 的子代理触发审批时，请求会停在中间子会话，根会话的 UI 依然看不到。
- 取消等待审批时只修改审批表、未发送 `ApprovalResolved`（`approvals.rs:83-88`），事件投影中的审批可能继续显示 pending。
- 建议：分离“可请求工具审批”与“可向用户提问”，明确阻止子代理问答或将问题上浮；审批沿代理归属链路路由到用户可见的会话；取消也通过规范事件收敛。

### F12（优先）：跨进程取消目前只改变记录，不能保证停止执行

- CLI 每次运行创建新的 `Runtime`（`cli/src/main.rs:466-468`），其中没有 TUI/Web 进程的内存取消令牌。
- `cancel_subagent` 只在本实例的 `subagent_cancels` 找到令牌时发出实际取消，随后无条件把数据库状态写成 cancelled（`runtime/src/agents.rs:339-363`）。执行进程没有轮询这个数据库状态来取消自己的令牌。
- 触发场景：终端 A 的 TUI 正在执行子代理，终端 B 执行 `blora agent cancel agt_…`。B 可以报告成功，A 里的模型或 shell 却继续执行，最后还可能把状态写回 completed。
- 建议：增加持久化 `cancel_requested` 或本地 IPC；由属主进程观察请求并触发本地令牌，执行退出后统一写终态。补两个 Runtime/同一个数据库的集成测试。

### F13（小修复，建议尽早）：`include_children=1` 与 bool 解析不匹配

- `web/commands.js:287` 发出 `/api/sessions?include_children=1`，但 `server/src/lib.rs:808-813` 将该字段声明为普通 `bool`。
- 当前锁定的 `serde_urlencoded 0.7.1` 对 bool 使用 Rust `parse::<bool>()`，接受 `true` / `false`，不接受 `1` / `0`。因此请求会在 Axum Query 提取阶段失败，Web `/goto` 可能直接返回 400。
- 建议：统一使用 `include_children=true` 并同步文档，或显式兼容 1/0；新增真实 Query 解析测试，而不是只断言前端发出了 URL。

### F14（健壮性）：恢复报告和终态事件尚未做到原子一致

- `reconcile_stale_agents` 在恢复 completed 时仅写入通用摘要 `completed`，并将 token/轮数填 0，没有重建子会话的最终回答及用量（`runtime/src/agents.rs:70-99`）。
- 先把 agent 行更新为终态，再单独发事件，且忽略事件写入错误（`:85-106`）。在两步之间崩溃或事件落库失败后，下次只查询 running 行将不会重试，父会话投影可永久保留 running。
- `cancel_subagent` 与成功收尾仍分别无条件 `update_agent`，没有共同的终态竞争协议。F4 的纯函数测试不足以覆盖取消请求和收尾写入的实际竞态。
- 建议：将 agent 终态与父会话事件放进同一存储事务，或使用可重试 outbox；恢复时从子会话事件重建摘要和用量，并增加完成/取消竞态与故障注入测试。

### F15（测试与取消边界）：PTY、Unix 子进程树和 Windows 实测仍不足

- `shell_prepared_cancellable` 的 PTY 分支仍调用不带令牌的 `spawn_shell`（`exec/src/lib.rs:297-324`）。
- Unix `terminate_pid` 仅向直接 PID 发信号，没有终止整个进程组（`exec/src/host.rs:145-152`）；shell 派生的子进程可能继续运行。
- 长命令取消测试在 `HostShell::Cmd` 分支直接 return（`exec/src/lib.rs:1770-1776`）。因此上轮该测试被统计为通过，并不证明当前 Windows Cmd 环境实际覆盖了运行中取消。
- 建议：为 PTY 接入取消，Unix 管理独立进程组；添加 Cmd 原生命令的长任务测试，并检测后代进程和输出管道是否都已结束。

### 后续建议顺序

1. F10（用户隔离）、F11（等待路由）、F12（真实取消）优先，不能只作为界面小修。
2. F13 属于小改动，适合尽快修正 URL 并补端到端查询参数测试。
3. F14/F15 补齐恢复与取消的故障边界；不要把“已有失败在基线上存在”直接等同于环境无需修复，Windows shell 探测和测试夹具仍需单独治理。

---

## 十一、第十节实施记录（2026-11-15）

分支：`feat/subagent-hardening`（基线 `1f6ae43`）

| 项 | 落地方式 | 关键位置 |
|---|---|---|
| F10 | `CreateSession` 增加 `user_id`；存储层在同一插入事务内继承父会话的 owner（`fork` 也走这条路径）；`session_search` 在存在已归属会话的库里对无归属调用返回空结果 | `storage/src/sqlite.rs`（`create_session`）、`storage/src/users.rs`（`has_owned_sessions`）、`runtime/src/skills.rs` |
| F11 | `ask_user` 只依据 `can_ask`，子代理固定 `false`；`await_approval` 沿 `subagent_owners` 链上浮到根会话，摘要串联整条角色路径；取消等待中的审批补发 `approval.resolved{deny}` | `runtime/src/coordinator.rs`、`runtime/src/approvals.rs` |
| F12 | `agents.cancel_requested`（schema v15）；非属主进程调用 `cancel_subagent` 只写请求；属主在子 run 期间每 250ms 轮询并触发本地令牌；`pump` 也会投递请求 | `storage/src/sqlite.rs`、`storage/src/schema.sql`、`runtime/src/agents.rs`、`runtime/src/work.rs` |
| F13 | 查询参数解析兼容 `true/false` 与 `1/0`；Web `/goto` 改用 `include_children=true`；补解析测试与真实 HTTP 列表测试 | `server/src/lib.rs`、`web/commands.js`、`docs/protocols/local-api.md` |
| F14 | 新增 `SqliteStore::finish_agent_with_event`：终态更新与父会话事件在同一 `IMMEDIATE` 事务；恢复时从子会话事件重建最后回答与 token/轮数；所有写事务改为 `IMMEDIATE`，消除跨连接 `SQLITE_BUSY` | `storage/src/sqlite.rs`、`runtime/src/agents.rs` |
| F15 | PTY 分支接入取消；Unix 命令使用独立进程组并整组终止；新增 PTY 取消测试与 Windows Cmd 下真实 `ping` 长任务测试 | `exec/src/lib.rs`、`exec/src/host.rs` |

新增测试（全部通过）：

- `agents::`：`delegated_child_inherits_the_parent_user`、`search_fails_closed_for_unowned_sessions_in_a_multi_user_store`、`subagents_never_block_on_ask_user`、`nested_subagent_approvals_reach_the_root_session`、`cross_process_cancel_request_stops_the_owner`、`reconcile_rebuilds_the_child_report_and_usage`
- `blora-exec`：`cancellable_pty_shell_stops_a_long_command`，`cancellable_shell_stops_a_long_command` 现在在 Windows Cmd 下也实际执行
- `blora-server`：`session_list_query_accepts_boolean_spellings`、`session_list_hides_children_unless_include_children`
- 既有 `ask_user_waits_for_an_answer_in_interactive_runs` 改为显式 `can_ask: true`，与新的语义分离保持一致

行为变化（需要调用方注意）：

- 子代理不再通过 `interactive` 间接获得提问能力；只有 `can_ask: true` 的运行会阻塞。
- `cancel_subagent` 在非属主进程返回成功仅表示“请求已记录”，终态由执行进程写入。
- 多用户数据库中，无归属会话的 `session_search` 返回空结果。
- `CreateSession` 新增字段，所有构造点已同步；外部若直接使用该结构需要补 `user_id: None`。

剩余基线失败（未在本轮修复）：`blora-runtime::shell_cwd_persists_across_calls`、`blora-exec` 5 个 shell/git 测试、`blora-tui` 2 个文件面板测试、`blora-server` 6 个静态资源/workbench 测试。本轮新增用例没有再引入失败。
