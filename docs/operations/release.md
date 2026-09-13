# Release

当前版本 `0.1.0`。发布物必须包含对应源码，满足 GPL-3.0-or-later。二进制分发需提供 Corresponding Source 获取方式。

升级：打开已有 `state.sqlite` 时会按 `schema_migrations` 补齐新表（当前到 v6：cron、artifacts、usage、memories）。备份先运行 `blora backup`。
