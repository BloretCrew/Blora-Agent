# 模式定位

Blora 的 Code、Work、Agent 三种模式共用一个 Runtime 和同一套事件模型。三种模式在提示词、工具集和默认行为上有区别。Imagine 是单独的图像生成模式，本文不展开。

| 模式 | 定位 | 设计参考 | 核心能力 |
|---|---|---|---|
| Code | 编码 | Claude Code、Codex、Grok Build | 读改代码、运行测试与构建、Git、代码审查、计划 |
| Work | 工作产出 | 办公与调研类代理 | 联网检索、读取 PDF/Office 文档、生成报告与表格文件、定时任务 |
| Agent | 长期自主助理 | Hermes Agent | 目标拆解、委派与定时、自我学习闭环（技能沉淀、记忆、跨会话检索） |

参考项目只用来对照设计思路。Blora 的实现全部从零编写，不复制其他 harness 的代码。

## 工具集

工具集由 `ToolRegistry::specs_for(mode)` 决定，提示词里的工具清单与实际下发的工具一致。

| 工具 | Code | Work | Agent |
|---|---|---|---|
| 文件、shell、Git、`apply_patch`、`update_plan`、`skill`、`delegate`、记忆工具 | ✓ | ✓ | ✓ |
| `ask_user` | ✓ | ✓ | ✓ |
| `web_search`、`web_fetch`、`read_document`、`schedule_task` | | ✓ | ✓ |
| `skill_manage`、`session_search` | | | ✓ |

子代理沿用父会话的模式。例如 Work 会话派出的 research 子代理也能联网。只读角色（research、review、plan）只拿到只读工具。

## 提示词

稳定 system 块的结构是“身份 + 模式策略 + 通用规则”。每种模式的内容字节稳定，可以命中前缀缓存。

- Code：先读后改，做最小改动，修根因；改完运行最相关的构建或测试，并用 `git_diff` 自查；不擅自 push 或改写历史；代码审查按严重度列出问题并给出 `path:line`。
- Work：先确定交付物和格式；较长的成果写成文件；先读用户提供的资料，再用 `web_search` 检索，用 `web_fetch` 阅读原文；非显而易见的事实要注明来源；数字用 shell 计算。
- Agent：用 `update_plan` 推进目标；能独立完成的部分用 `delegate` 委派，并给出完整说明；用户提到以前的工作时，先用 `session_search` 查历史；持久事实用 `remember` 保存；完成复杂任务后用 `skill_manage` 沉淀做法；只能根据本会话的亲身经验创建技能。

## 自我学习闭环（Agent）

- **技能**：`skill_manage` 可以创建、更新、删除 `.blora/skills/<name>.md`（`scope=workspace`）或 `$BLORA_HOME/skills/<name>.md`（`scope=global`），不需要确认。
  - 每次写入都记录 `skill.changed` 事件，事件中带有修改前的内容 `previous`，便于查看和撤销。
  - 名称只能使用小写字母、数字、`-` 和 `_`，内容最多 24 KB。
  - 新技能在下一轮请求中就会出现在技能列表里。
- **记忆**：`remember`、`forget` 会立即改写 `.blora/memory.md`；回复里以 `Remember:` 开头的行也会保存为记忆。会话摘要不再写入记忆，以免上下文越积越大。
- **历史检索**：`session_search` 查询 SQLite FTS5 全文索引（trigram 分词，支持中文）。索引覆盖用户消息、助手回复、问答和压缩摘要，不包括当前会话和子会话；登录用户只能检索自己的会话。

## 联网与文档（Work、Agent）

- `web_search` 根据环境变量自动选择后端：`SEARXNG_URL`、`BRAVE_SEARCH_API_KEY`、`TAVILY_API_KEY`、`EXA_API_KEY`，按此顺序选第一个已配置的。也可以用 `BLORA_SEARCH_BACKEND` 强制指定后端。
  - Blora 直接调用各服务公开的 HTTP 接口，没有引入任何 SDK。
  - 都没有配置时，返回配置说明。
- `web_fetch` 只抓取 http(s) 地址，HTML 转成可读文本。
  - 防护：拒绝私网、回环和链路本地地址，DNS 解析和每次重定向都会检查；最大 2 MB，超时 20 秒。
  - `BLORA_WEB_ALLOW_PRIVATE=1` 可以关闭私网拦截，只适合可信内网。
- 两个联网工具都受现有网络策略约束：未设置 `BLORA_NETWORK=1` 时一律拒绝；plan 模式拒绝；ask 模式需要审批；yolo 模式直接放行。
- `read_document` 读取 `.pdf`（需要系统安装 `pdftotext`）、`.docx`、`.xlsx`、`.pptx`。Office 文件由内置的 zip 解析器加 `flate2` 解压读取，不依赖外部程序。

## 向用户提问

`ask_user` 会写入 `input.requested` 事件，运行状态变为 `waiting_input`。界面通过 `Runtime::answer_input` 或 `POST /api/sessions/:id/input` 写入 `input.provided`，运行随即继续。

- TUI：提问显示在输入框上方。Enter 提交输入的文字，数字键选择选项，Esc 跳过。
- Web：对话区上方显示问题卡片，包括选项按钮、输入框和跳过按钮。
- 无人值守的运行（CLI `--print`、后台任务、子代理）不会阻塞，会直接告诉模型“无法询问，请自行判断并说明假设”。
