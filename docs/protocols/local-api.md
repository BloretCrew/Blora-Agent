# Local API 草案

`blora web` 提供本地 HTTP API：

```text
GET    /api/sessions
POST   /api/sessions
GET    /api/sessions/:id
GET    /api/sessions/:id/events
POST   /api/sessions/:id/runs
POST   /api/runs/:id/cancel
```

事件流使用 SSE。Web UI 只通过这些接口访问 Runtime。
