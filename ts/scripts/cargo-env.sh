# shellcheck shell=bash
# Source rustup's env file. Vercel's build image installs cargo to
# /rust (via a baked-in rustup config) without exporting CARGO_HOME
# into our shell, so $HOME/.cargo/env never exists. Probe the known
# candidates and source whichever is present.
for __cargo_dir in /rust "${HOME}/.cargo" /usr/local/cargo; do
  if [ -f "${__cargo_dir}/env" ]; then
    # shellcheck source=/dev/null
    . "${__cargo_dir}/env"
    export CARGO_BIN_DIR="${__cargo_dir}/bin"
    unset __cargo_dir
    break
  fi
done

if [ -z "${CARGO_BIN_DIR:-}" ]; then
  echo "cargo-env.sh: no rustup env file found in /rust, ~/.cargo, /usr/local/cargo" >&2
  return 1 2>/dev/null || exit 1
fi
