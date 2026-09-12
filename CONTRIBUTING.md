# Contributing to Blora Agent

Blora Agent is free software under the GNU GPL v3 or later.

## Rules

1. Do not copy source from other harness projects into this tree.
2. New code belongs in `blora-agent/` crates and docs.
3. Event log is append-only. Corrections are new events.
4. Domain types must not depend on HTTP, SQLite, or a provider SDK.
5. Run `scripts/check.sh` before sending changes.

## License of contributions

By contributing, you license your work under GPL-3.0-or-later, the same terms as the rest of the project.
