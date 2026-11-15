# Release

当前测试版为 `27.0.0-beta`（Python 包版本按 PEP 440 标记为 `27.0.0b0`）。GitHub Release 由 `v*` 标签触发，提供 Linux、macOS 和 Windows CLI 二进制；npm 包 `@bloret-crew/blora-agent@27.0.0-beta` 安装时下载对应版本的 Release 资产。Python SDK 版本随本仓库发布，wheel 可从 `sdks/python` 构建。

发布物必须包含对应源码，满足 GPL-3.0-or-later。二进制分发需提供 Corresponding Source 获取方式。

升级：打开已有 `state.sqlite` 时会按 `schema_migrations` 补齐数据库结构；备份先运行 `blora backup`。
