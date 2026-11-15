# Security operations

- 默认 fail-closed。
- 事件中的密钥按 visibility 和后续脱敏策略处理；当前 mock 不写入密钥。
- 创建 `BLORA_HOME`（默认 `~/.blora`）时在 Unix 上设置目录权限 `0700`。
- 单用户 Web 只监听回环地址，并校验 `Host` 必须是回环（`127.0.0.0/8`、`::1`、`localhost`）；远程访问必须使用 `gateway` + `blt_` 令牌。这样浏览器里的网页无法通过 DNS rebinding 驱动本机 Agent。
- Web 只通过 HTTP API 读工作区文件，不能直接访问磁盘或密钥。
