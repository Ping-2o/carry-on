#!/usr/bin/env bash
# build_engine_static.sh — build the REAL engine as a STATIC archive and vendor
# it into the SDK package's Vendor/ (fetched, never committed — the same law as
# every engine binary in this repo).
#
# Why static for the SDK: a committed dynamic dylib would ship an ad-hoc-signed
# binary (LOCAL-ONLY, not PLAT-006 distribution signing) and would need rpath
# handling in every client; SwiftPM links a static archive itself — no
# unsafeFlags, no runtime search path — so the package stays consumable as a
# real dependency (source distribution: the CLIENT builds the engine with this
# one command, then `swift build` just works).
#
#   sdk/swift/Scripts/build_engine_static.sh [--check]
#
# --check only verifies that the vendored archive exists and matches a fresh
# build's size-less fingerprint is not possible for binaries — --check verifies
# EXISTENCE + the vendored header's drift instead (the header is the ABI
# contract; the archive is a build artifact of it).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SDK="$(cd "$HERE/.." && pwd)"
REPO="${REPO:-$(cd "$SDK/../.." && pwd)}"
VENDOR="$SDK/Vendor"

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

# Repo canonical external target dir (AGENTS.md); override by exporting your own.
: "${CARGO_TARGET_DIR:=/Volumes/ADATA ELITE SE880/compilation/target}"
export CARGO_TARGET_DIR

mkdir -p "$VENDOR"

echo "build_engine_static: cargo build -p carryon-ffi --release (staticlib)"
(cd "$REPO" && cargo build -p carryon-ffi --release)

SRC="$CARGO_TARGET_DIR/release/libcarryon_ffi.a"
if [ ! -f "$SRC" ]; then
    echo "error: static archive not found at $SRC" >&2
    echo "       (carryon-ffi must build the staticlib crate-type — see Cargo.toml)" >&2
    exit 1
fi

# SwiftPM binary targets accept an .xcframework (not a raw .a): wrap the
# archive (macOS arm64 — the platform this repo builds and tests).
XCFW="$VENDOR/CarryOnEngine.xcframework"
rm -rf "$XCFW"
mkdir -p "$VENDOR/headers"
cp -f "$REPO/crates/carryon-ffi/include/carryon.h" "$VENDOR/headers/"
xcodebuild -create-xcframework \
    -library "$SRC" -headers "$VENDOR/headers" \
    -output "$XCFW" >/dev/null

# Sync the vendored header with a drift check (append-only ABI major 1).
HEADER_SRC="$REPO/crates/carryon-ffi/include/carryon.h"
HEADER_DST="$SDK/Sources/CCarryon/include/carryon.h"
if [ -f "$HEADER_DST" ] && ! cmp -s "$HEADER_SRC" "$HEADER_DST"; then
    echo "FAIL: vendored header drifted from $HEADER_SRC."
    echo "      The C ABI is append-only (ABI major 1); refresh the copy deliberately:"
    echo "      cp \"$HEADER_SRC\" \"$HEADER_DST\"  (after reviewing the diff)"
    exit 1
fi
mkdir -p "$(dirname "$HEADER_DST")"
cp "$HEADER_SRC" "$HEADER_DST"

echo "build_engine_static: vendored $XCFW"
ls -l "$XCFW"
