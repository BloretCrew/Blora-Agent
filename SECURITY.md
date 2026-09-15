# Security

Blora Agent executes model-requested work against a local workspace. Default policy is fail-closed.

## Report

Do not file public issues for unreleased tool execution or sandbox bypasses. Contact the maintainers privately.

## Current scope

- Writes, shell commands, worktree creation, and process kills require approval unless `--yes` / auto-approve is set.
- Network commands are denied unless `BLORA_NETWORK=1`; secret-looking paths and destructive git commands always ask.
- Every tool call is bounded: shell timeout (30s default, 600s max), output caps, wall-clock budget per run, token budget per session.
- Hooks in `BLORA_HOOKS_DIR` can block or force confirmation of any tool call; a failing hook is recorded as degraded rather than silently allowing.
- Provider credentials never enter the event log; the Web UI only reaches the workspace through the HTTP API.
