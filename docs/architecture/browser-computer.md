# 浏览器与桌面自动化

## Browser

`browser` 工具通过 Chrome DevTools Protocol 驱动本机 headless Chrome/Chromium，不依赖外部 MCP 或 Node 包。

### 动作

| 动作 | 参数 | 说明 |
|---|---|---|
| `open` / `navigate` | `url` | 导航并等待 `document.readyState` |
| `snapshot` | | 返回标题、URL、正文摘要和可交互元素（带 `ref`） |
| `click` | `ref` 或 `selector` | 滚动到元素并点击 |
| `type` | `ref`/`selector`, `text` | 聚焦并写入值，触发 input/change |
| `press` | `key` | 在 activeElement 上派发 keydown/keyup |
| `scroll` | `x`, `y` | `window.scrollBy` |
| `screenshot` | `full_page?` | 返回 PNG，走附件链路 |
| `evaluate` | `expression` | 执行任意 JS 并返回结果 |
| `console` | | 返回页面捕获的 console 消息 |
| `dialog` | `accept?`, `prompt_text?` | 处理 alert/confirm/prompt 并返回内容 |
| `cdp` | `method`, `params?` | 发送原始 CDP 命令 |
| `wait_for` | `selector`/`ref`, `timeout_seconds` | 轮询元素出现 |
| `back` / `forward` | | history 导航 |
| `list` | | 当前标题、URL、正文摘要 |
| `close` | | 结束浏览器进程 |

### 启动

按顺序查找 `BLORA_BROWSER`、`google-chrome`、`chromium`、`chromium-browser`、`microsoft-edge`。使用
`--headless=new --remote-debugging-port=0 --user-data-dir=.blora/browser/profile`。
容器/root 环境可设置 `BLORA_BROWSER_NO_SANDBOX=1`。

### 策略

- 只允许 http/https。
- 默认拒绝 localhost、回环、私网、链路本地地址；`BLORA_BROWSER_ALLOW_PRIVATE=1` 可关闭。
- `BLORA_BROWSER_ALLOWED_DOMAINS=a.com,b.com` 限定域名（含子域）。
- 导航还受 `Policy::browser_navigate` 和 `BLORA_NETWORK` 约束：Plan 拒绝，Ask 审批，AutoEdit/Yolo 放行。

### 截图

截图写入 `.blora/attachments/<session>/`，`tool.output` 只记录引用；支持视觉的模型会收到图像块。

## Computer use

`computer` 工具通过平台命令控制桌面：

- Linux/X11：`xdotool` 负责点击、移动、输入、按键、滚动；Wayland 下移动/点击/输入回退到 `ydotool`。截图使用 `grim`、`scrot` 或 ImageMagick `import`。
- macOS：`cliclick` 负责移动/点击，`osascript` 负责输入与常用按键，`screencapture` 截图。
- Windows：PowerShell + `System.Windows.Forms` 负责鼠标、键盘和截图。

动作：`screenshot`、`click`、`move`、`type`、`key`、`scroll`、`wait`。坐标从 0 开始，负值拒绝。

默认关闭：需要 `BLORA_COMPUTER_USE=1`；`BLORA_COMPUTER_BACKEND=fake` 用于无显示环境的测试。每次调用仍受
`Policy::computer_use` 约束（Plan 拒绝，Ask 审批，AutoEdit/Yolo 放行）。截图写入附件链路，返回图像内容块。

computer use 只在 Agent 模式提供，因为它会直接操作真实桌面。Wayland/macOS/Windows 后端需要在对应系统上实测，
并确保辅助功能/输入权限已授予。
