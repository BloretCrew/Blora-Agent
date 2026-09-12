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
