# ADR-003: Event log as source of truth

## Status

Accepted

## Decision

Session 真相是 append-only event log。TUI、Web、搜索、审计都从事件或 projection 读取。

## Why

恢复、回放、取消和多界面一致性都依赖不可变事实流，而不是可变 `messages.json`。
