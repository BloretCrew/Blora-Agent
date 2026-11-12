# 子代理

Blora 支持同步 `delegate` 和显式的 `subagent` 工具。子代理是独立会话，继承父会话的 workspace 和模式，只把最终报告交回父代理。

## 工具

`subagent { action, agent_id?, role?, prompt?, message?, background?, timeout_seconds? }`
- `spawn`：创建子代理。`role` 为 `research`、`review`、`plan`、`code` 或 `worker`；`background=true`（默认）立即返回 `agent_id`，同步模式等待完成并返回报告。
- `list`：列出父会话的子代理及其状态、预算和摘要。
- `send`：向运行中的子代理排队一条 steering 消息，在下一个 turn 边界送达。
- `wait`：等待子代理离开 `running`，默认 60 秒，返回最终状态与摘要。
- `resume`：复用已结束子代理的 child session，追加新 prompt 并在后台继续运行。
- `close`：取消子代理的 CancelToken 并把状态标记为 `cancelled`。
- `status`：读取单个子代理。

`delegate` 保留为同步兼容入口。

## 角色与限制

| 角色 | turn 预算 | 只读 |
|---|---|---|
| research | 6 | 是 |
| review | 6 | 是 |
| plan | 4 | 是 |
| code | 8 | 否 |
| worker | 6 | 否 |

- 最大嵌套深度 2，每个父会话最多 3 个运行中的子代理。
- 只读角色使用 Plan 权限，工具集限定为只读。
- 子代理不能询问用户；需要决策时在最终报告里说明假设。

## 后台运行

- `Runtime` 可克隆，所有共享状态在 `Arc` 内。
- 后台子代理在独立线程运行，取消令牌存入 `subagent_cancels`；`close` 会触发取消。
- 父代理可以继续工作，随后用 `subagent wait` 取结果，或 `subagent list` 查看进度。
- 事件：`subagent.spawned`、`subagent.message`、`subagent.completed`、`subagent.failed`。

## 结果注入

同步 `delegate` 直接把 `<subagent ...>` 报告作为工具结果返回。后台子代理通过 `wait` 返回报告；父会话的投影同时记录 `subagents` 列表，供 TUI/Web 展示。

后台子代理完成后，父会话下一轮会自动注入一条 `[subagent <role> <id> result]` 用户消息；`agents.summary_delivered` 防止重复注入。父代理不需要主动轮询也能看到结果。
