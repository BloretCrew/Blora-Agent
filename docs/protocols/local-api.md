# Local API

`blora web` 提供本地 HTTP API。Web UI 只通过这些接口访问 Runtime。

```text
GET    /api/sessions
POST   /api/sessions
GET    /api/sessions/:id
POST   /api/sessions/:id/run
GET    /api/sessions/:id/events
POST   /api/sessions/:id/fork
GET    /api/sessions/:id/export
POST   /api/sessions/:id/archive
POST   /api/sessions/:id/resume
POST   /api/sessions/:id/compact
GET    /api/tasks
POST   /api/tasks
POST   /api/tasks/:id/cancel
POST   /api/tasks/:id/pause
POST   /api/tasks/:id/resume
POST   /api/tasks/pump
GET    /api/approvals
POST   /api/approvals/:id/resolve
GET    /api/workspace
GET    /api/settings
```

事件流使用 SSE。密钥不会出现在 `/api/settings` 中。
