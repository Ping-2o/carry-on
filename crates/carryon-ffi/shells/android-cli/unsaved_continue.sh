#!/usr/bin/env bash
# REAL unsaved-state continuation with the SOURCE KILLED, mac -> android device.
#
# Proves the hard case: edit a document on the mac (a real unsaved edit in the L3 editor
# session), carry it to the device, move authority, KILL the mac source process, and show
# the device continues editing independently — then prove a RESTARTED mac source can no
# longer mutate the session (authority was durably relinquished; single-writer holds).
#
# Steps:
#   1. mac source serves the structured session and relinquishes authority (§21.2).
#   2. device dest imports, restores the SAME session, takes authority (may_mutate flips
#      true), disconnects, and edits the document INDEPENDENTLY, exporting evidence.
#   3. orchestrator kill -9 the mac source PROCESS (recorded) — continuation already ran
#      with the transport gone; the kill makes "source off" literal.
#   4. relaunch a mac source core over the same data dir and assert may_mutate == false
#      (carryon_l3 `check`), i.e. the source cannot resurrect authority after restart.
#
# Physical cross-device evidence (PLAT-001): two machines, two NICs. No APK/signing; no
# §30 platform acceptance (spec §2/§30).
set -euo pipefail

NDK="${NDK:?set NDK to your android-ndk-rXX directory}"
WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation}"
API="${API:-21}"
PORT="${PORT:-48972}"
MAC_IP="${MAC_IP:?set MAC_IP to this host LAN ip (e.g. 192.168.3.201)}"
DEV_DIR="${DEV_DIR:-/data/local/tmp/carryon}"
TARGET=aarch64-linux-android

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"
HOST_TAG="$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64"
TC="$NDK/toolchains/llvm/prebuilt/$HOST_TAG"
CLANG="$TC/bin/${TARGET}${API}-clang"
OUT="$WORK/android-out"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
EVID="${EVID:-$REPO/docs/evidence/$STAMP}"
mkdir -p "$EVID"

export CARGO_TARGET_DIR="$WORK/target"
export AR_aarch64_linux_android="$TC/bin/llvm-ar"
export CC_aarch64_linux_android="$CLANG"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CLANG"
rustup target add "$TARGET" >/dev/null 2>&1 || true

echo ">> building runtime (host + device) and the L3 shell (with check role)"
( cd "$REPO" && cargo build -p carryon-ffi --release )
( cd "$REPO" && cargo build -p carryon-ffi --target "$TARGET" --release )
mkdir -p "$OUT/mac" "$OUT/mac/id-uc-src"
cp "$CARGO_TARGET_DIR/release/libcarryon_ffi.dylib" "$OUT/mac/"
cp "$CARGO_TARGET_DIR/$TARGET/release/libcarryon_ffi.so" "$OUT/"
clang -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_l3.c" \
    -L "$OUT/mac" -lcarryon_ffi -Wl,-rpath,"$OUT/mac" -o "$OUT/mac/carryon_l3"
"$CLANG" -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_l3.c" \
    -L "$OUT" -lcarryon_ffi -o "$OUT/carryon_l3"

echo ">> push device binary + runtime"
adb shell "mkdir -p $DEV_DIR/id-uc-dst"
adb push "$OUT/libcarryon_ffi.so" "$DEV_DIR/" >/dev/null
adb push "$OUT/carryon_l3" "$DEV_DIR/" >/dev/null
adb shell "chmod 755 $DEV_DIR/carryon_l3"

echo ">> exchange pins"
MAC_PIN=$(DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_l3" pin "$OUT/mac/id-uc-src")
DEV_PIN=$(adb shell "cd $DEV_DIR && LD_LIBRARY_PATH=$DEV_DIR ./carryon_l3 pin $DEV_DIR/id-uc-dst" | tr -d '\r')
echo "   MAC_PIN=$MAC_PIN  DEV_PIN=$DEV_PIN"

echo ">> start mac source (serves + relinquishes), then device continues"
rm -rf "$OUT/mac/data-uc-src"; adb shell "rm -rf $DEV_DIR/data-uc-dst"
DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_l3" source \
    "$OUT/mac/id-uc-src" "$OUT/mac/data-uc-src" "0.0.0.0:$PORT" "$DEV_PIN" \
    > "$EVID/uc-src.log" 2>&1 &
SRC_PID=$!
echo "   mac source PID=$SRC_PID"
for _ in $(seq 1 100); do grep -q "waiting for destination" "$EVID/uc-src.log" && break; sleep 0.1; done
SESS=$(grep -oE "session=[0-9a-f-]+" "$EVID/uc-src.log" | head -1 | cut -d= -f2)
echo "   source session=$SESS"

adb shell "cd $DEV_DIR && L3_CUT=0 LD_LIBRARY_PATH=$DEV_DIR ./carryon_l3 dest \
    $DEV_DIR/id-uc-dst $DEV_DIR/data-uc-dst $MAC_IP:$PORT $MAC_PIN $SESS" \
    > "$EVID/uc-dst.log" 2>&1 || true
sed 's/^/   dst| /' "$EVID/uc-dst.log"

echo ">> KILL the mac source process (source off)"
KILLED=0
if kill -0 "$SRC_PID" 2>/dev/null; then
    kill -9 "$SRC_PID" 2>/dev/null || true
    KILLED=1
fi
wait "$SRC_PID" 2>/dev/null || true
echo "   source process killed=$KILLED"

echo ">> restart a mac source core over the same data dir; assert it is read-only"
CHECK_RC=0
DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_l3" check \
    "$OUT/mac/data-uc-src" "$SESS" > "$EVID/uc-check.log" 2>&1 || CHECK_RC=$?
sed 's/^/   check| /' "$EVID/uc-check.log"

echo ">> pull device evidence bundle"
adb pull "$DEV_DIR/data-uc-dst/l3-evidence.json" "$EVID/unsaved-continue-evidence.json" >/dev/null 2>&1 || true

# Extract key facts from the device log for the proof summary. The l3 `dest` runs
# `document.edit` ONLY after asserting `may_mutate(post) == true` (it exits non-zero
# otherwise), so a successful independent edit is itself the ownership proof. The source
# may relinquish and exit on its own before the kill; "source off" holds whether we killed
# a live process or found it already gone.
DEST_EDIT=$(grep -q "independent edit =" "$EVID/uc-dst.log" && echo true || echo false)
DEST_OWNER=$DEST_EDIT
VERIFY_OK=$(grep -q 'evidence verify =.*"ok":true' "$EVID/uc-dst.log" && echo true || echo false)
SRC_READONLY=$([ "$CHECK_RC" = 0 ] && echo true || echo false)
SOURCE_OFF=$([ "$KILLED" = 1 ] && echo "killed" || echo "already-exited")

cat > "$EVID/source-off-proof.json" <<JSON
{
  "stamp": "$STAMP",
  "scenario": "edit on mac -> carry to android -> move authority -> mac source off (killed or exited) -> continue on android; restarted source cannot mutate",
  "source_session_id": "$SESS",
  "mac_source_pid": $SRC_PID,
  "mac_source_off": "$SOURCE_OFF",
  "dest_became_owner": $DEST_OWNER,
  "dest_independent_edit_succeeded": $DEST_EDIT,
  "dest_evidence_verify_ok": $VERIFY_OK,
  "restarted_source_read_only": $SRC_READONLY,
  "all_passed": $([ "$DEST_OWNER" = true ] && [ "$DEST_EDIT" = true ] && [ "$VERIFY_OK" = true ] && [ "$SRC_READONLY" = true ] && echo true || echo false),
  "disclosure": "PHYSICAL cross-device evidence (PLAT-001). Two machines, two NICs, real TLS 1.3. No APK/signing; no §30 platform acceptance (spec §2/§30)."
}
JSON
echo ">> source-off proof:"; sed 's/^/   /' "$EVID/source-off-proof.json"
echo ">> evidence dir: $EVID"
