#!/bin/sh
# build_engine.sh — build the REAL Carry-On engine (carryon-ffi) in release mode
# and vendor the dylib into apps/macos/Vendor/ for the Swift binding to link.
#
#   Scripts/build_engine.sh
#
# - Honors the repo law (AGENTS.md): heavy builds go on external storage via
#   CARGO_TARGET_DIR (defaulted here to the repo's canonical external dir).
# - The dylib's install name is rewritten to @rpath/libcarryon_ffi.dylib so the
#   app bundle (Contents/Frameworks) and dev builds (absolute Vendor rpath) both
#   resolve it without touching the engine.
# - The dylib is ad-hoc re-signed after install_name_tool invalidates any
#   signature. This is LOCAL-ONLY signing, NOT PLAT-006 distribution signing.
set -eu

app_dir="$(cd "$(dirname "$0")/.." && pwd)"
repo_root="$(cd "$app_dir/../.." && pwd)"

# cargo may live in ~/.cargo/bin (hosted-env note in AGENTS.md).
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

# Repo canonical external target dir (AGENTS.md); override by exporting your own.
: "${CARGO_TARGET_DIR:=/Volumes/ADATA ELITE SE880/compilation/target}"
export CARGO_TARGET_DIR

echo "build_engine: cargo build -p carryon-ffi --release (CARGO_TARGET_DIR=$CARGO_TARGET_DIR)"
(cd "$repo_root" && cargo build -p carryon-ffi --release)

src="$CARGO_TARGET_DIR/release/libcarryon_ffi.dylib"
if [ ! -f "$src" ]; then
    echo "error: engine dylib not found at $src" >&2
    exit 1
fi

mkdir -p "$app_dir/Vendor"
cp -f "$src" "$app_dir/Vendor/libcarryon_ffi.dylib"
install_name_tool -id @rpath/libcarryon_ffi.dylib "$app_dir/Vendor/libcarryon_ffi.dylib"

# Ad-hoc sign the modified dylib (LOCAL-ONLY; not PLAT-006 distribution signing).
codesign --sign - --force "$app_dir/Vendor/libcarryon_ffi.dylib" 2>/dev/null || true

echo "build_engine: vendored $app_dir/Vendor/libcarryon_ffi.dylib (install name @rpath/libcarryon_ffi.dylib)"
