# Agent Loop

Run 状态：`queued`、`running`、`waiting_approval`、`waiting_input`、`waiting_task`、`compacting`、`paused`、`completed`、`failed`、`cancelled`。后三者为终态。

每一轮：

1. 检查取消与墙钟预算（`BLORA_MAX_WALL_SECS`，默认 900）。
2. 把排队的 steer 消息写成 `user.input`（`/steer`、`POST /api/sessions/:id/steer`）。
3. 编译消息；估算 token 达阈值则先压缩。
4. 带重试与回退地调用 Provider，流式写入 `assistant.delta`。
5. 无工具调用则 `run.completed`，写 checkpoint，触发 `stop` hook。若 `finish_reason` 是输出上限（`length` / `max_tokens`），先自动续写，最多 2 次；续写提示只出现在请求里，不写入事件日志。
6. 有工具调用则执行整批：只读工具批次并发执行，结果按调用顺序落库；同一批调用连续重复 3 次注入提醒，6 次终止。工具参数不是合法 JSON 对象时不执行，返回解析错误与原始片段，让模型自行纠正。

默认回合上限为 40（`RunOptions::default()`、CLI `--max-turns`）。用完时内核再发一次不带工具的收尾请求，让模型说明已完成、未完成和下一步，然后记 `run.failed{maximum model turns reached}`。

Provider 的推理流（reasoning）在每次调用结束时汇总为一条 `assistant.reasoning` 事件（单次最多 32k 字符），只用于展示与审计，不会回放给模型。TUI 与 Web 默认折叠显示，`/reasoning` 切换展开。

`ask_user` 写 `input.requested` 并让 Run 进入 `waiting_input`，回答写 `input.provided` 后恢复 `running`。`RunOptions::can_ask` 表示有人在场（TUI、Web）；无人值守的运行直接得到“请自行判断”的工具结果，不会阻塞。模式与工具集的划分见 [modes.md](modes.md)。

事件序列（mock）：

```text
run.created
run.started
user.input
context.snapshot.created
model.requested
assistant.delta*
assistant.message.completed
usage.recorded
model.response.completed
run.completed
checkpoint.created
```

取消在非终态发出 `run.cancel_requested`，然后 `run.cancelled`。重复取消不得破坏已终态 Run。流中断时已输出的文本以 `[stream interrupted]` 结尾落库，不会重复回放。

## Hooks

`BLORA_HOOKS_DIR/<event>` 可执行文件，stdin 收 `{"hook","payload"}` JSON，stdout 返回 JSON，退出码 0 放行、2 阻断。事件：`session-start`、`user-prompt-submit`、`tool-before`、`tool-after`、`pre-compact`、`stop`。`decision` 取 `allow` / `block` / `ask`；`ask` 会走交互审批。`additional_context` 追加到提示或工具结果。超时 10 秒或报错时放行并记录 `hook.completed{degraded:true}`。

## 子代理

`delegate` 创建子会话，深度上限 2、同一父会话并发 3。同一批多个 `delegate` 用 scoped thread 并行执行，结果仍按调用顺序落库。每个子代理有独立的 `CancelToken`（观察父 token，取消子代理不会取消父运行）。子代理收到按角色生成的任务说明：内置 research / review / plan 只读、code 可写；其它角色名读取 `.blora/agents/{role}.md`（可含 `readonly` / `budget` frontmatter），否则按可写 worker 处理。说明里写明交付格式：结论在前，附证据路径与命令、改动文件和未决问题。`subagent.spawned` 记录 `depth`，子会话的助手增量会写成父会话 `subagent.message`，`subagent.completed` 回写 token 与轮数。结果以 `<subagent>` 包裹返回父会话；子代理失败或取消也作为工具结果返回，不炸掉父 run。子代理调用 `handoff` 时，该摘要优先于最后一条助手文本。`steer_subagent` / `cancel_subagent`（TUI `/steer-agent` `/cancel-agent`，CLI `blora agent list|steer|cancel`，API `POST /api/agents/:id/steer|cancel`）可在运行中干预。

可写子代理在 `ask` / `acceptEdits` 权限下触发的审批记录在**根会话**上（沿 delegate 链上浮，摘要带 `[subagent:worker > subagent:code]` 形式的前缀），TUI 右栏、`/approvals`、Web 审批面板都能看到并处理；取消等待中的审批会补发 `approval.resolved{deny}`，投影不会停留在 pending。子代理内的前台 `shell` 与 PTY 会轮询取消令牌，取消后终止进程（Unix 终止整个进程组）并返回 `Cancelled`。

子代理不向用户提问：`ask_user` 只看 `can_ask`，子代理固定为 `false`，直接得到“自行判断”的工具结果，不会阻塞父 run。子会话创建时在同一事务内继承父会话的 `user_id`；多用户数据库中，没有归属的会话调用 `session_search` 会返回空结果而不是跨用户检索。

跨进程取消：`cancel_subagent` 在本进程找不到令牌时写入 `agents.cancel_requested`，拥有该子代理的进程（TUI/Web）在运行期间轮询该标记并取消本地令牌；终态由实际执行方写入。`agents` 的终态更新与父会话事件在同一个 `IMMEDIATE` 事务中提交，崩溃恢复时从子会话事件重建摘要与用量。

进程异常退出后，遗留的 `running` 子代理行会在下次 `delegate` 或 `pump` 时收敛（跳过本进程活跃令牌与仍存活的 `owner_pid`）：子会话最后一次 run 已终态则沿用该结果并发 `subagent.completed`/`subagent.failed`，否则标为 `failed{interrupted}`。取消判定以子会话 run 终态为准，父取消与子完成竞争时不会把成功结果改标为 `cancelled`。子会话仍按 id 可访问（`/goto <id>`、Web 子代理面板“打开子会话”），但默认不出现在会话选择器、TUI 侧栏和 `GET /api/sessions`；`GET /api/sessions?include_children=true`（兼容 `1`）或 TUI `/sessions all` 可列出它们。
