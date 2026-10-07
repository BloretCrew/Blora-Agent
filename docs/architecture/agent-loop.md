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

Provider 的推理流（reasoning）在每次调用结束时汇总为一条 `assistant.reasoning` 事件（单次最多 32k 字符），只用于展示与审计，不会回放给模型。

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

`delegate` 创建子会话，深度上限 2、并发 3。子代理收到按角色生成的任务说明（research / review / plan 只读，code 可写），说明里写明交付格式：结论在前，附证据路径与命令、改动文件和未决问题。`subagent.spawned` 记录 `depth`，`subagent.completed` 回写子会话的 token 与轮数，结果以 `<subagent>` 包裹返回父会话。`steer_subagent` / `cancel_subagent` 可在运行中干预。
