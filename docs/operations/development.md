# Development

```bash
scripts/check.sh
cargo test --workspace
cargo run -p blora-cli -- session create --workspace .
cargo run -p blora-cli -- run --session ses_… --mock --yes "list files"
cargo run -p blora-cli --
cargo run -p blora-cli -- web --bind 127.0.0.1:8787
cargo run -p blora-cli -- session fork ses_…
cargo run -p blora-cli -- session export ses_…
```

TUI 快捷键见 [`tui.md`](tui.md)。Provider 矩阵见 [`../architecture/providers.md`](../architecture/providers.md)。OpenAPI 见 [`../protocols/openapi.yaml`](../protocols/openapi.yaml)。

`BLORA_PROVIDER` 可写 `openai,anthropic`。`BLORA_NETWORK=1` 才允许 curl/ssh 等网络命令。`BLORA_MAX_WALL_SECS` 限制单次 run 墙钟（默认 900）。`BLORA_WORKTREE=1` 在 git worktree 中执行。`BLORA_HOOKS_DIR` 可放 `session-start` / `tool-before` / `tool-after` 可执行文件。`BLORA_MCP_COMMAND` 启动 stdio MCP。`BLORA_PLUGINS_DIR` 或 `{workspace}/.blora/plugins/*.json` 加载插件。`BLORA_EXEC=sandbox|container` 隔离 shell。`BLORA_MAX_TOKENS` 限制累计 token。`blora backup` / `blora restore` 复制 SQLite。

`blora gateway --bind 0.0.0.0:8787` 开启 token 鉴权。`blora user add NAME` 生成 `blt_` token。`blora plugin list|install|remove`。`blora memory distill ses_…`。

`BLORA_HOME` 可指向临时目录做隔离测试。真实模型使用 `BLORA_API_KEY` / `BLORA_API_BASE` / `BLORA_MODEL` / `GEMINI_API_KEY`。

首次启动 TUI 或 Web 时会通过 Bloret PassPort 登录：App ID 和 App Secret 使用内置应用配置，也可通过 `BLORA_PASSPORT_APP_ID`、`BLORA_PASSPORT_APP_SECRET` 覆盖；另可设置 `BLORA_PASSPORT_URL` 和 `BLORA_PUBLIC_URL`。TUI 和 Web 使用 OAuth 设备码登录：先显示 `user_code` 与 `verification_uri`，再打开或点击 Passport 页面，客户端按服务端间隔轮询授权结果。设备码和访问令牌不会展示或写入日志；登录成功后再保存用户信息。设备码接口遵循 `/oauth/device/code`、`/oauth/token` 和 `/oauth/userinfo`。
