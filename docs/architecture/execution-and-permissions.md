# 执行与权限

所有工具通过 `ExecutionBackend`，禁止在业务代码里直接 `std::process`。

## 后端

- `LocalBackend`：本机文件、搜索、shell、git 只读子集、apply_patch、后台进程登记。
- `WorktreeHandle`：在 `{workspace}/.blora/worktrees/{session}` 创建分离的 git worktree；`BLORA_WORKTREE=1` 或 `--worktree` 启用。
- `BLORA_EXEC=sandbox`：清空环境，并在可用时用 `unshare -n` 去掉网络。
- `BLORA_EXEC=bwrap`：bubblewrap 沙箱，根文件系统只读、工作区可写、私有 `/tmp`、无网络、独立 PID 命名空间；`BLORA_BWRAP_RW` 可追加可写路径（冒号分隔）。
- `BLORA_EXEC=container`：`docker run --rm --network=none` 挂载工作区；镜像由 `BLORA_CONTAINER_IMAGE` 指定。
- `BLORA_EXEC=pty` 或工具参数 `pty: true`：通过 `script(1)` 分配伪终端。
- `BLORA_EXEC=remote`：`ssh $BLORA_REMOTE` 在 `BLORA_REMOTE_ROOT` 执行。

## 工具

`read_file`（可选 offset/limit 行范围）、`write_file`（新建或整体替换）、`list_dir`、`search`、`shell`（可 background、pty、timeout_seconds ≤ 600）、`apply_patch`（old_string 必须唯一匹配，或 replace_all）、`git_status`、`git_diff`、`git_log`、`git_branch`、`git_worktree`、`process`、`schedule_task`、`delegate`、`handoff`、`remember`、`recall`、`forget`。MCP 工具以 `mcp__` 前缀接入，插件以 `plugin__` 前缀接入。

只读工具（read_file、list_dir、search、git 只读、recall、handoff）在同一批次内并发执行；只读角色的子代理只能看到只读工具。工具结果超过 24k 字符时头尾截断并在事件里标记 `truncated`。

## 权限

结果只有 `allow`、`deny`、`ask`。默认 fail-closed。写入、shell、worktree add、process kill 需要审批，除非 `--yes` / 自动批准。`tool-before` hook 返回 `ask` 时同样进入审批，返回 `block` 直接拒绝。

能力：文件读取默认允许，`.env` / 密钥文件名需审批；网络默认拒绝（`BLORA_NETWORK=1` 后才 Ask/Allow）；Git commit/push/reset/clean/rebase 为破坏性操作需审批。审批事件记录 capability、摘要、决定和 `policy_version`（当前 1）。单次 run 还受 turn 上限、`BLORA_MAX_TOKENS`、`BLORA_MAX_WALL_SECS`（默认 900）约束；后台任务另有 180 秒硬上限。
