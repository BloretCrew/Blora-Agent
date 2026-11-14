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

- [ ] 默认 `Ask` 权限下，可写子代理需要审批时，TUI/Web 能在父会话看到并批准，父 run 不卡死（F1）
- [ ] 模拟崩溃后重启，遗留 `running` 子代理自动变为 `failed{interrupted}`，可继续 `delegate`（F2）
- [ ] 子代理执行长 shell 时取消，1 秒内返回 `Cancelled`（F3）
- [ ] 子代理成功与父取消竞争时，最终状态为 `completed`（F4）
- [ ] 并发上限失败时，子会话在各视图均为 archived（F5）
- [ ] 会话列表默认不显示子会话，但可从 `/agents` 或面板打开（F6）
- [ ] `blora agent list|steer|cancel` 可用且有测试（F7）
- [ ] 全量测试通过（含当前基线失败的 `shell_cwd_persists_across_calls` 与 server 静态资源测试，若这些不属于本次范围需单独记录）
- [ ] 文档与代码一致（F9）

---

## 七、与现有待办的关系

- 仓库根 `to_do.md` 记录的是 Windows `bash/sh` 探测与 `shell_cwd_persists_across_calls`，与本方案 F3 相邻但独立；建议先修该基线失败，再做 F3 的取消切片，避免测试噪声。
- 本方案不包含外部 harness（Codex/Grok）子代理能力；它们目前没有 `delegate` 对应物。
