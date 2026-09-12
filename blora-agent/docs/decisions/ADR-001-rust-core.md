# ADR-001: Rust Core

## Status

Accepted

## Decision

Blora Agent 的 Runtime、事件、存储、执行和 TUI 使用 Rust。Web UI 使用 TypeScript。

## Why

进程、PTY、取消、并发任务、单文件分发和终端控制更适合原生运行时。Web 仍属于浏览器生态。
