# Blora Agent 架构总览

Blora Agent 是从零实现的本地优先 harness。第一阶段是单用户模块化单体：Rust Core 可嵌入 TUI，也可作为本地 daemon 提供 Web API。

## 分层

1. **Domain**：Workspace、Session、Run、Event、Task、Agent、Policy。不依赖 IO。
2. **Runtime**：Agent Loop 状态机，每一步产生可持久化事件。
3. **Infrastructure**：SQLite、文件系统、进程、HTTP、Provider adapter。
4. **Interface**：CLI、TUI、Web API、后续 SDK/ACP。

## 已确定决策

- 从零开发，不 Fork 参考项目。
- Rust Core + 未来 TypeScript Web。
- Canonical Event Log 为 Session 真相。
- SQLite 本地存储，事件表追加写。
- Code / Work / Agent 共用一个 Runtime，用模式区分。

## 当前实现

工程文件位于仓库根目录。已实现：

- Canonical Event Log、SQLite 追加写、Session/Run projection
- 本地工具 `read_file` / `write_file` / `list_dir` / `search` / `shell`
- 工作区路径沙箱；写入和 shell 默认需审批
- Mock Provider 与 OpenAI 兼容流式 adapter
- CLI、TUI、本地 Web UI（Blora Design）
- Work：后台任务表、延迟调度、失败重试、session wakeup
- Agent：`delegate` 子代理，深度 2 / 并发 3 / 子代理 6 turn 预算
- 交互审批（TUI y/n，Web 允许/拒绝）
- Git 只读工具、apply_patch、上下文压缩、checkpoint、provider 重试与工具死循环检测
- Provider：Chat Completions、Responses、Anthropic；`BLORA_PROVIDER` 可用逗号做回退链
- Work cron（含列表/范围）、session fork/export/archive、MCP stdio（`BLORA_MCP_COMMAND`）、`blora acp`
- Git worktree 执行、后台 process 工具、`BLORA_HOOKS_DIR` 钩子、Web 任务/审批/工作区/设置页
- 项目规则/技能进上下文、显式记忆、插件 JSON、自动压缩、token 预算
- `BLORA_EXEC=sandbox|container` 隔离执行、`blora backup`/`restore`/`usage`、运行取消 API

```text
cargo run -p blora-cli --         # TUI
cargo run -p blora-cli -- --web   # Web
cargo run -p blora-cli -- web     # Web（子命令写法）
```
