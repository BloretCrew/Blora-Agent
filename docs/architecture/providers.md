# Provider 能力矩阵

内核只保存 canonical transcript。Adapter 负责编译请求、解析流、归一化 usage。

| Provider | 环境变量 | 协议 | 流式 | 工具调用 | Fixture |
|---|---|---|---|---|---|
| blora (Bloret PassPort) | 登录 PassPort 后默认，或 `BLORA_PROVIDER=blora` | Chat Completions | SSE | tool_calls | `passport.rs` |
| mock | `--mock` 或无密钥 | 内部 | 模拟 delta | 是 | runtime 测试 |
| openai | `BLORA_PROVIDER=openai` | Chat Completions | SSE | tool_calls | `crates/blora-model` |
| responses | `BLORA_PROVIDER=responses` | OpenAI Responses | SSE | function_call | `responses.rs` |
| anthropic | `BLORA_PROVIDER=anthropic` | Messages | SSE | tool_use | `anthropic.rs` |
| gemini | `BLORA_PROVIDER=gemini` | Gemini generateContent | SSE | functionCall | `gemini.rs` |

回退：`BLORA_PROVIDER=openai,anthropic`。认证失败不回退；429 / timeout / connection 可回退。

密钥：`BLORA_API_KEY`、`OPENAI_API_KEY`、`ANTHROPIC_API_KEY`。不写入事件。

## Bloret PassPort（blora）

Bloret PassPort 的 AI API 代理（`https://passport.bloret.net/v1`，OpenAI Chat Completions
兼容）。模型调用名固定为 `blora`（服务端会覆盖上游模型），展示名为 **Blora**。

认证使用 OAuth 三段式 Key `{AppID};{AppSecret};{UserToken}`：`UserToken` 来自 PassPort
登录（设备码或回调）后存储的 `passport_app_token`，`AppID`/`AppSecret` 复用
`BLORA_PASSPORT_APP_ID`/`BLORA_PASSPORT_APP_SECRET`（默认内置应用）。Web 与 TUI 登录
PassPort 后，未显式选择供应商的会话默认走 blora；显式 `provider` 请求仍然优先。
PassPort 每日限额见其 `/ai` 页面（默认 200 次/天）。

Gemini 及其他协议在内部模型稳定后再加。
