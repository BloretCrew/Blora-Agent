# Security operations

- 默认 fail-closed。
- 事件中的密钥按 visibility 和后续脱敏策略处理；当前 mock 不写入密钥。
- 数据目录权限由用户负责；后续应在创建 `~/.blora` 时限制为 0700。
