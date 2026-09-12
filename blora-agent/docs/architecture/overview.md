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

`blora-types`、`blora-events`、`blora-session`、`blora-storage`、`blora-runtime`、`blora-cli`。mock run 可创建 Session、写入事件、流式 assistant 文本、取消、重启后重建 projection。
