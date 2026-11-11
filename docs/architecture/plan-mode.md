# Plan mode

Plan mode 让模型先研究、写计划，再由用户批准后退出。它是会话级权限模式，不再只是单次 run 的参数。

## 工具

- `enter_plan_mode {reason?}`：把会话切到 `plan`。当前模式记录在 `permission.before_plan`。
- `exit_plan_mode {plan, approve?}`：把计划写入 `.blora/plan.md`，恢复之前的权限模式。
  - 交互式 run 会走审批；拒绝时计划不落盘。
  - 非交互 run 必须显式 `approve=true`。

## 运行时行为

- `sessions.permission_mode` / `session_settings` 持久化会话模式；下一次 run 不带显式 `permission_mode` 时沿用。
- 工具声明按当前模式过滤：Plan 下只下发只读工具、`update_plan`、`ask_user` 和 `exit_plan_mode`；MCP/插件工具不下发。
- 每次工具调用都会按当前会话模式重建 `Policy`，所以进入 Plan 后当前 run 的写入立即被拒绝，不必等到下一次 run。
- 事件：`permission.changed {from,to,source}` 和 `plan.mode.changed {entered,reason}`。
- `POST /api/sessions/:id/permission` 可由客户端直接切换会话模式。

## 写入限制

Plan 模式只允许写 `.blora/plan.md`（`Policy::file_write_path`）。shell 的只读命令仍可执行，改动/危险命令和联网被拒绝。

## 与 `update_plan` 的区别

`update_plan` 是工作清单，任何模式都可用；`enter_plan_mode` 改变权限与工具集，用于需要用户批准的实施计划。
