# TUI 快捷键

界面是一块留白的对话画布：顶栏只写 `blora · mode · 会话`，消息用左侧色条区分你 / 助手，底栏是 `❯` 输入、一行状态、一行快捷键。没有围满全屏的边框。

输入 `/` 后，prompt 上方弹出命令列表（名称、参数提示、说明）。边打边过滤（前缀 / 子串 / 子序列），`Tab` 补全，`↑` `↓` 选择，`Enter` 执行。需要参数的命令（提示以 `<` 开头）在补全后停在 `/name `，补完参数再回车。`Esc` 先关补全或结果面板，再退出。`Ctrl+P` 在空 prompt 上打开 `/` 菜单。`NO_COLOR` 时退回终端原色。

| 键 | 作用 |
|---|---|
| Enter | 发送当前输入；以 `/` 开头则补全或执行选中的斜杠命令 |
| Tab | 补全选中的斜杠命令 |
| ↑ / ↓ | 在斜杠补全列表中移动 |
| Ctrl+P | 打开斜杠菜单（prompt 为空时） |
| Esc | 关闭斜杠菜单或结果面板；否则退出 |
| Backspace | 删除输入 |
| `[` / Left | 上一个会话（输入为空时） |
| `]` / Right | 下一个会话（输入为空时） |
| PageUp / PageDown | 滚动 transcript |
| Home / End | 滚到最早 / 最新 |
| y / n | 批准或拒绝当前审批（输入为空时） |
| Ctrl+N | 新建会话 |
| Ctrl+C | 请求取消并退出 |
| 滚轮 | 滚动 transcript；在斜杠列表上则移动选中项 |
| 单击 | 执行斜杠命令、审批、切换会话、底栏动作 |

鼠标（终端需支持 SGR 鼠标）：滚轮滚动对话；斜杠列表悬停高亮、单击执行（需要参数则补全）；点 `y allow` / `n deny` 审批；顶栏 `‹ ›` 切换会话；点 `running` 取消当前 run；点 `ask`/`yolo` 切换自动批准；底栏快捷键可点。点斜杠菜单或结果面板以外的区域会关闭它们。

`/help`、`/git`、`/files` 等较长输出显示在 prompt 上方的结果面板，`Esc` 或点击关闭。

## 斜杠命令

会话：`/new` `/sessions` `/goto` `/status` `/context` `/id` `/pwd` `/fork` `/resume` `/archive` `/export` `/timeline` `/code` `/work` `/agent` `/plan` `/mode` `/next` `/prev` `/reload` `/quit`

运行：`/compact` `/checkpoint` `/cancel` `/yes` `/no` `/permissions` `/approvals` `/model` `/provider` `/exec` `/worktree`

工作：`/tasks` `/task` `/cron` `/loop` `/pause` `/unpause` `/cancel-task` `/pump` `/agents` `/search` `/find` `/clear` `/tools` `/copy`

记忆：`/memory` `/remember` `/recall` `/forget` `/distill`

扩展：`/plugins` `/marketplace` `/install` `/uninstall` `/skills` `/mcp` `/hooks` `/init`

仓库：`/git` `/diff` `/log` `/files` `/read` `/artifacts`

系统：`/help` `/keymap` `/doctor` `/users` `/web` `/usage`
