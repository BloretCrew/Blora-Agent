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
- `execute_code` is a general execution entry: it requires `Policy::code_exec` (Plan denies, Ask confirms, AutoEdit/Yolo allow), runs Python/JavaScript in the workspace, and only exposes an allowlisted tool bridge. Recursive execution, subagents, user questions, and team/goal tools are blocked.
- Browser automation only opens http(s), refuses localhost/private addresses by default, honors `BLORA_BROWSER_ALLOWED_DOMAINS`, and is gated by `Policy::browser_navigate` plus `BLORA_NETWORK`.
- `computer` is disabled unless `BLORA_COMPUTER_USE=1`, requires `Policy::computer_use`, and is only offered in Agent mode. Screenshots and actions are written to the session's attachment/audit surfaces.
- MCP servers are configured by workspace/global JSON; `${ENV}` expansion is supported, tokens are never written to events, and HTTP transport keeps `Mcp-Session-Id` in memory only.
- Media tools (`edit_image`, `generate_video`, `text_to_speech`, `vision_analyze`) require `Policy::media_generate` and a configured endpoint; generated files are stored as attachments.
- Provider credentials never enter the event log; the Web UI only reaches the workspace through the HTTP API.
