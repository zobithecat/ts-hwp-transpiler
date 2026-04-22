#!/usr/bin/env bash
# Semi-automatic round-trip runner. Re-runs hwp-roundtrip on every Rust
# source change under crates/codec. Non-zero exit on divergence.
#
# Usage:
#   ./scripts/watch-roundtrip.sh <fixture.hwp>
#
# Prefers watchexec; falls back to cargo-watch.

set -eu

FIXTURE="${1:-crates/codec/tests/fixtures/blank.hwp}"

if command -v watchexec >/dev/null 2>&1; then
  exec watchexec \
    --exts rs \
    --watch crates/codec/src \
    --watch crates/core/src \
    --clear \
    -- cargo run -q -p hwp-transpiler-codec --bin hwp-roundtrip -- "$FIXTURE"
elif command -v cargo-watch >/dev/null 2>&1; then
  exec cargo watch \
    -w crates/codec/src \
    -w crates/core/src \
    -c \
    -s "cargo run -q -p hwp-transpiler-codec --bin hwp-roundtrip -- '$FIXTURE'"
else
  echo "install watchexec or cargo-watch:" >&2
  echo "  brew install watchexec   # or   cargo install cargo-watch" >&2
  exit 127
fi
