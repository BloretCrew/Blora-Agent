# ADR-005: Agent SDK boundary

## Status

Accepted

## Decision

Blora Agent exposes an embeddable SDK in two forms:

1. A Rust facade crate, `blora-sdk`, which runs the existing `blora-runtime`
   Agent Loop in-process.
2. A stdio JSON-RPC protocol, implemented by `blora-sdk-protocol` and exposed
   as `blora sdk --stdio`, for Node/TypeScript and Python clients.

The Agent Loop is not rewritten or extracted in this phase. The runtime gains
explicit configuration and injection seams (`RuntimeConfig`, `ProviderSource`,
`KernelTool`, per-session event replay, approval/question timeout) so that SDK
embeddings do not inherit CLI/TUI/Web-server behavior.

## Why

- The existing `Runtime::run()` already implements event persistence,
  compaction, retries, tool dispatch, approvals, and subagents. Rewriting it
  would fork behavior and discard the current regression suite.
- Rust hosts need an in-process library; Node and Python hosts need a process
  boundary that avoids static linking and GPL implications.
- A stdio sidecar gives per-tenant isolation without ports, authentication, or
  a daemon.

## Consequences

- `RuntimeConfig::legacy()` keeps all existing CLI/TUI/server behavior.
- SDK profiles set `inherit_env = false` and must not read `BLORA_*`,
  `~/.blora`, or `providers.json` implicitly.
- The web/chat profile declares no filesystem or shell tools and uses
  in-memory storage by default.
- The SDK is licensed GPL-3.0-or-later. Hosts that cannot accept GPL linking
  should use the process-boundary protocol.
