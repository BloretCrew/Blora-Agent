# ADR-004: SQLite for MVP storage

## Status

Accepted

## Decision

本地状态使用 SQLite。事件表禁止更新和删除。

## Why

零运维、单文件、事务和 WAL 足够覆盖单用户恢复。多用户阶段再抽象 PostgreSQL。
