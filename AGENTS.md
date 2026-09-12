# Blora Agent contributor notes

- This is a from-scratch GPL-3.0-or-later project.
- Do not fork or copy other harness implementations.
- Canonical events are the source of truth for sessions.
- `blora-types` must stay free of IO and provider types.
- Unknown event types must not break replay.
- Prefer adding a crate over growing `blora-runtime` into a monolith.
- Web UI, when added, must use published `@bloret-crew/blora-design` packages only.
