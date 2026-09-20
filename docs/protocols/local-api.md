# Local API

`blora web` 提供本地 HTTP API。Web UI 只通过这些接口访问 Runtime。

```text
GET    /api/sessions
GET    /api/sessions?q=
POST   /api/sessions
GET    /api/sessions/:id
POST   /api/sessions/:id/run         {"prompt", "permission_mode": "plan|ask|auto-edit|yolo", ...}
GET    /api/sessions/:id/events
POST   /api/sessions/:id/fork
GET    /api/sessions/:id/export
POST   /api/sessions/:id/archive
POST   /api/sessions/:id/resume
POST   /api/sessions/:id/compact
POST   /api/sessions/:id/cancel
POST   /api/sessions/:id/steer        {"message": "..."} — delivered at the next turn boundary
GET    /api/tasks
POST   /api/tasks
POST   /api/tasks/:id/cancel
POST   /api/tasks/:id/pause
POST   /api/tasks/:id/resume
POST   /api/tasks/pump
GET    /api/approvals
POST   /api/approvals/:id/resolve
GET    /api/workspace
GET    /api/workspace/file?path=
GET    /api/artifacts
GET    /api/settings
GET    /api/usage
GET    /api/plugins
GET    /api/marketplace
POST   /api/marketplace
GET    /api/auth/me
GET    /api/auth/device
POST   /api/auth/device/poll
POST   /api/auth/logout
GET    /ws
```

事件流使用 SSE。密钥不会出现在 `/api/settings` 中。
