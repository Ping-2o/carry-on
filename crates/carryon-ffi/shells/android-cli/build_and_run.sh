#!/usr/bin/env bash
# Cross-compile the Carry-On C shell for aarch64-linux-android, push it and the
# container runtime (libcarryon_ffi.so) to a connected device via adb, and run the
# full import→carry→relaunch lifecycle on the device CPU + filesystem.
#
# LOCAL/PHYSICAL evidence (spec §2/§30/PLAT-001): this proves the engine executes
# on real hardware through the C ABI. It is NOT an APK and claims no "supported"
# platform (PLAT-006 packaging/signing is out of scope for this shell).
#
# Requirements:
#   - Android NDK (r27+). Set NDK=/path/to/android-ndk-rXX.
#   - A heavy build/output dir on external storage. Set WORK=/path (default below).
#   - rustup target: aarch64-linux-android (added automatically if missing).
#   - adb on PATH with exactly one device attached.
set -euo pipefail

NDK="${NDK:?set NDK to your android-ndk-rXX directory}"
WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation}"
API="${API:-21}"               # min API; runs on newer devices (device here is 33)
DEV_DIR="${DEV_DIR:-/data/local/tmp/carryon}"
TARGET=aarch64-linux-android

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"   # shells/android-cli -> repo root
HOST_TAG="$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64"  # NDK prebuilt dir name
TC="$NDK/toolchains/llvm/prebuilt/$HOST_TAG"
CLANG="$TC/bin/${TARGET}${API}-clang"
OUT="$WORK/android-out"

export CARGO_TARGET_DIR="$WORK/target"
export AR_aarch64_linux_android="$TC/bin/llvm-ar"
export CC_aarch64_linux_android="$CLANG"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CLANG"

echo ">> ensuring rust target $TARGET"
rustup target add "$TARGET" >/dev/null 2>&1 || true

echo ">> building container runtime (cdylib) for $TARGET"
( cd "$REPO" && cargo build -p carryon-ffi --target "$TARGET" --release )

SO="$CARGO_TARGET_DIR/$TARGET/release/libcarryon_ffi.so"
mkdir -p "$OUT"
cp "$SO" "$OUT/"

echo ">> compiling C shells with NDK clang"
"$CLANG" \
    -I "$REPO/crates/carryon-ffi/include" \
    "$HERE/carryon_shell.c" \
    -L "$OUT" -lcarryon_ffi \
    -o "$OUT/carryon_shell"
"$CLANG" \
    -I "$REPO/crates/carryon-ffi/include" \
    "$HERE/carryon_authority.c" \
    -L "$OUT" -lcarryon_ffi \
    -o "$OUT/carryon_authority"

echo ">> pushing to device $DEV_DIR"
adb shell "mkdir -p $DEV_DIR"
adb push "$OUT/libcarryon_ffi.so" "$DEV_DIR/" >/dev/null
adb push "$OUT/carryon_shell" "$DEV_DIR/" >/dev/null
adb push "$OUT/carryon_authority" "$DEV_DIR/" >/dev/null
adb shell "chmod 755 $DEV_DIR/carryon_shell $DEV_DIR/carryon_authority"

echo ">> running lifecycle shell on device"
adb shell "cd $DEV_DIR && LD_LIBRARY_PATH=$DEV_DIR ./carryon_shell $DEV_DIR/data"

echo ">> running L4 authority-transfer shell on device"
adb shell "cd $DEV_DIR && LD_LIBRARY_PATH=$DEV_DIR ./carryon_authority $DEV_DIR/authority"
