#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
bin_dir="${CODEX24H_BIN_DIR:-$HOME/.local/bin}"
cd "$project_dir"
cargo build --release --locked
mkdir -p "$bin_dir"
temporary_binary="$bin_dir/.codex24h-install-$$"
trap 'rm -f -- "$temporary_binary"' EXIT
install -m 755 target/release/codex24h "$temporary_binary"
mv -f -- "$temporary_binary" "$bin_dir/codex24h"
install -m 755 scripts/codex24h-mail "$bin_dir/codex24h-mail"
install -m 755 scripts/codex24h-attach "$bin_dir/codex24h-attach"
install -m 755 scripts/codex24h-session "$bin_dir/codex24h-session"
printf 'Installed %s/codex24h\n' "$bin_dir"
printf 'Run: codex24h [the same arguments you pass to codex]\n'
