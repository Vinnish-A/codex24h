#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
output_dir="${1:-$project_dir/target/release-assets}"
mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd)"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex24h-package.XXXXXXXX")"
trap 'rm -rf -- "$work_dir"' EXIT
cargo build --release --locked
mkdir "$work_dir/helpers"
python3 - "$project_dir/scripts" "$work_dir/helpers" <<'PYCOMPILE'
from pathlib import Path
import py_compile
import sys
for name in ('mail', 'attach', 'requests', 'session'):
    source = Path(sys.argv[1], 'codex24h-' + name)
    py_compile.compile(str(source), cfile=str(Path(sys.argv[2], source.name)), doraise=True)
PYCOMPILE
python3 -m PyInstaller --noconfirm --clean --onedir --strip \
    --name codex24h-helper --distpath "$work_dir/dist" \
    --workpath "$work_dir/build" --specpath "$work_dir" \
    --add-data "$work_dir/helpers:scripts" \
    --hidden-import tomli --hidden-import argparse --hidden-import fcntl \
    --hidden-import getpass --hidden-import hashlib --hidden-import smtplib \
    --hidden-import sqlite3 --hidden-import ssl --hidden-import email.headerregistry \
    --hidden-import email.utils --hidden-import select --hidden-import signal \
    --hidden-import uuid --hidden-import subprocess --hidden-import tempfile \
    --hidden-import json --hidden-import shlex --hidden-import datetime \
    --hidden-import contextlib --hidden-import shutil \
    "$project_dir/packaging/helper.py"
package_dir="$work_dir/dist/codex24h-helper"
install -m 755 target/release/codex24h "$package_dir/codex24h"
install -m 644 packaging/THIRD_PARTY.md "$package_dir/THIRD_PARTY.md"
mkdir -p "$package_dir/licenses"
for library in python3.10 libpython3.10-stdlib libssl3 libsqlite3-0 libexpat1 zlib1g liblzma5 libbz2-1.0 libuuid1 libmpdec3; do
    if [[ -f "/usr/share/doc/$library/copyright" ]]; then
        cp "/usr/share/doc/$library/copyright" "$package_dir/licenses/$library.txt"
    fi
done
python3 - "$package_dir/licenses" <<'PYLICENSE'
import importlib.metadata
from pathlib import Path
import sys
for name in ('tomli', 'pyinstaller'):
    dist = importlib.metadata.distribution(name)
    for file in dist.files or ():
        if 'license' in str(file).lower() or Path(file).name.startswith('COPYING'):
            source = Path(dist.locate_file(file))
            if source.is_file():
                Path(sys.argv[1], name + '-' + source.name).write_bytes(source.read_bytes())
PYLICENSE
for helper in mail attach requests session; do
    ln -s codex24h-helper "$package_dir/codex24h-$helper"
done
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
printf '%s\n' "$version" > "$package_dir/VERSION"
asset="codex24h-linux-$(uname -m).tar.gz"
tar -czf "$output_dir/$asset" -C "$package_dir" .
(cd "$output_dir" && sha256sum "$asset" > "$asset.sha256")
printf 'Release archive: %s/%s\n' "$output_dir" "$asset"
