# 领域模型

| 实体 | 含义 |
|---|---|
| Workspace | 本地工作区路径与能力边界 |
| Session | 可恢复的长期会话 |
| Run | 一次模型驱动执行 |
| Turn | Run 内的一轮模型调用 |
| Event | 追加写的规范化事实 |
| Task | 可后台运行的工作单元 |
| Agent / Subagent | 带预算与权限的执行者 |
| Approval | 权限询问与决定 |

标识符带前缀：`ses_`、`run_`、`trn_`、`evt_`、`tsk_`、`agt_`、`wsp_`、`apr_`、`art_`、`chk_`。使用 UUID v7，按时间可排序。
