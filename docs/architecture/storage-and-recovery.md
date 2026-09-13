# 存储与恢复

SQLite 是第一版唯一存储。

- `events` 追加写，触发器禁止 UPDATE/DELETE。
- `sessions` / `runs` 是派生索引，projection 始终从事件重建。
- 崩溃恢复：重新打开数据库，按 `sequence` 重放事件。
- 压缩摘要等小产物存在 `artifacts` 表；大文件仍建议只存路径/哈希。
- Unix 上数据目录权限为 `0700`。
- `blora backup` / `blora restore` 复制 SQLite 与 WAL。

环境变量 `BLORA_HOME` 覆盖数据目录，默认 `~/.blora`。
