# 工作流

`workflow` 是一套显式的多步流程引擎。定义由模型或用户给出，运行时保存定义和每次运行的状态；模型在每一步执行后用 `advance` 推进。

## 定义

```json
{
  "name": "release",
  "definition": {
    "steps": [
      {"type": "tool", "tool": "shell", "arguments": {"command": "cargo test"}},
      {"type": "subagent", "role": "review", "prompt": "Review the diff"},
      {"type": "checkpoint", "note": "ready to ship"}
    ]
  }
}
```

也可以直接传数组。步骤是任意 JSON；运行时只负责顺序和状态，不自动执行步骤内容，模型根据 `status` 返回的当前步骤决定下一步。

`background=true` 时，运行时会按顺序执行：

- `{"type":"tool","tool":"read_file","arguments":{...}}`：调用内置工具。
- `{"type":"subagent","role":"review","prompt":"..."}`：同步运行子代理并记录报告。
- `{"type":"checkpoint"}`：无操作，用于标记阶段。
- `{"type":"ask","prompt":"..."}`：后台不能提问，记录为跳过。

任一步失败会停止并把 run 标为 `failed`。

## 工具

`workflow { action, workflow_id?, name?, definition?, result? }`

- `create`：保存定义（至少一步）。
- `start`：创建 run，`current_step=0`，返回第一步；`background=true` 时在后台自动执行步骤。
- `status`：返回工作流状态、当前步骤序号和内容。
- `list`：列出会话内的工作流。
- `advance`：推进到下一步；可附带 `result` 记录到 `state_json`。最后一步后状态变为 `completed`。
- `cancel`：把最新 run 标记为 `cancelled`。
- `delete`：删除工作流及其 runs。

## 存储与事件

- `workflows` 表保存定义与状态；`workflow_runs` 保存每次运行的 `current_step`、`state_json`、开始/结束时间。
- 事件 `workflow.updated {workflow_id, status, step}` 随每次状态变化写入。

## 与目标和团队任务的区别

- `goal`：长期目标与进度。
- `team_task`：工作区级 Kanban，可派发给后台子代理。
- `workflow`：一个有明确步骤顺序的执行计划，适合需要人工/模型逐步推进的多阶段任务。
