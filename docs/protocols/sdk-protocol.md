# SDK Protocol

`blora sdk --stdio` speaks newline-delimited JSON-RPC 2.0 on stdin/stdout.
One JSON object per line; empty lines are ignored.

The protocol is transport-neutral. The first transport is stdio; a WebSocket
transport may reuse the same messages later.

## Versions

- `protocolVersion`: `"1.0"` (negotiated by `initialize`).
- Event schema: `EventEnvelope.schema_version`, currently `1`.

Clients must ignore unknown event types and unknown fields. New fields are
additive and carry `#[serde(default)]` semantics.

## Client requests

| Method | Params | Result |
|---|---|---|
| `initialize` | `protocolVersion`, `profile`, `persistence`, `provider`, `workspace`, `features`, `hostTools`, `callbacks` | `serverInfo`, `capabilities` |
| `agent.shutdown` | `cancelRunning`, `timeoutMs` | `{}` |
| `session.create` | `title?`, `mode?`, `workspace?`, `permission?` | `sessionId`, `sequence` |
| `session.resume` / `session.fork` / `session.archive` | `sessionId` | `sessionId` |
| `session.list` | `limit?`, `cursor?` | `sessions[]` |
| `session.project` | `sessionId` | projection |
| `session.events` | `sessionId`, `afterSequence?`, `limit?` | `events[]`, `nextSequence` |
| `session.subscribe` / `session.unsubscribe` | `sessionId`, `afterSequence?` | `sequence` |
| `run.start` | `sessionId`, `prompt`, `attachments?`, `outputSchema?`, `maxTurns?`, `model?`, `permission?` | `runId`, `startSequence` |
| `run.wait` | `runId`, `timeoutMs?` | outcome |
| `run.cancel` | `runId` | `{}` |
| `run.steer` | `sessionId`, `message` | `{}` |
| `approval.resolve` | `approvalId`, `decision` | `{}` |
| `input.answer` | `sessionId`, `requestId`, `answer` | `{}` |
| `host_tools.register` | `tools[]` | `registered[]` |

## Server notifications

- `event`: a full canonical `EventEnvelope` in `params.event`.
- `approval.requested`: `approvalId`, `sessionId`, `runId`, `capability`,
  `summary`, `policyVersion`, `timeoutMs`.
- `input.requested`: `sessionId`, `requestId`, `question`, `options`,
  `questions`, `timeoutMs`.
- `stream.lagged`: `sessionId`, `lastDeliveredSequence`, `latestSequence`.
  The client should call `session.events` to backfill.
- `run.finished`: `runId`, `status`, `finalText?`, `usage?`, `error?`.

## Server requests

- `host_tool.invoke`: JSON-RPC request whose `id` is the tool `callId`.
  Params: `callId`, `tool`, `arguments`, `sessionId`, `runId`, `timeoutMs`.
  Result: `{ parts, metadata?, is_error? }`.
- `host_tool.cancelled`: notification with `callId` and `reason`.

## Errors

Standard JSON-RPC codes cover parsing and method errors. Business errors use
`-32001..-32014` and carry `data.kind`: `invalid_config`, `session_not_found`,
`session_busy`, `provider_error`, `policy_denied`, `approval_required`,
`unsupported`, `cancelled`, `host_tool_failed`, `host_tool_timeout`,
`storage_error`, `protocol_version_unsupported`, `invalid_state`.

## Security

- stdio `initialize` is written by the host into the child process stdin.
  Keys must never be echoed, logged, or persisted in events.
- `host_tools.register` must complete before the first `run.start`; tool names
  match `^[a-zA-Z0-9_-]{1,64}$` and must not collide with built-ins, `mcp__*`,
  or `plugin__*`.
- Line limit 8 MiB; prompt limit 1 MiB; tool result text limit 1 MiB.
