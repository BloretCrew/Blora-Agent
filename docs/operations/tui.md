# TUI 快捷键

界面是一块留白的对话画布：顶栏只写 `blora · mode · 会话`，消息用左侧色条区分你 / 助手，底栏是 `❯` 输入、一行状态、一行快捷键。没有围满全屏的边框。

输入 `/` 后，prompt 上方弹出命令列表（名称、参数提示、说明）。边打边过滤（前缀 / 子串 / 子序列），`Tab` 补全，`↑` `↓` 选择，`Enter` 执行。需要参数的命令（提示以 `<` 开头）在补全后停在 `/name `，补完参数再回车。`Esc` 先关补全或结果面板，再退出。`Ctrl+P` 在空 prompt 上打开 `/` 菜单。`NO_COLOR` 时退回终端原色。

| 键 | 作用 |
|---|---|
| Enter | 发送当前输入；以 `/` 开头则补全或执行选中的斜杠命令；运行中则作为插话排队，下一轮送达 |
| Tab | 补全选中的斜杠命令 |
| ↑ / ↓ | 浏览本次启动期间已发送的输入历史；↓ 回到最新位置时恢复草稿。斜杠菜单与弹窗中用于选择项目 |
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

## Git 文件选择与差异

Git 窗口默认打开“操作”页。单击文件行选中并查看差异，在同一文件行上 500 毫秒内双击切换勾选；`↑` / `↓` 选择文件，空格切换勾选。右侧显示所选文件的完整统一差异，涵盖已暂存和未暂存变更、新文件与删除文件；二进制文件显示 Git 的差异说明。

鼠标停在差异栏上滚动，或使用 `PageUp` / `PageDown` 浏览差异；文件列表与差异各自保留滚动位置。切换文件会重置差异位置。较窄且高度足够的终端使用上下布局，极小窗口可点击绿色控件展开。

## 首次使用引导

首次启动进入欢迎、账号、完成三个步骤。`Tab` / 方向键选择操作，`Enter` 确认，鼠标可直接点击操作行。点击登录后显示设备码与授权地址，可打开浏览器、复制或取消。`Esc` 先取消正在进行的登录，再关闭引导；`Ctrl+C` / `Ctrl+Q` 退出程序。小终端会保留当前选中的操作，方向键可查看其他选项。

常规尺寸使用与设置、供应商窗口相同的圆角边框、标题栏和主题色。标题栏红色控件关闭引导，黄色控件最小化，绿色控件切换展开；最小化后按 `Enter` 恢复。欢迎页说明工作模式，账号页集中显示授权信息，完成页列出账号和配置保留情况。`PageUp` / `PageDown` 可滚动说明和较长授权地址；小终端使用紧凑布局。

选择“稍后设置”保留已有供应商配置，点击“开始使用”保存引导版本。`/login` 打开账号步骤，`/onboarding` 重开完整引导；关闭引导后保留原会话和输入状态。

## 斜杠命令

会话：`/new` `/sessions` `/goto` `/status` `/context` `/id` `/pwd` `/fork` `/resume` `/archive` `/export` `/timeline` `/code` `/work` `/agent` `/plan` `/mode` `/next` `/prev` `/reload` `/quit`

运行：`/compact` `/checkpoint` `/cancel` `/steer` `/yes` `/no` `/permissions` `/approvals` `/model` `/provider` `/exec` `/worktree`

工作：`/tasks` `/task` `/cron` `/loop` `/pause` `/unpause` `/cancel-task` `/pump` `/agents` `/search` `/find` `/clear` `/tools` `/copy`

记忆：`/memory` `/remember` `/recall` `/forget` `/distill`

扩展：`/plugins` `/marketplace` `/install` `/uninstall` `/skills` `/mcp` `/hooks` `/init`

仓库：`/git` `/diff` `/log` `/files` `/read` `/artifacts`

系统：`/help` `/keymap` `/doctor` `/users` `/web` `/usage`
