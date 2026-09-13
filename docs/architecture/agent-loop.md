# Agent Loop

Run 状态：`queued`、`running`、`waiting_approval`、`waiting_input`、`waiting_task`、`compacting`、`paused`、`completed`、`failed`、`cancelled`。后三者为终态。

当前 loop：mock 或真实 provider 流式输出，工具调用，审批，压缩，checkpoint。

当前 mock loop：

```text
run.created
run.started
user.input
model.requested
assistant.delta*
assistant.message.completed
model.response.completed
usage.recorded
run.completed
```

取消在非终态发出 `run.cancel_requested`，然后 `run.cancelled`。重复取消不得破坏已终态 Run。
