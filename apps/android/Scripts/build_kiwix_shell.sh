#!/usr/bin/env bash
# build_kiwix_shell.sh — build the mac-side Kiwix companion driver used by the
# stage-10 physical position-carry runs (the desktop half of the A04 reader).
#
#   crates/carryon-adapter-kiwix/shell/carryon_kiwix.c
#     pin    <id_dir>
#     source <id_dir> <data_dir> <bind_addr> <dest_pin_hex> <zim_path> <article_path> <scroll_y> [D|A]
#     dest   <id_dir> <data_dir> <src_addr> <src_pin_hex> <session_id> <zim_path> <article_path> <scroll_y> <navigate_path>
#
# It links a host engine built with --features kiwix (real libzim, see
# crates/carryon-adapter-kiwix/build.rs). Defaults match the SSD layout used by
# the phase-2/3 evidence; override with env.
#
#   CARGO_TARGET_DIR  — the checkout's OWN target dir (never shared)
#   CARRYON_LIBZIM_LIBDIR — host libzim dir (dylib + headers)
#   OUT               — output binary path
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
WORKDIR="${WORKDIR:-/Volumes/ADATA ELITE SE880/compilation}"
TARGET_DIR="${CARGO_TARGET_DIR:-$WORKDIR/target-kiwix}"
LIBZIM_LIBDIR="${CARRYON_LIBZIM_LIBDIR:-$WORKDIR/prefix/host/lib}"
OUT="${OUT:-/tmp/carryon-kiwix/carryon_kiwix}"

[ -f "$LIBZIM_LIBDIR/libzim.dylib" ] || {
  echo "FAIL: host libzim not found at $LIBZIM_LIBDIR/libzim.dylib (set CARRYON_LIBZIM_LIBDIR)"
  exit 1
}

echo ">> building carryon-ffi --features kiwix (host release)"
(
  cd "$REPO"
  CARGO_TARGET_DIR="$TARGET_DIR" \
    CARRYON_LIBZIM_LIBDIR="$LIBZIM_LIBDIR" \
    cargo build -p carryon-ffi --features kiwix --release
)

mkdir -p "$(dirname "$OUT")"
cc "$REPO/crates/carryon-adapter-kiwix/shell/carryon_kiwix.c" \
   -I "$REPO/crates/carryon-ffi/include" \
   -L "$TARGET_DIR/release" -lcarryon_ffi \
   -Wl,-rpath,"$TARGET_DIR/release" \
   -o "$OUT"
echo ">> OK: $OUT"
