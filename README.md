# Blora Agent

从零实现的本地优先 Agent Harness，覆盖 **Code**、**Work** 和 **Agent** 三种工作模式。

本仓库是全新代码，不 Fork、不改造、不拼接任何现有 Harness。同目录下的其他项目只作设计对照。

**许可证：GNU GPL v3 或更高版本。** 见 [`LICENSE`](LICENSE)。

## 能力

- Canonical Event Log + SQLite 追加写存储
- 可恢复 Session / Run 状态机
- 本地工具：`read_file`、`write_file`、`list_dir`、`glob`、`search`、`shell`、`apply_patch`、`process`；超长工具输出落盘到 `.blora/tool-output/` 可分页回读
- 项目规则从仓库根逐级向下发现 `AGENTS.md` / `CLAUDE.md`，外加全局 `~/.blora/rules.md`
- 工作区路径沙箱；权限模式 `plan` / `ask` / `auto-edit` / `yolo`（`--permission`，`--yes` = yolo），shell 命令按只读 / 改动 / 危险分级，危险命令永远要确认
- 模型用 `update_plan` 维护清单，`plan.updated` 事件进投影，CLI 与 Web API 可见
- Mock Provider 与 OpenAI Chat / Responses / Anthropic 流式接口
- CLI、TUI、本地 Web UI（Blora Design；会话 / 任务 / 审批 / 工作区 / 设置）
- Work 模式：延迟任务、cron、暂停/恢复、`blora task pump`
- Agent 模式：`delegate` / `handoff`，深度/并发预算；research/review/plan 只读
- TUI/Web 交互审批、Git 工具、worktree 执行、上下文压缩、checkpoint、artifact
- Provider：`openai` / `responses` / `anthropic` / `mock`，逗号分隔回退
- `blora session fork|export|archive`、MCP（`BLORA_MCP_COMMAND`）、`blora acp`、`BLORA_HOOKS_DIR`
- 插件 JSON、显式记忆、sandbox/container 执行、`blora backup` / `usage`
- 会话搜索、事件时间线、产物与工作区文件只读查看
- Gemini、PTY、SSH 远程执行、多用户 Gateway、插件市场、自动记忆、WebSocket
- GitHub：`blora github install` 后，在 Issue 或 PR 评论 `/blora`、`/ba` 或 `@blora`，由评论决定回复、改代码或开 PR

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

数据目录：`$BLORA_HOME`，默认 `~/.blora/state.sqlite`。网络命令默认拒绝，设置 `BLORA_NETWORK=1` 后才可审批执行。

### Windows

在 Windows 上用同样的 `cargo run -p blora-cli` 启动 TUI 或 Web。需要已安装 Git，因为工具命令走 POSIX shell：

- 若 `bash` 或 `sh` 在 `PATH` 上（Git for Windows 会提供），命令用 `sh -c` 执行，`cd`、管道和引号与 Linux 一致。
- 否则使用 `cmd.exe /D /C`。这时 `ls` 这类命令不可用，除非它们自己也在 `PATH` 上。
- `BLORA_SHELL` 可指定解释器：`bash` 的路径、`cmd`，或 `powershell`。
- 浏览器用 `cmd /C start` 打开。复制文本优先用 `clip`，粘贴用 PowerShell 的 `Get-Clipboard`。这些子进程不弹出控制台窗口。
- 结束超时进程用 `taskkill /T /F`。`BLORA_EXEC=pty` 在 Windows 上退回普通 shell，因为没有 Unix 的 `script`。
- `BLORA_EXEC=bwrap` 只在 Linux 上可用。Docker 隔离仍然调用本机 `docker`。
- Hook 可以是 `.exe`、`.cmd`、`.bat`、`.ps1`。没有扩展名的脚本只在 POSIX shell 下按 shebang 运行。
- 数据目录默认是 `%USERPROFILE%\.blora`。Unix 上该目录权限为 `0700`；Windows 沿用用户配置目录自己的访问控制。

## 仓库结构

```text
Cargo.toml
crates/           Rust workspace
web/              Web UI
docs/             架构、ADR、研究材料
scripts/check.sh
```

对照用源码（不是本项目实现）仍放在 `grok-build/`、`hermes-agent/` 等目录。
