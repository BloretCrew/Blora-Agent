# Development

```bash
scripts/check.sh
cargo test --workspace
cargo run -p blora-cli -- session create --workspace .
cargo run -p blora-cli -- run --session ses_… --mock --yes "list files"
cargo run -p blora-cli -- run --session ses_… --permission plan "how should we add caching?"
cargo run -p blora-cli -- run --session ses_… --permission auto-edit "fix the failing test"
cargo run -p blora-cli --
cargo run -p blora-cli -- web --bind 127.0.0.1:8787
cargo run -p blora-cli -- session fork ses_…
cargo run -p blora-cli -- session export ses_…
```

TUI 快捷键见 [`tui.md`](tui.md)。Provider 矩阵见 [`../architecture/providers.md`](../architecture/providers.md)。OpenAPI 见 [`../protocols/openapi.yaml`](../protocols/openapi.yaml)。

`BLORA_PROVIDER` 可写 `openai,anthropic`。`BLORA_NETWORK=1` 才允许 curl/ssh 等网络命令。无头一次运行：`blora -p "提示"`（或把提示从 stdin 读入）。`--output-format text|json|stream-json`，`--max-turns`，`--session` 可续跑已有会话。需要审批的工具在非交互运行里直接失败，进程退出码为 2。权限模式 `--permission plan|ask|auto-edit|yolo`（`--yes` = yolo）见 [`../architecture/execution-and-permissions.md`](../architecture/execution-and-permissions.md)。`BLORA_MAX_WALL_SECS` 限制单次 run 墙钟（默认 900）。`BLORA_WORKTREE=1` 在 git worktree 中执行。`BLORA_HOOKS_DIR` 可放 `session-start` / `user-prompt-submit` / `tool-before` / `tool-after` / `pre-compact` / `stop` 可执行文件（stdin JSON，stdout JSON，退出码 2 阻断）。`BLORA_MCP_COMMAND` 启动 stdio MCP。`BLORA_PLUGINS_DIR` 或 `{workspace}/.blora/plugins/*.json` 加载插件。`BLORA_EXEC=sandbox|bwrap|container` 隔离 shell。`BLORA_MAX_TOKENS` 限制累计 token。`blora backup` / `blora restore` 复制 SQLite。

请求层：`BLORA_MAX_RETRIES`（默认 5）、`BLORA_STREAM_IDLE_SECS`（默认 300）、`BLORA_MAX_OUTPUT_TOKENS`（默认 8192）、`BLORA_CONTEXT_WINDOW`（默认 128000）、`BLORA_COMPACT_PCT`（默认 85）。

`blora gateway --bind 0.0.0.0:8787` 开启 token 鉴权。`blora user add NAME` 生成 `blt_` token。`blora plugin list|install|remove`。`blora memory distill ses_…`。

`BLORA_HOME` 可指向临时目录做隔离测试。真实模型使用 `BLORA_API_KEY` / `BLORA_API_BASE` / `BLORA_MODEL` / `GEMINI_API_KEY`。

## 首次使用与账号登录

首次启动 TUI 或首次访问 Web 时显示 OOBE（首次使用引导）：欢迎 → 连接账号 → 完成。点击“登录 Bloret PassPort”后才申请设备码并开始授权；本地模式可选择“稍后设置”，继续使用已有自定义供应商和环境变量凭据。默认 PassPort 供应商仍要求有效登录。

TUI 将引导版本记录在当前 `BLORA_HOME` 的 SQLite 数据库中，Web 在当前浏览器保存非敏感完成标记。点击“开始使用”后保存完成状态；有效的已有账号自动完成当前版本引导。中途退出可在下次启动继续设置。TUI 使用 `/onboarding` 重开引导、`/login` 打开账号步骤、`/logout` 退出；Web 在设置中提供重开入口，侧栏提供账号与退出入口。令牌过期可重新登录，退出后等待用户主动发起登录。

登录使用 OAuth 设备授权：显示用户可见的 `user_code` 与 `verification_uri`，打开 Passport 页面批准后保存用户信息。内部 `device_code` 和访问令牌仅在服务端处理。Web 按浏览器隔离授权尝试，支持取消、重试与过期反馈；关闭账号步骤会停止该次授权。

回归脚本：重建 `blora` 后可运行 `python3 scripts/oobe-tui-test.py`；启动隔离的 Web 服务后，安装 Playwright 并运行 `BLORA_TEST_URL=http://127.0.0.1:18787 node scripts/oobe-browser-test.cjs`。浏览器脚本使用本地页面与模拟账号接口，覆盖桌面、移动端、授权错误及周边路由；实际 OAuth 交互由服务器测试中的本地模拟授权服务验证。

App ID 和 App Secret 使用内置应用配置，也可通过 `BLORA_PASSPORT_APP_ID`、`BLORA_PASSPORT_APP_SECRET` 覆盖；另可设置 `BLORA_PASSPORT_URL` 和 `BLORA_PUBLIC_URL`。设备授权遵循 `/oauth/device/code`、`/oauth/token` 和 `/oauth/userinfo`。网关继续执行原有 Bearer token 访问认证，引导完成状态仅控制界面流程。
