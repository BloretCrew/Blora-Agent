# Blora Agent

从零实现的本地优先 Agent Harness，覆盖 **Code**、**Work** 和 **Agent** 三种工作模式。

本仓库是全新代码，不 Fork、不改造、不拼接任何现有 Harness。同目录下的其他项目只作设计对照。

**许可证：GNU GPL v3 或更高版本。** 见 [`LICENSE`](LICENSE)。

## 能力

- Canonical Event Log + SQLite 追加写存储
- 可恢复 Session / Run 状态机
- 本地工具：`read_file`、`write_file`、`list_dir`、`search`、`shell`
- 工作区路径沙箱，写入和命令默认需要 `--yes`
- Mock Provider 与 OpenAI 兼容流式接口
- CLI、TUI、本地 Web UI（Blora Design）

## 快速开始

```bash
cargo test --workspace
cargo run -p blora-cli -- session create --workspace .
cargo run -p blora-cli -- run --session ses_… --mock --yes "list files"
cargo run -p blora-cli -- tui
cargo run -p blora-cli -- serve --bind 127.0.0.1:8787
```

真实模型：

```bash
export BLORA_API_KEY=...
export BLORA_API_BASE=https://api.openai.com/v1   # 可选
export BLORA_MODEL=gpt-4o-mini                    # 可选
cargo run -p blora-cli -- run --session ses_… --yes "fix the failing test"
```

数据目录：`$BLORA_HOME`，默认 `~/.blora/state.sqlite`。

## 仓库结构

```text
Cargo.toml
crates/           Rust workspace
web/              Web UI
docs/             架构、ADR、研究材料
scripts/check.sh
```

对照用源码（不是本项目实现）仍放在 `grok-build/`、`hermes-agent/` 等目录。
