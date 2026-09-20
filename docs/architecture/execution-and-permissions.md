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

`read_file`（可选 offset/limit 行范围）、`write_file`（新建或整体替换）、`list_dir`、`search`、`shell`（可 background、pty、timeout_seconds ≤ 600）、`apply_patch`（old_string 必须唯一匹配，或 replace_all）、`git_status`、`git_diff`、`git_log`、`git_branch`、`git_worktree`、`process`、`schedule_task`、`delegate`、`handoff`、`remember`、`recall`、`forget`、`update_plan`。MCP 工具以 `mcp__` 前缀接入，插件以 `plugin__` 前缀接入。

只读工具（read_file、list_dir、search、git 只读、recall、handoff、update_plan）在同一批次内并发执行；只读角色的子代理只能看到只读工具。工具结果超过 24k 字符时头尾截断并在事件里标记 `truncated`。

`update_plan` 让模型维护一份清单（`steps[{title,status}]`，status 为 `pending` / `in_progress` / `done`，可附 `note`）。每次调用整体替换，落为 `plan.updated` 事件；投影里保留最新一份（`projection.plan` / `plan_note`），CLI `session show` 打印，Web API 的会话 JSON 带 `plan` / `plan_note`。

## 权限模式

`PermissionMode` 有四档，从严到松：

| 模式 | 文件写入 | 只读命令 | 改动状态的命令 | 危险命令 | 网络 |
|---|---|---|---|---|---|
| `plan` | 只允许 `.blora/plan.md`，其余 Deny | Allow | Deny | Deny | Deny |
| `ask`（默认） | Ask | Ask | Ask | Ask | `BLORA_NETWORK=1` 时 Ask |
| `auto-edit` | Allow（密钥文件仍 Ask） | Allow | Ask | Ask | `BLORA_NETWORK=1` 时 Ask |
| `yolo` | Allow | Allow | Allow | **Ask** | `BLORA_NETWORK=1` 时 Allow |

`--yes` 等价于 `--permission yolo`；`--permission` 显式指定时覆盖 `--yes`。Web API 通过请求体 `permission_mode` 指定。只读角色的子代理固定为 `plan`。运行时把模式写进 `run.created.permission_mode`、投影的 `permission_mode` 和 `<environment_context>`，让模型知道当前能做什么。用户批准一次后，重试用的 `Policy::granting()` 只把 Ask 变 Allow，Deny 仍是 Deny。

### shell 命令分级

`blora-policy::classify_shell` 先按 `&&` / `||` / `;` / `|` / `&` / 换行拆段（引号内不拆，`2>&1` 与 `&>` 视为重定向），再逐段判定：

- **只读**：首词在只读清单里（ls / cat / grep / rg / find / git status|log|diff|show… / cargo check|test|build|clippy… / npm test|run… 等），且没有输出重定向（`2>&1`、`2>/dev/null` 除外）、没有 `find -exec` 或 `awk system()`。`git commit`、`cargo install`、`sed -i` 一律不算只读。
- **危险**：`rm -r`、`sudo` / `su` / `doas`、`dd` / `mkfs` / `shred`、`kill` / `pkill`、`chmod -R`、`git push --force|-d`、`git reset --hard`、`git clean`、`git branch -d`、`find -delete|-exec`、`xargs rm`、`eval` / `exec`、`crontab`（非 `-l`）、`| sh` / `| bash`、`> /dev/sd*`、fork bomb、含 `rm` 的命令替换。任一段危险即整条危险。
- **其余**为改动状态的命令。

危险命令在任何模式下都不会静默放行（yolo 也 Ask，plan 直接 Deny）。网络命令（curl / wget / ssh / scp / nc / rsync…）单独走 `network()` 判定。

## 审批

结果只有 `allow`、`deny`、`ask`。默认 fail-closed。`tool-before` hook 返回 `ask` 时同样进入审批，返回 `block` 直接拒绝。

能力：文件读取默认允许，`.env` / 密钥文件名需审批；网络默认拒绝（`BLORA_NETWORK=1` 后才 Ask/Allow）；Git commit/push/reset/clean/rebase/merge 为破坏性操作，按上表处理。审批事件记录 capability、摘要、决定和 `policy_version`（当前 2，权限模式引入时递增）。单次 run 还受 turn 上限、`BLORA_MAX_TOKENS`、`BLORA_MAX_WALL_SECS`（默认 900）约束；后台任务另有 180 秒硬上限。
