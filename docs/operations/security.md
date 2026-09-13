# Security operations

- 默认 fail-closed。
- 事件中的密钥按 visibility 和后续脱敏策略处理；当前 mock 不写入密钥。
- 创建 `BLORA_HOME`（默认 `~/.blora`）时在 Unix 上设置目录权限 `0700`。
- Web 只通过 HTTP API 读工作区文件，不能直接访问磁盘或密钥。
