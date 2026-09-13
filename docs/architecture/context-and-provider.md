# Context 与 Provider

内部上下文分三层：

- **Stable**：身份、核心规则、安全策略。前缀尽量字节稳定，便于缓存。
- **Context**：工作区、项目规则、环境快照、工具目录、权限。
- **Volatile**：当前输入、近期事件、后台完成、reminder。

`blora-context` 把事件编译为 stable / context / volatile 三层 system prompt 加上 transcript。Provider adapter 再编译成具体协议：OpenAI Responses、Anthropic Messages、Chat Completions。内核绝不把某一协议的 payload 当作事实来源。`BLORA_PROVIDER=openai,anthropic` 在可重试错误时回退。
