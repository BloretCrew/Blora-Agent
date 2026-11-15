#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$ROOT"
cargo build -p blora-cli --quiet
BLORA_BIN="$ROOT/target/debug/blora" PYTHONPATH="$ROOT/sdks/python" \
  python3 -m unittest discover -s sdks/python/tests -v
python3 -m pip wheel --no-deps --wheel-dir "$ROOT/target/python-wheel" "$ROOT/sdks/python"
