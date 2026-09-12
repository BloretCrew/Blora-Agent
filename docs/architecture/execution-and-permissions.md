# 执行与权限

第一版只定义原则，尚未实现真实执行后端。

- 所有工具通过 `ExecutionBackend`，禁止在业务代码里直接 `std::process`。
- 首个后端是 Local，随后 Worktree，再后 Sandbox / Container / Remote。
- 权限结果只有 `allow`、`deny`、`ask`。默认 fail-closed。
- 审批事件必须记录 capability、摘要、决定与 policy version。
