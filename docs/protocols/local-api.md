# Local API

`blora web` 提供本地 HTTP API。Web UI 只通过这些接口访问 Runtime。

```text
GET    /api/sessions          session summaries: id, title, workspace_path, mode, status, updated_at, last_sequence
GET    /api/sessions?q=       same summary shape; full transcript stays on GET /api/sessions/:id
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
POST   /api/auth/device/poll        {"attempt_id": "…"}
POST   /api/auth/device/cancel      {"attempt_id": "…"}
POST   /api/auth/logout
GET    /ws
```

事件流使用 SSE。密钥不会出现在 `/api/settings` 中。

## 首次引导与设备授权

`GET /api/auth/device` 由用户主动登录时调用，返回 `attempt_id`、`user_code`、`verification_uri`、`expires_in`、`interval`，并设置绑定授权浏览器的 HttpOnly Cookie。内部设备凭据保留在服务器。一个浏览器的新授权会替换旧尝试，其他浏览器的尝试保持独立。

`POST /api/auth/device/poll` 执行一次令牌查询，返回 `status: pending | slow_down | authenticated`。等待状态包含下一次查询的 `interval` 秒数；成功包含 `authenticated`、`username`、`name` 并设置登录 Cookie。客户端按间隔调度，服务器同时限制查询频率和并发。拒绝、过期及无效尝试返回错误。

`POST /api/auth/device/cancel` 取消当前浏览器拥有的尝试。客户端离开账号步骤、重试或退出时应取消旧尝试，并忽略旧请求的响应。

登录 Cookie 使用随机会话标识，服务器负责映射到用户；退出立即撤销映射。当前映射保存在服务器内存中，重启 Web 服务后需要重新登录，用户令牌与引导完成记录继续保留。

`GET /api/auth/me` 返回账号信息及 `credentials_valid`，区分账号会话与 AI 凭据可用性。临近过期时尝试用已保存的 refresh token 刷新；无可用凭据时引导重新登录。OOBE 完成标记由终端或浏览器分别保存，网关 API 的访问权限继续由服务端认证决定。
