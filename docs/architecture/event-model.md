# Canonical Event Model

事件追加后不可修改。纠正使用新事件。未知 `type` 必须被保留，projection 跳过它们，不能让整个 Session 无法读取。

## Envelope

```json
{
  "event_id": "evt_…",
  "schema_version": 1,
  "session_id": "ses_…",
  "run_id": "run_…",
  "turn_id": "trn_…",
  "sequence": 1,
  "timestamp": "2026-09-12T00:00:00Z",
  "actor": "user",
  "visibility": "user",
  "type": "user.input",
  "payload": { "text": "hello" },
  "metadata": {}
}
```

`sequence` 在每个 Session 内从 1 连续递增，由存储层分配。

## 首批类型

`session.*`、`run.*`、`user.input`、`assistant.delta`、`assistant.message.completed`、`model.*`、`tool.*`、`approval.*`、`task.*`、`subagent.*`、`context.*`、`retry.started`、`usage.recorded`、`artifact.created`、`checkpoint.created`。

## 后续类型

- `assistant.reasoning {text}`：模型推理文本，只用于展示，不回放给 Provider。
- `plan.updated {steps, note}`：最新的 `update_plan` 清单。
- `input.requested {request_id, question, options, call_id, questions}`：`ask_user` 提问。`questions` 列出每一题的 `kind`、`question`、`header` 和 `options`。旧日志没有 `questions` 时仍可回放，界面会按 `question` 和 `options` 合成一道题。Run 进入 `waiting_input`。
- `input.provided {request_id, answer}`：回答；`answer` 为 `null` 表示跳过。Run 回到 `running`。
- `skill.changed {name, action, scope, path, previous}`：Agent 创建、更新或删除技能；`previous` 保存修改前的内容。
- `permission.changed {from, to, source}`：会话级权限模式变化；`source` 为 `runtime`、`user` 或 `tool`。
- `plan.mode.changed {entered, reason}`：进入或退出 plan mode。
- `tool.discovered {query, tools, source}`：`tool_search` 启用了额外工具，下一轮请求会声明它们。

## 多模态附件

`user.input`、`assistant.message.completed` 和 `tool.output` 可以携带 `attachments: AttachmentRef[]`：

```json
{
  "id": "att_<sha256>",
  "kind": "image",
  "mime": "image/png",
  "path": ".blora/attachments/<session>/<sha256>.png",
  "sha256": "...",
  "bytes": 12345,
  "width": 800,
  "height": 600
}
```

事件只保存引用，不保存图片字节。旧事件没有该字段时按空数组反序列化；回放时由 `AttachmentResolver` 按需读取并转成 Provider 的内容块。`tool.output.text` 仍是唯一的文本来源，`attachments` 只承载图片/文件。

`user.input`、`assistant.message.completed`、`input.*` 与 `context.compaction.completed` 的文本同时写入派生的 FTS5 索引 `transcript_fts`，供 `session_search` 使用。索引缺失时，会从事件日志重建。
