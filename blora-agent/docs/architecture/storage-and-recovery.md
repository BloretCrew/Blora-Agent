# 存储与恢复

SQLite 是第一版唯一存储。

- `events` 追加写，触发器禁止 UPDATE/DELETE。
- `sessions` / `runs` 是派生索引，projection 始终从事件重建。
- 崩溃恢复：重新打开数据库，按 `sequence` 重放事件。
- 大产物未来进 blob 目录，库中只存哈希与元数据。

环境变量 `BLORA_HOME` 覆盖数据目录，默认 `~/.blora`。
