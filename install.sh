#!/usr/bin/env bash
set -euo pipefail

bin_dir="${CODEX24H_BIN_DIR:-$HOME/.local/bin}"
for dependency in cargo python3 curl tar; do
    command -v "$dependency" >/dev/null || { printf 'Missing dependency: %s\n' "$dependency" >&2; exit 1; }
done

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex24h-install.XXXXXXXX")"
stage_dir=""
cleanup() {
    rm -rf -- "$work_dir"
    if [[ -n "$stage_dir" ]]; then rm -rf -- "$stage_dir"; fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
    project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
else
    curl --fail --show-error --location \
        https://codeload.github.com/Vinnish-A/codex24h/tar.gz/refs/heads/main \
        --output "$work_dir/source.tar.gz"
    tar -xzf "$work_dir/source.tar.gz" -C "$work_dir"
    project_dir="$work_dir/codex24h-main"
fi

cargo build --release --locked --manifest-path "$project_dir/Cargo.toml" --target-dir "$work_dir/target"
mkdir -p "$bin_dir"
stage_dir="$(mktemp -d "$bin_dir/.codex24h-install.XXXXXXXX")"
install -m 755 "$work_dir/target/release/codex24h" "$stage_dir/codex24h"
for helper in mail attach requests session; do
    install -m 755 "$project_dir/scripts/codex24h-$helper" "$stage_dir/codex24h-$helper"
done
for program in "$stage_dir"/*; do
    mv -f -- "$program" "$bin_dir/"
done
printf 'Installed %s/codex24h\n' "$bin_dir"
printf 'Run: codex24h [the same arguments you pass to codex]\n'
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) printf 'Add %s to your PATH to use the codex24h command.\n' "$bin_dir" ;;
esac
