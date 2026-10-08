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
- `input.requested {request_id, question, options, call_id}`：`ask_user` 提问；Run 进入 `waiting_input`。
- `input.provided {request_id, answer}`：回答；`answer` 为 `null` 表示跳过。Run 回到 `running`。
- `skill.changed {name, action, scope, path, previous}`：Agent 创建、更新或删除技能；`previous` 保存修改前的内容。

`user.input`、`assistant.message.completed`、`input.*` 与 `context.compaction.completed` 的文本同时写入派生的 FTS5 索引 `transcript_fts`，供 `session_search` 使用。索引缺失时，会从事件日志重建。
