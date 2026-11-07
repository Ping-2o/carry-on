#!/usr/bin/env bash
# fetch_engine_lib.sh — cross-compile the REAL engine for Android and place it in
# the app's jniLibs directory.
#
#   cargo build -p carryon-ffi --target aarch64-linux-android --release
#
# with the NDK cross-compile env proven by
# crates/carryon-ffi/shells/android-cli/build_and_run.sh (AR_/CC_/LINKER; NDK
# r27+; external CARGO_TARGET_DIR). The resulting libcarryon_ffi.so is copied to
# app/src/main/jniLibs/arm64-v8a/ — a FETCHED build artifact, gitignored (see
# app/.gitignore), never committed. The vendored header
# app/src/main/cpp/include/carryon.h is re-synced with a drift check.
#
# Env:
#   NDK  — Android NDK root (default: the machine's external-storage NDK r27c).
#   WORK — heavy build/output dir (default: same external dir as the shells).
#          NEVER share a CARGO_TARGET_DIR across worktrees/checkouts (see
#          AGENTS.md: shared dirs caused a real incident 2026-10-29).
#   REPO — this repository's root (default: computed from this script's path).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ANDROID="$(cd "$HERE/.." && pwd)"
REPO="${REPO:-$(cd "$APP_ANDROID/../.." && pwd)}"

NDK="${NDK:-/Volumes/ADATA ELITE SE880/compilation/android-ndk-r27c}"
WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation}"
TARGET=aarch64-linux-android
ABI=arm64-v8a
API="${API:-29}"   # match minSdk 29 (spec §9)

HOST_TAG="$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64"
TC="$NDK/toolchains/llvm/prebuilt/$HOST_TAG"
CLANG="$TC/bin/${TARGET}${API}-clang"
SO_SRC="$WORK/target/$TARGET/release/libcarryon_ffi.so"
JNI_DIR="$APP_ANDROID/app/src/main/jniLibs/$ABI"
HEADER_SRC="$REPO/crates/carryon-ffi/include/carryon.h"
HEADER_DST="$APP_ANDROID/app/src/main/cpp/include/carryon.h"

[ -d "$NDK" ] || { echo "FAIL: NDK not found at $NDK (set NDK=...)"; exit 1; }
[ -x "$CLANG" ] || { echo "FAIL: NDK clang missing: $CLANG"; exit 1; }
[ -f "$HEADER_SRC" ] || { echo "FAIL: engine header missing: $HEADER_SRC"; exit 1; }

echo ">> ensuring rustup target $TARGET"
rustup target add "$TARGET" >/dev/null 2>&1 || true

echo ">> cross-compiling carryon-ffi (cdylib) for $TARGET (release)"
echo "   CARGO_TARGET_DIR=$WORK/target   (each checkout gets its own dir!)"
mkdir -p "$JNI_DIR"
(
  cd "$REPO"
  export CARGO_TARGET_DIR="$WORK/target"
  export AR_aarch64_linux_android="$TC/bin/llvm-ar"
  export CC_aarch64_linux_android="$CLANG"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CLANG"
  cargo build -p carryon-ffi --target "$TARGET" --release
)

echo ">> copying engine binary -> $JNI_DIR/"
cp "$SO_SRC" "$JNI_DIR/libcarryon_ffi.so"

echo ">> syncing ABI header (drift check)"
mkdir -p "$(dirname "$HEADER_DST")"
if [ -f "$HEADER_DST" ] && ! cmp -s "$HEADER_SRC" "$HEADER_DST"; then
  echo "FAIL: vendored header drifted from $HEADER_SRC."
  echo "      The C ABI is append-only (ABI major 1); refresh the copy deliberately:"
  echo "      cp \"$HEADER_SRC\" \"$HEADER_DST\"  (after reviewing the diff)"
  exit 1
fi
cp "$HEADER_SRC" "$HEADER_DST"

echo ">> re-checking the JNI shim against the NDK sysroot"
"$CLANG" -fsyntax-only -Wall -Wextra -I "$REPO/crates/carryon-ffi/include" \
    "$APP_ANDROID/app/src/main/cpp/carryon_jni.c"

echo ">> OK: $JNI_DIR/libcarryon_ffi.so"
ls -l "$JNI_DIR/libcarryon_ffi.so"
