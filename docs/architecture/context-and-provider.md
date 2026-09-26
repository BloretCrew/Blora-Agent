# Context 与 Provider

内部上下文分三层，目标是让请求前缀逐字节稳定，命中 Provider 的前缀缓存：

- **Stable**（system 块 1）：身份、核心规则、安全策略。进程生命周期内字节不变，打缓存断点。
- **Context**（system 块 2）：项目规则、`.blora/skills/*.md`、`.blora/memory.md`。只在磁盘文件变化时改变，打缓存断点。项目规则按从泛到专的顺序拼接：`$BLORA_HOME/rules.md`（全局）→ 从仓库根（含 `.git` 的目录，没有则最多向上 6 层）到工作区逐级的 `AGENTS.md` / `CLAUDE.md` → 工作区的 `.blora/rules.md`；每份最多 4000 字节，最多 10 份。
- **Volatile**：日期、OS、Git 快照、当前 shell 工作目录封装为 `<environment_context>` user 片段，放在最新一条用户输入之前，永不进入 system 块。工作目录每轮从 `session.cwd.changed` 重读。

`blora-context::compile_messages` 负责装配，并在最新用户输入与最新工具结果上再打两个断点，总数不超过 Anthropic 的 4 个上限。OpenAI 系与 Gemini 走服务端自动前缀缓存，请求携带 `prompt_cache_key`（会话 id）作为亲和键。

技能目录 `.blora/skills/` 只把名字和一行摘要放进上下文（`*.md` 优先于同名目录里的 `SKILL.md`，最多 32 个）。全文由只读工具 `skill` 按名字读取，避免每轮把技能正文打进缓存前缀。

## 工具结果卫生

- 单条工具结果超过 24k 字符时头尾截断，事件里标记 `truncated`。
- 工具结果累计超过 60k 字符后，除最近 5 条外全部替换为占位符（microcompact）。用户与助手文本不受影响。

## 压缩

- 触发：估算 token 数达到窗口的 85%（`BLORA_CONTEXT_WINDOW`，默认 128k；`BLORA_COMPACT_PCT`）。
- 方式：把 system 块之外的历史交给当前模型写结构化摘要（目标、约束、进度、决策、文件与符号、全部用户消息逐字、下一步）。最近约 6k token 的尾部逐字保留，切点只落在用户消息边界。
- 守卫：摘要缩减不足 20% 判失败；连续失败 3 次后熔断，不再自动压缩。手动 `/compact` 在无模型时退化为确定性摘要。
- 记录：`context.compaction.completed` 携带 `preserve_from_sequence`、前后 token 数；失败记录 `context.compaction.failed`。

## Provider 层

`BLORA_PROVIDER=openai,anthropic` 为回退链。每个 Provider 内部最多重试 `BLORA_MAX_RETRIES`（默认 5）次：429 与 5xx 重试，4xx 直接失败，`Retry-After` 优先于本地指数退避（1s 起，封顶 30s，±20% 抖动）。401/403 不会回退到下一个 Provider。流式读取有空闲看门狗（`BLORA_STREAM_IDLE_SECS`，默认 300）。`BLORA_MAX_TOKENS` 限制累计用量，`BLORA_MAX_OUTPUT_TOKENS` 限制单次输出（默认 8192）。
