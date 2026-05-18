#!/usr/bin/env bash
set -euo pipefail

# Vercel's build image ships rustup + cargo at /rust. We *don't*
# re-run the rustup installer — it trips on the existing install
# and aborts because $HOME=/vercel doesn't match the euid home.
# Just source the env file (cargo-env.sh probes /rust, ~/.cargo,
# /usr/local/cargo for the rustup-generated `env`).
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=cargo-env.sh
. "${SCRIPT_DIR}/cargo-env.sh"

# rust-toolchain.toml at repo root pins channel 1.85 + the
# wasm32-unknown-unknown target, but Vercel's pre-installed rustup
# may use a different default toolchain. Explicitly install the
# pinned channel and target — rustup is idempotent so this is a
# no-op if the toolchain is already present.
rustup show active-toolchain || rustup toolchain install --profile minimal stable
rustup target add wasm32-unknown-unknown

# Prebuilt wasm-pack binary (cargo install wasm-pack compiles from
# source and takes ~5 min on Vercel — the release tarball is ~5s).
WASM_PACK_VERSION="0.13.1"
WASM_PACK_DIR="wasm-pack-v${WASM_PACK_VERSION}-x86_64-unknown-linux-musl"
curl -fsSL \
  "https://github.com/rustwasm/wasm-pack/releases/download/v${WASM_PACK_VERSION}/${WASM_PACK_DIR}.tar.gz" \
  | tar xz -C /tmp
mv "/tmp/${WASM_PACK_DIR}/wasm-pack" "${CARGO_BIN_DIR}/wasm-pack"

npm ci
