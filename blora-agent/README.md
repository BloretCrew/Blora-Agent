# Blora Agent

从零实现的本地优先 Agent Harness，同时覆盖 **Code**、**Work** 和 **Agent** 三种工作模式。

本仓库是全新代码，不 Fork、不改造、不拼接任何现有 Harness。参考项目只用于设计对照。

**许可证：GNU GPL v3 或更高版本。** 完整文本见 [`LICENSE`](LICENSE)。

## 当前状态

Phase 0/1 已落地：

- Canonical Event Log
- Session / Run 状态机与 projection 重建
- SQLite 追加写事件存储
- 可恢复、可回放的 mock Agent Loop
- `blora` CLI

尚未实现：真实 Provider、本地工具执行、TUI、Web。

## 快速开始

需要 Rust 1.85+（本仓库锁定 1.98.1）。

```bash
cargo test --workspace
cargo run -p blora-cli -- license
cargo run -p blora-cli -- session create --workspace .
cargo run -p blora-cli -- session list
cargo run -p blora-cli -- run --session ses_… "hello"
```

默认数据目录：`$BLORA_HOME`，否则 `~/.blora/state.sqlite`。

## 架构要点

```text
TUI / Web / CLI
        │
        ▼
   Rust Runtime
        │
        ▼
 Canonical Event Log  →  Session Projection
        │
        ▼
   SQLite (append-only events)
```

内核不把任何 Provider 的 `messages[]` 当作事实来源。事件日志才是 Session 真相。

## 仓库结构

```text
crates/
  blora-types      标识符、错误、状态
  blora-events     Canonical Event Model
  blora-session    Session / Run projection
  blora-storage    SQLite 事件存储
  blora-runtime    Agent Loop coordinator
  blora-cli        blora 命令行
docs/              架构规格与 ADR
```

## 许可

Copyright (C) 2026 Blora Agent contributors

本程序是自由软件：你可以在 GNU GPL v3 或（由你选择）任何更高版本下再分发和修改。详见 [`LICENSE`](LICENSE) 与 [`NOTICE`](NOTICE)。
