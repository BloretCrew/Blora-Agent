# Security

Blora Agent executes model-requested work against a local workspace. Default policy is fail-closed.

## Report

Do not file public issues for unreleased tool execution or sandbox bypasses. Contact the maintainers privately.

## Current scope

- Four permission modes: `plan` (read-only; only `.blora/plan.md` is writable), `ask` (default: writes and mutating commands confirm), `auto-edit` (workspace edits allowed, mutating commands confirm), `yolo` (`--yes`). Read-only inspection commands never prompt outside `ask`.
- Dangerous shell commands (recursive delete, privilege escalation, disk tools, force push, `| sh`, …) always confirm, even in `yolo`; `plan` denies them outright. Each `&&` / `|` / `;` segment is classified separately.
- Network commands are denied unless `BLORA_NETWORK=1`; secret-looking paths and destructive git commands ask unless in `yolo`.
- Every tool call is bounded: shell timeout (30s default, 600s max), output caps, wall-clock budget per run, token budget per session.
- Hooks in `BLORA_HOOKS_DIR` can block or force confirmation of any tool call; a failing hook is recorded as degraded rather than silently allowing.
- Provider credentials never enter the event log; the Web UI only reaches the workspace through the HTTP API.
