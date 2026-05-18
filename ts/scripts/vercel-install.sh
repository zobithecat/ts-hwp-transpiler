#!/usr/bin/env bash
set -euo pipefail

# Install rustup with no default toolchain — rust-toolchain.toml at repo
# root pins channel 1.85 + wasm32-unknown-unknown target, so the first
# cargo invocation will fetch the right toolchain automatically.
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --profile minimal --default-toolchain none

# Vercel pre-sets CARGO_HOME=/rust so the env file lands at /rust/env
# (not ~/.cargo/env). Fall back to the standard location for local
# `bash scripts/vercel-install.sh` runs.
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
. "${CARGO_HOME}/env"

# Prebuilt wasm-pack binary (cargo install wasm-pack compiles from
# source and takes ~5 min on Vercel — the release tarball is ~5s).
WASM_PACK_VERSION="0.13.1"
WASM_PACK_DIR="wasm-pack-v${WASM_PACK_VERSION}-x86_64-unknown-linux-musl"
curl -fsSL \
  "https://github.com/rustwasm/wasm-pack/releases/download/v${WASM_PACK_VERSION}/${WASM_PACK_DIR}.tar.gz" \
  | tar xz -C /tmp
mv "/tmp/${WASM_PACK_DIR}/wasm-pack" "${CARGO_HOME}/bin/wasm-pack"

npm ci
