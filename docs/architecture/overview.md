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
- 会话搜索、事件时间线、artifact 列表、工作区只读文件、TUI `/search`/`/tools`
- 权限能力（网络默认拒绝、密钥路径、Git 破坏性操作）、墙钟超时、tool.started/completed、workspaces 表
- 文档：OpenAPI、TUI 快捷键、Provider 矩阵
- Gemini provider、PTY（`script` / `pty: true`）、SSH 远程执行（`BLORA_EXEC=remote` + `BLORA_REMOTE`）
- 多用户 Gateway（`blora gateway` + `blora user add`）、WebSocket 控制、插件市场、自动记忆蒸馏
- 请求层：稳定/上下文/易失三层提示与缓存断点、模型驱动压缩（85% 阈值、逐字尾部、三次熔断）、指数退避重试与流空闲看门狗、cached token 计量
- 工具层：唯一匹配的 apply_patch、只读工具批次并发、工具结果截断与 microcompact、三态 hooks（allow/block/ask）
- Work/Agent：cron 至多一次语义与失活恢复、任务退避重试与 180s 硬上限、子代理深度与用量回写、运行中 steer
- 权限：四档 `PermissionMode`（plan / ask / auto-edit / yolo），shell 命令按段分级（只读 / 改动 / 危险），`run.created.permission_mode` 与 `<environment_context>` 告知模型；`update_plan` 清单工具与 `plan.updated` 事件

```text
cargo run -p blora-cli --         # TUI
cargo run -p blora-cli -- --web   # Web
cargo run -p blora-cli -- web     # Web（子命令写法）
```
