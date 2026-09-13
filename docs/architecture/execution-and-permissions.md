# 执行与权限

所有工具通过 `ExecutionBackend`，禁止在业务代码里直接 `std::process`。

## 后端

- `LocalBackend`：本机文件、搜索、shell、git 只读子集、apply_patch、后台进程登记。
- `WorktreeHandle`：在 `{workspace}/.blora/worktrees/{session}` 创建分离的 git worktree；`BLORA_WORKTREE=1` 或 `--worktree` 启用。
- Sandbox / Container / Remote 仍为后续阶段。

## 工具

`read_file`、`write_file`、`list_dir`、`search`、`shell`（可 background）、`apply_patch`、`git_status`、`git_diff`、`git_log`、`git_branch`、`git_worktree`、`process`、`schedule_task`、`delegate`、`handoff`。MCP 工具以 `mcp__` 前缀接入。

## 权限

结果只有 `allow`、`deny`、`ask`。默认 fail-closed。写入、shell、worktree add、process kill 需要审批，除非 `--yes` / 自动批准。审批事件记录 capability、摘要、决定。
