# 进程边界

## 第一阶段

`blora` CLI 与 Runtime、SQLite 运行在同一进程。数据文件默认 `~/.blora/state.sqlite`。

## 后续

```text
blora tui     内嵌 Core
blora serve   本地 daemon + HTTP/SSE
blora web     TypeScript UI，只连接 API
```

Web 不得直接访问工作区文件、密钥或 Provider。工具执行始终发生在 Runtime 进程（或未来的 Execution Backend 进程）中。
