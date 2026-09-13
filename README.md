# Blora Agent

从零实现的本地优先 Agent Harness，覆盖 **Code**、**Work** 和 **Agent** 三种工作模式。

本仓库是全新代码，不 Fork、不改造、不拼接任何现有 Harness。同目录下的其他项目只作设计对照。

**许可证：GNU GPL v3 或更高版本。** 见 [`LICENSE`](LICENSE)。

## 能力

- Canonical Event Log + SQLite 追加写存储
- 可恢复 Session / Run 状态机
- 本地工具：`read_file`、`write_file`、`list_dir`、`search`、`shell`、`apply_patch`、`process`
- 工作区路径沙箱，写入和命令默认需要 `--yes`
- Mock Provider 与 OpenAI Chat / Responses / Anthropic 流式接口
- CLI、TUI、本地 Web UI（Blora Design；会话 / 任务 / 审批 / 工作区 / 设置）
- Work 模式：延迟任务、cron、暂停/恢复、`blora task pump`
- Agent 模式：`delegate` / `handoff`，深度/并发预算；research/review/plan 只读
- TUI/Web 交互审批、Git 工具、worktree 执行、上下文压缩、checkpoint、artifact
- Provider：`openai` / `responses` / `anthropic` / `mock`，逗号分隔回退
- `blora session fork|export|archive`、MCP（`BLORA_MCP_COMMAND`）、`blora acp`、`BLORA_HOOKS_DIR`
- 插件 JSON、显式记忆、sandbox/container 执行、`blora backup` / `usage`

## 快速开始

```bash
cargo test --workspace
cargo run -p blora-cli --                  # TUI
cargo run -p blora-cli -- --web            # Web UI，默认 http://127.0.0.1:8787
cargo run -p blora-cli -- web              # 同上
cargo run -p blora-cli -- session create --workspace .
cargo run -p blora-cli -- run --session ses_… --mock --yes "list files"
cargo run -p blora-cli -- task create --session ses_… --title scan --prompt "list files" --cron "0 * * * *" --mock --yes
cargo run -p blora-cli -- task pump
cargo run -p blora-cli -- acp
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
