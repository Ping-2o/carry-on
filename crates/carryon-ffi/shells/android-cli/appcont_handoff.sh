#!/usr/bin/env bash
# REAL external-application continuation, mac -> android device:
#   a real file (default: a generated PNG) open on the mac is captured by the file
#   adapter (L1, content-hash identity, §10.7), carried over a LAN TLS 1.3 link, and
#   on the device reconstructed byte-for-byte and opened in a REAL installed Android
#   app (Samsung Gallery) via the documented ACTION_VIEW intent (§10.3 PLAT-003).
#
# Physical cross-device evidence (PLAT-001): two machines, two NICs, real TLS. The
# destination verifies the reconstructed file's content-hash equals the source's
# before launch — file identity guaranteed. Honest L1: no unsaved in-memory app state.
# Not an APK; no platform "supported" (PLAT-006 out of scope).
set -euo pipefail

NDK="${NDK:?set NDK to your android-ndk-rXX directory}"
WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation}"
API="${API:-21}"
PORT="${PORT:-48961}"
MAC_IP="${MAC_IP:?set MAC_IP to this host LAN ip (e.g. 192.168.3.201)}"
DEV_DIR="${DEV_DIR:-/data/local/tmp/carryon}"
FILE="${FILE:-}"            # a real file to carry; if empty, a PNG is generated
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

echo ">> building runtime (host + device) and the appcont shell"
( cd "$REPO" && cargo build -p carryon-ffi --release )
( cd "$REPO" && cargo build -p carryon-ffi --target "$TARGET" --release )
mkdir -p "$OUT/mac" "$OUT/doc"
cp "$CARGO_TARGET_DIR/release/libcarryon_ffi.dylib" "$OUT/mac/"
cp "$CARGO_TARGET_DIR/$TARGET/release/libcarryon_ffi.so" "$OUT/"
clang -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_appcont.c" \
    -L "$OUT/mac" -lcarryon_ffi -Wl,-rpath,"$OUT/mac" -o "$OUT/mac/carryon_appcont"
"$CLANG" -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_appcont.c" \
    -L "$OUT" -lcarryon_ffi -o "$OUT/carryon_appcont"

if [ -z "$FILE" ]; then
    echo ">> generating a real PNG to carry"
    FILE="$OUT/doc/carryon-doc.png"
    OUT="$OUT" python3 - <<'PY'
import struct, zlib, os
W,H=512,256
def chunk(t,d):
    c=t+d; return struct.pack(">I",len(d))+c+struct.pack(">I",zlib.crc32(c)&0xffffffff)
raw=bytearray()
for y in range(H):
    raw.append(0)
    for x in range(W): raw+=bytes(((x*255)//W,(y*255)//H,(x^y)&0xff))
png=b"\x89PNG\r\n\x1a\n"+chunk(b"IHDR",struct.pack(">IIBBBBB",W,H,8,2,0,0,0))
png+=chunk(b"IDAT",zlib.compress(bytes(raw),9))+chunk(b"IEND",b"")
open(os.environ["OUT"]+"/doc/carryon-doc.png","wb").write(png)
PY
fi
HASH=$(shasum -a 256 "$FILE" | cut -d' ' -f1)
echo "   file=$FILE sha256=$HASH"

echo ">> push device binary + runtime + placeholder"
adb shell "mkdir -p $DEV_DIR/id-appdst"
adb push "$OUT/libcarryon_ffi.so" "$DEV_DIR/" >/dev/null
adb push "$OUT/carryon_appcont" "$DEV_DIR/" >/dev/null
adb shell "chmod 755 $DEV_DIR/carryon_appcont; echo placeholder > $DEV_DIR/placeholder.txt"

echo ">> pass 1: exchange stable pins"
mkdir -p "$OUT/mac/id-appsrc"
MAC_PIN=$(DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_appcont" pin "$OUT/mac/id-appsrc")
DEV_PIN=$(adb shell "cd $DEV_DIR && LD_LIBRARY_PATH=$DEV_DIR ./carryon_appcont pin $DEV_DIR/id-appdst" | tr -d '\r')
echo "   MAC_PIN=$MAC_PIN  DEV_PIN=$DEV_PIN"

echo ">> pass 2: mac serves the real file; device imports + reconstructs"
DST_PNG=/sdcard/Download/carryon-doc.png
rm -rf "$OUT/mac/data-appsrc"
adb shell "rm -rf $DEV_DIR/data-appdst $DST_PNG"
DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_appcont" source \
    "$OUT/mac/id-appsrc" "$OUT/mac/data-appsrc" "0.0.0.0:$PORT" "$DEV_PIN" "$FILE" \
    > "$OUT/appsrc.log" 2>&1 &
SRC_PID=$!
for _ in $(seq 1 50); do grep -q "waiting for destination" "$OUT/appsrc.log" && break; sleep 0.1; done
SESS=$(grep -oE "session=[0-9a-f-]+" "$OUT/appsrc.log" | head -1 | cut -d= -f2)
echo "   source session=$SESS"
adb shell "cd $DEV_DIR && APPCONT_CUT=0 LD_LIBRARY_PATH=$DEV_DIR ./carryon_appcont dest \
    $DEV_DIR/id-appdst $DEV_DIR/data-appdst $MAC_IP:$PORT $MAC_PIN $SESS $HASH $DST_PNG"
wait "$SRC_PID" || true

echo ">> verify device-side hash matches source"
DEV_HASH=$(adb shell "sha256sum $DST_PNG" | awk '{print $1}' | tr -d '\r')
if [ "$DEV_HASH" != "$HASH" ]; then echo "FAIL hash mismatch: $DEV_HASH != $HASH"; exit 2; fi
echo "   device hash == source hash ($HASH) — byte-for-byte identity"

echo ">> index + launch the real Android app (Samsung Gallery) on the carried image"
adb shell "am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE -d file://$DST_PNG" >/dev/null 2>&1
sleep 1
ID=$(adb shell "content query --uri content://media/external/images/media --projection _id --where \"_data LIKE '%carryon-doc.png'\"" | grep -oE "_id=[0-9]+" | head -1 | cut -d= -f2 | tr -d '\r')
adb shell "input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard" >/dev/null 2>&1 || true
if [ -n "$ID" ]; then
    adb shell "am start -a android.intent.action.VIEW -d content://media/external/images/media/$ID -t image/png --grant-read-uri-permission -n com.sec.android.gallery3d/.app.GalleryActivity" | head -1
else
    adb shell "am start -a android.intent.action.VIEW -d file://$DST_PNG -t image/png" | head -1
fi
sleep 3
echo ">> foreground app:"
adb shell "dumpsys activity activities | grep -m1 ResumedActivity" | tr -d '\r'
adb exec-out screencap -p > "$OUT/gallery-proof.png"
echo ">> screenshot saved: $OUT/gallery-proof.png"
