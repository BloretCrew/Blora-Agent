# Security operations

- 默认 fail-closed。
- 事件中的密钥按 visibility 和后续脱敏策略处理；当前 mock 不写入密钥。
- 创建 `BLORA_HOME`（默认 `~/.blora`）时在 Unix 上设置目录权限 `0700`。
- 单用户 Web 只监听回环地址，并校验 `Host` 必须是回环（`127.0.0.0/8`、`::1`、`localhost`）；远程访问必须使用 `gateway` + `blt_` 令牌。这样浏览器里的网页无法通过 DNS rebinding 驱动本机 Agent。
- Web 只通过 HTTP API 读工作区文件，不能直接访问磁盘或密钥。

## 依赖审计（待办）

`cargo deny check advisories` 目前报告以下项，CI 暂时只 gate `licenses bans sources`：

- 可直接升级（同一 semver 线）：`rustls` < 0.23.46、`rustls-webpki` < 0.103.16、`time` < 0.3.47、`rustix` < 1.1.6。
- 需要跨版本升级：`libsqlite3-sys` < 0.39（随 `rusqlite` 升级）、`quick-xml` < 0.41。
- 无修复、需评估替换：`bincode` 1.3（来自 `syntect`）、`paste`。

升级完成后把 `.github/workflows/ci.yml` 的 deny `arguments` 改回 `check`，让 advisories 重新进入 gate。
