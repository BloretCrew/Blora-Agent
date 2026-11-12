# 工具目录

`ToolRegistry::specs_for(mode)` 决定每个模式实际下发的工具。只读工具可并行执行，也可用于只读子代理。

图例：C=Code，W=Work，A=Agent，I=Imagine；R=只读。

## 文件与搜索

| 工具 | 模式 | R | 说明 |
|---|---|---|---|
| `read_file` | C/W/A | ✓ | 读文本；图片文件返回图像内容 |
| `view_image` | C/W/A | ✓ | 显式查看图片，`detail` 可选 |
| `write_file` | C/W/A | | 新建或整体替换 |
| `list_dir` | C/W/A | ✓ | 列目录 |
| `search` | C/W/A | ✓ | 正则逐行搜索，include/max_results |
| `grep` | C/W/A | ✓ | 上下文、smart case、multiline、类型过滤、三种输出模式 |
| `glob` | C/W/A | ✓ | 按文件名模式查找 |
| `apply_patch` | C/W/A | | 唯一匹配替换 |
| `notebook_edit` | C/A | | 读写 .ipynb 单元格 |

## 执行与 Git

| 工具 | 模式 | R | 说明 |
|---|---|---|---|
| `shell` | C/W/A | | 前台/后台/PTY，可指定 cwd |
| `process` | C/W/A | | list/output/wait/stdin/kill |
| `execute_code` | C/A | | Python/JS 通过桥调用白名单工具 |
| `git_status` | C/W/A | ✓ | |
| `git_diff` | C/W/A | ✓ | |
| `git_log` | C/W/A | ✓ | |
| `git_branch` | C/W/A | ✓ | |
| `git_worktree` | C/W/A | | list/add/remove |

## 代码智能

| 工具 | 模式 | R | 说明 |
|---|---|---|---|
| `lsp` | C/A | ✓ | diagnostics/hover/definition/references/symbols |

## 联网与文档

| 工具 | 模式 | R | 说明 |
|---|---|---|---|
| `web_search` | C/W/A | ✓ | 需要网络策略 |
| `web_fetch` | C/W/A | ✓ | HTML 转文本 |
| `read_document` | C/W/A | ✓ | PDF/DOCX/XLSX/PPTX |

## 规划、委派与提问

| 工具 | 模式 | R | 说明 |
|---|---|---|---|
| `update_plan` | C/W/A | ✓ | 工作清单 |
| `enter_plan_mode` | C/W/A | | 会话级 plan mode |
| `exit_plan_mode` | C/W/A | | 写计划并恢复权限 |
| `delegate` | C/W/A | | 同步子代理 |
| `subagent` | C/W/A | | 后台 spawn/list/send/wait/close/status |
| `handoff` | C/W/A | ✓ | 子代理返回报告 |
| `ask_user` | C/W/A | ✓ | 结构化提问 |

## 记忆与技能

| 工具 | 模式 | R | 说明 |
|---|---|---|---|
| `remember` | C/W/A | | 写 `.blora/memory.md` |
| `recall` | C/W/A | ✓ | 读记忆 |
| `forget` | C/W/A | | 删除记忆 |
| `skill` | C/W/A | ✓ | 按名加载技能全文 |
| `skill_manage` | A | | 创建/更新/删除技能 |
| `session_search` | A | ✓ | FTS5 历史检索 |

## 工具发现与 MCP

| 工具 | 模式 | R | 说明 |
|---|---|---|---|
| `tool_search` | C/W/A | ✓ | 搜索并启用延迟工具 |
| `mcp_list_resources` | C/W/A | ✓ | |
| `mcp_read_resource` | C/W/A | ✓ | |
| `mcp_list_prompts` | C/W/A | ✓ | |
| `mcp_get_prompt` | C/W/A | ✓ | |

## 协作与编排

| 工具 | 模式 | R | 说明 |
|---|---|---|---|
| `goal` | W/A | | propose/get/list/update/approve/reject |
| `team` | W/A | | create/list/board/dispatch/delete |
| `team_task` | W/A | | Kanban 任务 CRUD、claim、评审、心跳、评论 |
| `workflow` | W/A | | create/start/status/advance/cancel/delete |
| `artifact` | C/W/A | | create/read/update/delete/list/present |
| `structured_output` | C/W/A | ✓ | 校验并记录结构化输出 |
| `schedule` | W/A | | 定时任务完整 CRUD |

## 媒体与自动化

| 工具 | 模式 | R | 说明 |
|---|---|---|---|
| `generate_image` | I/W/A | | 图片生成 |
| `edit_image` | I/W/A | | 图片编辑 |
| `generate_video` | I/W/A | | 视频生成 |
| `text_to_speech` | I/W/A | | TTS |
| `vision_analyze` | C/W/A | ✓ | 独立视觉模型分析 |
| `browser` | W/A | | 原生 CDP 浏览器 |
| `computer` | A | | 桌面控制，默认关闭 |

## 动态工具

- MCP：`mcp__{server}__{tool}`，单服务器兼容 `mcp__{tool}`。
- 插件：`plugin__{name}`。
- MCP/插件默认由 `tool_search` 延迟启用；`BLORA_TOOL_SEARCH=0` 恢复一次性声明。
