# Event Stream

客户端（TUI / Web / CLI）只消费 envelope，不直接拼 Provider 消息。

- 实时：SSE，每条事件一个 `data: {json}`。
- 回放：`GET /api/sessions/:id/events` 按 sequence 返回。
- 投影：客户端可以自己 fold，也可以请求服务端 projection。
- 未知 type：显示为 generic system 项，不得断开流。
