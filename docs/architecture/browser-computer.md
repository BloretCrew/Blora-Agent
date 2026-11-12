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

桌面控制尚未实现。计划中的接口是 `computer { action, x, y, text, key, screen }`，平台后端使用
X11（xdotool/scrot）、Wayland（ydotool/grim）、macOS（osascript/screencapture）和 Windows PowerShell。
默认关闭，需要显式开启并审批。
