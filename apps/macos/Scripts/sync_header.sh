#!/bin/sh
# sync_header.sh — copy the canonical C ABI header into the CCarryon target.
#
#   Scripts/sync_header.sh [--force]
#
# Canonical source: crates/carryon-ffi/include/carryon.h (ABI 1.1, append-only
# within ABI major 1; see tests/header_sync.rs for the Rust-side invariant).
#
# Drift check: if the vendored copy already exists and DIFFERS from the canonical
# header, refuse to overwrite it silently — either the vendored copy was
# hand-edited (must not happen; it is generated) or the canonical header moved
# (re-sync deliberately with --force after reviewing the ABI change). A silent
# overwrite would hide ABI drift from the reviewer, so it fails closed.
set -eu

app_dir="$(cd "$(dirname "$0")/.." && pwd)"
repo_root="$(cd "$app_dir/../.." && pwd)"
src="$repo_root/crates/carryon-ffi/include/carryon.h"
dst_dir="$app_dir/Sources/CCarryon/include"
dst="$dst_dir/carryon.h"

if [ ! -f "$src" ]; then
    echo "error: canonical header not found: $src" >&2
    exit 1
fi

if [ -f "$dst" ] && ! cmp -s "$src" "$dst"; then
    if [ "${1:-}" != "--force" ]; then
        echo "error: vendored header drifted from crates/carryon-ffi/include/carryon.h" >&2
        echo "       (refusing to overwrite silently; review the diff, then re-run with --force)" >&2
        diff -u "$dst" "$src" | head -40 >&2 || true
        exit 1
    fi
    echo "sync_header: forced overwrite of drifted vendored header"
fi

mkdir -p "$dst_dir"
cp -f "$src" "$dst"
echo "sync_header: $src -> $dst (in sync)"
