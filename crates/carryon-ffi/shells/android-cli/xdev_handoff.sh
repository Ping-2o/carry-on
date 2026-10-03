#!/usr/bin/env bash
# REAL cross-device Carry-On handoff + L4 authority transfer:
#   mac (source, this host) ──LAN TLS 1.3──> android device (destination, via adb).
#
# Two physical machines, two NICs, mutual-pinned TLS over the wifi LAN. NOT loopback.
# Physical cross-device evidence (PLAT-001). Not a "supported platform" claim
# (no APK/signing; PLAT-006 out of scope).
#
# Requires: NDK (NDK=...), a heavy build dir (WORK=...), rustup target
# aarch64-linux-android, adb with one device attached, and both machines on the same
# LAN. Set MAC_IP to this host's LAN address reachable from the device.
set -euo pipefail

NDK="${NDK:?set NDK to your android-ndk-rXX directory}"
WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation}"
API="${API:-21}"
PORT="${PORT:-48960}"
MAC_IP="${MAC_IP:?set MAC_IP to this host LAN ip (e.g. 192.168.3.201)}"
DEV_DIR="${DEV_DIR:-/data/local/tmp/carryon}"
TARGET=aarch64-linux-android

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"
HOST_TAG="$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64"
TC="$NDK/toolchains/llvm/prebuilt/$HOST_TAG"
CLANG="$TC/bin/${TARGET}${API}-clang"
OUT="$WORK/android-out"

export CARGO_TARGET_DIR="$WORK/target"
export AR_aarch64_linux_android="$TC/bin/llvm-ar"
export CC_aarch64_linux_android="$CLANG"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CLANG"

rustup target add "$TARGET" >/dev/null 2>&1 || true

echo ">> building runtime for host (mac) and for $TARGET (device)"
( cd "$REPO" && cargo build -p carryon-ffi --release )
( cd "$REPO" && cargo build -p carryon-ffi --target "$TARGET" --release )

mkdir -p "$OUT/mac"
cp "$CARGO_TARGET_DIR/release/libcarryon_ffi.dylib" "$OUT/mac/"
cp "$CARGO_TARGET_DIR/$TARGET/release/libcarryon_ffi.so" "$OUT/"

echo ">> compiling xdev shell for host + device"
clang -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_xdev.c" \
    -L "$OUT/mac" -lcarryon_ffi -Wl,-rpath,"$OUT/mac" -o "$OUT/mac/carryon_xdev"
"$CLANG" -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_xdev.c" \
    -L "$OUT" -lcarryon_ffi -o "$OUT/carryon_xdev"

echo ">> pushing device binary + runtime"
adb shell "mkdir -p $DEV_DIR/id-dst"
adb push "$OUT/libcarryon_ffi.so" "$DEV_DIR/" >/dev/null
adb push "$OUT/carryon_xdev" "$DEV_DIR/" >/dev/null
adb shell "chmod 755 $DEV_DIR/carryon_xdev"

echo ">> pass 1: exchange stable pins"
mkdir -p "$OUT/mac/id-src"
MAC_PIN=$(DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_xdev" pin "$OUT/mac/id-src")
DEV_PIN=$(adb shell "cd $DEV_DIR && LD_LIBRARY_PATH=$DEV_DIR ./carryon_xdev pin $DEV_DIR/id-dst" | tr -d '\r')
echo "   MAC_PIN=$MAC_PIN"
echo "   DEV_PIN=$DEV_PIN"

echo ">> pass 2: start mac source (binds 0.0.0.0:$PORT), then device dest connects"
rm -rf "$OUT/mac/data-src"
adb shell "rm -rf $DEV_DIR/data-dst"
DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_xdev" source \
    "$OUT/mac/id-src" "$OUT/mac/data-src" "0.0.0.0:$PORT" "$DEV_PIN" > "$OUT/src.log" 2>&1 &
SRC_PID=$!
# wait for the listener line + session id
for _ in $(seq 1 50); do grep -q "waiting for destination" "$OUT/src.log" && break; sleep 0.1; done
SESS=$(grep -oE "session=[0-9a-f-]+" "$OUT/src.log" | head -1 | cut -d= -f2)
echo "   source session=$SESS listening on $MAC_IP:$PORT"

adb shell "cd $DEV_DIR && XDEV_CUT=0 LD_LIBRARY_PATH=$DEV_DIR ./carryon_xdev dest \
    $DEV_DIR/id-dst $DEV_DIR/data-dst $MAC_IP:$PORT $MAC_PIN $SESS"

wait "$SRC_PID" || true
echo ">> source log:"
sed 's/^/   /' "$OUT/src.log"
