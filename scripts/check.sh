#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p blora-cli
if command -v node >/dev/null 2>&1; then
    ./scripts/check-sdk-node.sh
fi
if command -v python3 >/dev/null 2>&1; then
    ./scripts/check-sdk-python.sh
fi
