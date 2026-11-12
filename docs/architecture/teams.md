# 团队任务板

`team` / `team_task` 提供工作区级 Kanban。团队按 workspace 隔离，任务可被后台子代理认领。

## 看板

`team create` 会自动创建六列：Backlog、Ready、In Progress、Review、Blocked、Done。`team board` 按列输出任务 JSON。

状态与列的对应通过 `status` 推导（例如 `in_progress` ↔ In Progress）。任务记录同时保存 `column_id`，便于 UI 拖拽。

## 任务字段

- `title`、`description`、`priority`
- `status`：`backlog`、`ready`、`in_progress`、`review`、`blocked`、`done`、`cancelled`
- `assignee`：后台子代理 id
- `depends_on`：前置任务 id 数组；未全部 `done` 时 `claim` 会拒绝
- `review_state`：`requested`、`changes_requested`、`approved`
- `heartbeat_at`：认领者心跳时间
- 评论、附件、链接

## 动作

`team`：create / list / board / dispatch / delete。

`team_task`：

- 生命周期：create / get / list / claim / complete / block / unblock / update / delete
- 评审：request_review / request_changes / approve
- 协作：comment / comments / attach / attachments / link / links
- 心跳：heartbeat

`team dispatch` 会把 `ready` 任务派给后台 worker 子代理，并自动 claim。`depends_on` 未完成的任务不会被派发或认领。

## 事件

`team.created`、`team.task.updated` 记录团队和任务状态变化；任务评论/附件/链接保存在 SQLite。

## 存储

- `teams`、`team_columns`、`team_tasks`
- `team_task_comments`、`team_task_attachments`、`team_task_links`
- 当前存储版本 19。
