# Local API 草案

第一版通过 CLI 暴露能力。后续 `blora serve` 将提供：

```text
GET    /api/sessions
POST   /api/sessions
GET    /api/sessions/:id
GET    /api/sessions/:id/events
POST   /api/sessions/:id/runs
POST   /api/runs/:id/cancel
```

事件流使用 SSE。本版本尚未启动 HTTP 服务。
