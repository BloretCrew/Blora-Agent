#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$ROOT"
cargo build -p blora-cli --quiet
BLORA_BIN="$ROOT/target/debug/blora" node --test sdks/node/test/*.test.js
(cd npm/blora-agent && npm test && npm pack --dry-run)
