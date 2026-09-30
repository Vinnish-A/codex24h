#!/usr/bin/env bash
set -euo pipefail

bin_dir="${CODEX24H_BIN_DIR:-$HOME/.local/bin}"
lib_dir="${CODEX24H_LIB_DIR:-$(dirname -- "$bin_dir")/lib/codex24h}"
for dependency in curl tar sha256sum; do
    command -v "$dependency" >/dev/null || { printf 'Missing dependency: %s\n' "$dependency" >&2; exit 1; }
done
if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
    printf 'Prebuilt releases currently support Linux x86_64 (including WSL).\n' >&2
    exit 1
fi

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex24h-install.XXXXXXXX")"
stage_dir=""
link_dir=""
cleanup() {
    rm -rf -- "$work_dir"
    if [[ -n "$stage_dir" ]]; then rm -rf -- "$stage_dir"; fi
    if [[ -n "$link_dir" ]]; then rm -rf -- "$link_dir"; fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

repo_url="https://github.com/Vinnish-A/codex24h"
version="${CODEX24H_VERSION:-}"
asset="codex24h-linux-x86_64.tar.gz"
if [[ -n "${CODEX24H_ARCHIVE:-}" ]]; then
    cp -- "$CODEX24H_ARCHIVE" "$work_dir/$asset"
    cp -- "$CODEX24H_ARCHIVE.sha256" "$work_dir/$asset.sha256"
else
    if [[ -z "$version" ]]; then
        release_url="$(curl -fsSL --connect-timeout 15 --max-time 60 -o /dev/null -w '%{url_effective}' "$repo_url/releases/latest")"
        version="${release_url##*/}"
    fi
    [[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { printf 'Invalid release version: %s\n' "$version" >&2; exit 1; }
    base_url="$repo_url/releases/download/$version"
    curl -fsSL --connect-timeout 15 --max-time 180 "$base_url/$asset" -o "$work_dir/$asset"
    curl -fsSL --connect-timeout 15 --max-time 60 "$base_url/$asset.sha256" -o "$work_dir/$asset.sha256"
fi
(cd "$work_dir" && sha256sum --check "$asset.sha256")
mkdir "$work_dir/package"
tar -xzf "$work_dir/$asset" -C "$work_dir/package"
"$work_dir/package/codex24h-mail" --help >/dev/null
package_version="$(cat "$work_dir/package/VERSION")"
version="${version:-v$package_version}"
[[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { printf 'Invalid package version\n' >&2; exit 1; }
[[ "$package_version" == "${version#v}" ]] || { printf 'Release version mismatch\n' >&2; exit 1; }

mkdir -p "$bin_dir" "$lib_dir/releases"
bin_dir="$(cd "$bin_dir" && pwd)"
lib_dir="$(cd "$lib_dir" && pwd)"
release_dir="$lib_dir/releases/$version"
if [[ ! -d "$release_dir" ]]; then
    stage_dir="$(mktemp -d "$lib_dir/.install.XXXXXXXX")"
    cp -a "$work_dir/package/." "$stage_dir/"
    mv -- "$stage_dir" "$release_dir"
    stage_dir=""
fi
stage_dir="$(mktemp -d "$lib_dir/.links.XXXXXXXX")"
ln -s "releases/$version" "$stage_dir/current"
mv -Tf -- "$stage_dir/current" "$lib_dir/current"
for program in codex24h codex24h-mail codex24h-attach codex24h-requests codex24h-session; do
    # Rename on the destination filesystem, including custom bin directories.
    link_dir="$(mktemp -d "$bin_dir/.codex24h-link.XXXXXXXX")"
    ln -s "$lib_dir/current/$program" "$link_dir/$program"
    mv -Tf -- "$link_dir/$program" "$bin_dir/$program"
    rmdir "$link_dir"
    link_dir=""
done
# Keep an old release only while a running process still uses it.
for old_dir in "$lib_dir"/releases/*; do
    [[ "$old_dir" == "$release_dir" || ! -d "$old_dir" ]] && continue
    in_use=false
    for process in /proc/[0-9]*/exe; do
        executable="$(readlink "$process" 2>/dev/null || true)"
        if [[ "$executable" == "$old_dir/"* ]]; then in_use=true; break; fi
    done
    if [[ "$in_use" == false ]]; then rm -rf -- "$old_dir"; fi
done
printf 'Installed codex24h %s to %s\n' "$version" "$bin_dir"
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) printf 'Add %s to your PATH to use the codex24h command.\n' "$bin_dir" ;;
esac
