# Event Stream

客户端（TUI / Web / CLI）只消费 envelope，不直接拼 Provider 消息。

- 实时：SSE，连接时先发一条 `snapshot`（当前 sequence 与 run 状态），此后每追加一个事件推送一条 `{sequence, type, status, payload}`。`tool.output` 的 payload 不随流发送，需按需拉取。推送由运行时的订阅通道驱动，不再轮询。
- Web 客户端按类型增量处理：`assistant.delta` 直接追加到进行中的气泡，`user.input` / `tool.requested` 追加条目，`approval.*` 与 `task.*` 只刷新对应面板，其余事件重新拉取会话。
- `plan.updated` 覆盖 `projection.plan` / `plan_note`；`run.created.permission_mode` 记入 `projection.permission_mode`。
- 回放：`GET /api/sessions/:id/events` 按 sequence 返回。
- 投影：客户端可以自己 fold，也可以请求服务端 projection。TUI 只拉取新事件增量应用。
- 未知 type：显示为 generic system 项，不得断开流。
