# Provider 能力矩阵

内核只保存 canonical transcript。Adapter 负责编译请求、解析流、归一化 usage。

| Provider | 环境变量 | 协议 | 流式 | 工具调用 | Fixture |
|---|---|---|---|---|---|
| mock | `--mock` 或无密钥 | 内部 | 模拟 delta | 是 | runtime 测试 |
| openai | `BLORA_PROVIDER=openai` | Chat Completions | SSE | tool_calls | `crates/blora-model` |
| responses | `BLORA_PROVIDER=responses` | OpenAI Responses | SSE | function_call | `responses.rs` |
| anthropic | `BLORA_PROVIDER=anthropic` | Messages | SSE | tool_use | `anthropic.rs` |

回退：`BLORA_PROVIDER=openai,anthropic`。认证失败不回退；429 / timeout / connection 可回退。

密钥：`BLORA_API_KEY`、`OPENAI_API_KEY`、`ANTHROPIC_API_KEY`。不写入事件。

Gemini 及其他协议在内部模型稳定后再加。
