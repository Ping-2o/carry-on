#!/usr/bin/env bash
# REAL L3 STRUCTURED CONTINUATION, mac -> android device (not process migration):
# a full working editor session (document + unsaved edits + cursor + selection +
# viewport + active tab + schema metadata) is carried over a LAN TLS 1.3 link; the
# destination restores the SAME logical session, the source disconnects, and the
# destination continues editing independently (L4 authority). The device-side
# evidence bundle records the measured continuation metrics.
#
# Physical cross-device evidence (PLAT-001): two machines, two NICs. No APK/signing
# (PLAT-006 out of scope). endpoint_changed_symbols is a SEPARATE symbol-distance
# metric (spec 5.6), never reported as bytes or runtime.
set -euo pipefail

NDK="${NDK:?set NDK to your android-ndk-rXX directory}"
WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation}"
API="${API:-21}"
PORT="${PORT:-48962}"
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

echo ">> building runtime (host + device) and the L3 shell"
( cd "$REPO" && cargo build -p carryon-ffi --release )
( cd "$REPO" && cargo build -p carryon-ffi --target "$TARGET" --release )
mkdir -p "$OUT/mac" "$OUT/mac/id-l3src"
cp "$CARGO_TARGET_DIR/release/libcarryon_ffi.dylib" "$OUT/mac/"
cp "$CARGO_TARGET_DIR/$TARGET/release/libcarryon_ffi.so" "$OUT/"
clang -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_l3.c" \
    -L "$OUT/mac" -lcarryon_ffi -Wl,-rpath,"$OUT/mac" -o "$OUT/mac/carryon_l3"
"$CLANG" -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_l3.c" \
    -L "$OUT" -lcarryon_ffi -o "$OUT/carryon_l3"

echo ">> push device binary + runtime"
adb shell "mkdir -p $DEV_DIR/id-l3dst"
adb push "$OUT/libcarryon_ffi.so" "$DEV_DIR/" >/dev/null
adb push "$OUT/carryon_l3" "$DEV_DIR/" >/dev/null
adb shell "chmod 755 $DEV_DIR/carryon_l3"

echo ">> pass 1: exchange stable pins"
MAC_PIN=$(DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_l3" pin "$OUT/mac/id-l3src")
DEV_PIN=$(adb shell "cd $DEV_DIR && LD_LIBRARY_PATH=$DEV_DIR ./carryon_l3 pin $DEV_DIR/id-l3dst" | tr -d '\r')
echo "   MAC_PIN=$MAC_PIN  DEV_PIN=$DEV_PIN"

echo ">> pass 2: mac serves the structured session; device restores + continues"
rm -rf "$OUT/mac/data-l3src"
adb shell "rm -rf $DEV_DIR/data-l3dst"
DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_l3" source \
    "$OUT/mac/id-l3src" "$OUT/mac/data-l3src" "0.0.0.0:$PORT" "$DEV_PIN" \
    > "$OUT/l3src.log" 2>&1 &
SRC_PID=$!
for _ in $(seq 1 50); do grep -q "waiting for destination" "$OUT/l3src.log" && break; sleep 0.1; done
SESS=$(grep -oE "session=[0-9a-f-]+" "$OUT/l3src.log" | head -1 | cut -d= -f2)
echo "   source session=$SESS"
adb shell "cd $DEV_DIR && L3_CUT=0 LD_LIBRARY_PATH=$DEV_DIR ./carryon_l3 dest \
    $DEV_DIR/id-l3dst $DEV_DIR/data-l3dst $MAC_IP:$PORT $MAC_PIN $SESS"
wait "$SRC_PID" || true
echo ">> source log:"; sed 's/^/   /' "$OUT/l3src.log"

echo ">> pull evidence bundle + print metrics"
adb pull "$DEV_DIR/data-l3dst/l3-evidence.json" "$OUT/l3-evidence.json" >/dev/null
python3 -c "import json;m=json.load(open('$OUT/l3-evidence.json'))['metrics'];print(json.dumps(m,indent=1))"
echo ">> L3 STRUCTURED CONTINUATION complete — not arbitrary process migration."
