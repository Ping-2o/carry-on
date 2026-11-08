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
#   NDK        — Android NDK root (default: the machine's external-storage NDK r27c).
#   WORK       — heavy build/output dir (default: same external dir as the shells).
#                NEVER share a CARGO_TARGET_DIR across worktrees/checkouts (see
#                AGENTS.md: shared dirs caused a real incident 2026-10-29).
#   CARGO_TARGET_DIR — exact target dir override (worktree-specific dirs:
#                .../target-kiwix, .../target-artcraft). Defaults to $WORK/target.
#   REPO       — this repository's root (default: computed from this script's path).
#
#   CARRYON_KIWIX=1 — compile the `kiwix` feature IN (the org.carryon.kiwix
#                adapter over the real libzim C backend) for the companion
#                reader (stage 10). Needs a static libzim for aarch64-linux-
#                android; paths default to the Phase-1/2 builds on the SSD:
#                libzim  CARRYON_LIBZIM_LIBDIR / CARRYON_LIBZIM_INCDIR
#                deps    CARRYON_LIBZIM_DEP_LIBDIRS (liblzma:libzstd dirs)
#                The static libzim links the C++ runtime dynamically in the
#                final cdylib, so libc++_shared.so from the NDK is copied into
#                jniLibs alongside (the Phase-2 §E gotcha — ship it).
#                CI does NOT build this (no libzim on runners); the default
#                path (no CARRYON_KIWIX) stays byte-for-byte the old build.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ANDROID="$(cd "$HERE/.." && pwd)"
REPO="${REPO:-$(cd "$APP_ANDROID/../.." && pwd)}"

NDK="${NDK:-/Volumes/ADATA ELITE SE880/compilation/android-ndk-r27c}"
WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation}"
TARGET=aarch64-linux-android
ABI=arm64-v8a
API="${API:-29}"   # match minSdk 29 (spec §9)
TARGET_DIR="${CARGO_TARGET_DIR:-$WORK/target}"
KIWIX="${CARRYON_KIWIX:-0}"

HOST_TAG="$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64"
TC="$NDK/toolchains/llvm/prebuilt/$HOST_TAG"
CLANG="$TC/bin/${TARGET}${API}-clang"
CXX="$TC/bin/${TARGET}${API}-clang++"
SO_SRC="$TARGET_DIR/$TARGET/release/libcarryon_ffi.so"
JNI_DIR="$APP_ANDROID/app/src/main/jniLibs/$ABI"
HEADER_SRC="$REPO/crates/carryon-ffi/include/carryon.h"
HEADER_DST="$APP_ANDROID/app/src/main/cpp/include/carryon.h"

[ -d "$NDK" ] || { echo "FAIL: NDK not found at $NDK (set NDK=...)"; exit 1; }
[ -x "$CLANG" ] || { echo "FAIL: NDK clang missing: $CLANG"; exit 1; }
[ -f "$HEADER_SRC" ] || { echo "FAIL: engine header missing: $HEADER_SRC"; exit 1; }

echo ">> ensuring rustup target $TARGET"
rustup target add "$TARGET" >/dev/null 2>&1 || true

FEATURES=()
KIWIX_LIBZIM_DIR="${CARRYON_LIBZIM_DIR:-/Volumes/ADATA ELITE SE880/compilation/libzim}"
if [ "$KIWIX" != "0" ]; then
  [ -x "$CXX" ] || { echo "FAIL: NDK clang++ missing: $CXX"; exit 1; }
  [ -f "$KIWIX_LIBZIM_DIR/build-android/src/libzim.a" ] || {
    echo "FAIL: static libzim for $TARGET not found at $KIWIX_LIBZIM_DIR/build-android/src/libzim.a"
    echo "      (see docs/kiwix-bridge/BUILD-EVIDENCE.md §B; set CARRYON_LIBZIM_DIR=...)"
    exit 1
  }
  FEATURES=(--features kiwix)
  export CARRYON_LIBZIM_LIBDIR="${CARRYON_LIBZIM_LIBDIR:-$KIWIX_LIBZIM_DIR/build-android/src}"
  export CARRYON_LIBZIM_INCDIR="${CARRYON_LIBZIM_INCDIR:-$KIWIX_LIBZIM_DIR/build-android/include:$KIWIX_LIBZIM_DIR/include}"
  export CARRYON_LIBZIM_STATIC=1
  export CARRYON_LIBZIM_DEP_LIBDIRS="${CARRYON_LIBZIM_DEP_LIBDIRS:-$KIWIX_LIBZIM_DIR/build-android/subprojects/xz-5.2.11/src/liblzma:$KIWIX_LIBZIM_DIR/build-android/subprojects/zstd-1.5.4/build/meson/lib}"
  export CXX_aarch64_linux_android="$CXX"
  echo ">> kiwix feature ON (static libzim: $CARRYON_LIBZIM_LIBDIR)"
fi

echo ">> cross-compiling carryon-ffi (cdylib) for $TARGET (release)"
echo "   CARGO_TARGET_DIR=$TARGET_DIR   (each checkout gets its own dir!)"
mkdir -p "$JNI_DIR"
(
  cd "$REPO"
  export CARGO_TARGET_DIR="$TARGET_DIR"
  export AR_aarch64_linux_android="$TC/bin/llvm-ar"
  export CC_aarch64_linux_android="$CLANG"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CLANG"
  cargo build -p carryon-ffi "${FEATURES[@]}" --target "$TARGET" --release
)

echo ">> copying engine binary -> $JNI_DIR/"
cp "$SO_SRC" "$JNI_DIR/libcarryon_ffi.so"

if [ "$KIWIX" != "0" ]; then
  # The static libzim path leaves the C++ runtime dynamic in the cdylib
  # (DT_NEEDED libc++_shared.so — see docs/kiwix-bridge/BUILD-EVIDENCE.md).
  CPP_SHARED="$TC/sysroot/usr/lib/$TARGET/libc++_shared.so"
  [ -f "$CPP_SHARED" ] || { echo "FAIL: $CPP_SHARED missing"; exit 1; }
  cp "$CPP_SHARED" "$JNI_DIR/libc++_shared.so"
  echo ">> shipped libc++_shared.so (libzim's C++ runtime dependency)"
  NEEDED=$("$TC/bin/llvm-readelf" -d "$JNI_DIR/libcarryon_ffi.so" 2>/dev/null | grep NEEDED | grep -o '\[[^]]*\]' || true)
  echo ">> DT_NEEDED: $NEEDED"
fi

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
ls -l "$JNI_DIR/"
