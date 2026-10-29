#!/usr/bin/env bash
# PHYSICAL CROSS-DEVICE CARRY, mac -> Samsung Galaxy A04 (SM-A045F, Android 13),
# over real LAN TLS 1.3 (two machines, two NICs). The PhotoCraft GUI is
# desktop-only, so the destination is the HEADLESS path: the device imports the
# cut into its own carryon core, reconstructs the real flagship document from
# its store by digest, and the on-device PhotoCraft engine (the aarch64
# `photocraft-cli` cross-built in Phase 1) renders the carried dirty state —
# the composite-pixel sha256 must equal the source's (the Phase-2/3 oracle,
# now computed ON the device).
#
# Physical evidence for the flagship document carry (PLAT-001 style): two
# machines, two NICs. No APK/signing (PLAT-006 out of scope).
#
# Honest scoping: destination verification is headless (engine/CLI), NOT a GUI
# restore — the GUI cannot run on Android (MOBILE-FEASIBILITY.md §4).
set -euo pipefail

WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation/photocraft-live}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/Volumes/ADATA ELITE SE880/compilation/target-carryon}"
PHYS_TARGET_DIR="${PHYS_TARGET_DIR:-/Volumes/ADATA ELITE SE880/compilation/target-phys}"
PHASE1_ARTIFACTS="${PHASE1_ARTIFACTS:-/Volumes/ADATA ELITE SE880/compilation/photocraft-phase1-artifacts}"
PORT="${PORT:-48964}"
DEV_DIR="${DEV_DIR:-/data/local/tmp/carryon-photocraft}"
MAC_IP="${MAC_IP:?set MAC_IP to this host LAN ip}"
PHOTOCRAFT_BIN="${PHOTOCRAFT_BIN:-/Volumes/ADATA ELITE SE880/compilation/target-host/debug/photocraft}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
EVIDENCE="$REPO/docs/artcraft-bridge/evidence/phys-carry"

SRC_BIN="$CARGO_TARGET_DIR/debug/examples/phys_source"
DEST_BIN="$PHYS_TARGET_DIR/aarch64-linux-android/debug/examples/phys_dest"
CLI_BIN="$PHASE1_ARTIFACTS/aarch64-linux-android/photocraft-cli"

mkdir -p "$WORK/phys" "$EVIDENCE"
adb shell "mkdir -p $DEV_DIR/id-src $DEV_DIR/data $DEV_DIR/id-dst"

echo ">> cleanup previous run"
adb shell "rm -rf $DEV_DIR/data/*" >/dev/null 2>&1 || true
pkill -9 -f "photocraft-live/phys" 2>/dev/null || true
rm -rf "$WORK/phys"

echo ">> launch the live source app (control channel on 7878)"
APP_ROOT="$WORK/phys/app-a"
mkdir -p "$APP_ROOT"
"$PHOTOCRAFT_BIN" --control 7878 --control-token-file "$WORK/phys/a.token" \
    --automation-read-root "$APP_ROOT" --automation-write-root "$APP_ROOT" \
    > "$WORK/phys/app-a.log" 2>&1 &
APP_PID=$!
trap 'kill -9 $APP_PID 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do [ -f "$WORK/phys/a.token" ] && break; sleep 0.5; done

cat > "$WORK/phys/control-cfg.json" <<EOF
{"addr":"127.0.0.1:7878","token_file":"$WORK/phys/a.token","read_root":"$APP_ROOT","write_root":"$APP_ROOT"}
EOF

echo ">> push device binaries (phys_dest + photocraft-cli)"
adb push "$DEST_BIN" "$DEV_DIR/phys_dest" >/dev/null
adb push "$CLI_BIN" "$DEV_DIR/photocraft-cli" >/dev/null
adb shell "chmod 755 $DEV_DIR/phys_dest $DEV_DIR/photocraft-cli"

echo ">> pass 1: exchange stable pins"
MAC_PIN=$("$SRC_BIN" pin "$WORK/phys/id-src")
DEV_PIN=$(adb shell "cd $DEV_DIR && ./phys_dest pin $DEV_DIR/id-dst" | tr -d '\r')
echo "   MAC_PIN=${MAC_PIN:0:16}…  DEV_PIN=${DEV_PIN:0:16}…"

echo ">> pass 2: source pulls live state, seals, listens on the LAN"
"$SRC_BIN" serve "$WORK/phys/id-src" "$WORK/phys/data-src" "0.0.0.0:$PORT" "$DEV_PIN" \
    "$WORK/phys/src-evidence.json" "$WORK/phys/digests.json" "$WORK/phys/control-cfg.json" \
    > "$WORK/phys/src.log" 2>&1 &
SRC_PID=$!
for _ in $(seq 1 100); do grep -q "waiting for destination" "$WORK/phys/src.log" && break; sleep 0.2; done
SESS=$(grep -oE 'session=[0-9a-f-]+' "$WORK/phys/src.log" | head -1 | cut -d= -f2 || true)
echo "   source session=$SESS"

echo ">> push digests; device imports over LAN TLS and re-verifies its store"
adb push "$WORK/phys/digests.json" "$DEV_DIR/digests.json" >/dev/null
adb shell "cd $DEV_DIR && ./phys_dest dest $DEV_DIR/id-dst $DEV_DIR/data $MAC_IP:$PORT $MAC_PIN $SESS $DEV_DIR/digests.json $DEV_DIR/dest-evidence.json"
wait "$SRC_PID" || true
COMPOSITE=$(grep -oE 'COMPOSITE_SHA=[0-9a-f]+' "$WORK/phys/src.log" | head -1 | cut -d= -f2 || true)
echo "   source composite sha256=$COMPOSITE"

echo ">> on-device verification: the PhotoCraft engine renders the carried state"
adb shell "cd $DEV_DIR && ./photocraft-cli convert data/carried.pcraft composite.png"
DEV_SHA=$(adb shell "cd $DEV_DIR && sha256sum composite.png" | tr -d '\r' | awk '{print $1}')
echo "   device composite sha256=$DEV_SHA"
if [ "$DEV_SHA" = "$COMPOSITE" ]; then
    echo "   COMPOSITE PIXEL HASH: MATCH"
    ORACLE=match
else
    echo "   COMPOSITE PIXEL HASH: MISMATCH (expected $COMPOSITE)"
    ORACLE=mismatch
fi

echo ">> record device specs + pull evidence"
MODEL=$(adb shell getprop ro.product.model | tr -d '\r')
RELEASE=$(adb shell getprop ro.build.version.release | tr -d '\r')
ABI=$(adb shell getprop ro.product.cpu.abi | tr -d '\r')
HW=$(adb shell "cat /proc/cpuinfo | grep -m1 Hardware" | tr -d '\r')
MEM=$(adb shell "grep MemTotal /proc/meminfo" | tr -d '\r')
adb pull "$DEV_DIR/dest-evidence.json" "$EVIDENCE/dest-evidence.json" >/dev/null
adb pull "$DEV_DIR/composite.png" "$EVIDENCE/composite-device.png" >/dev/null
cp "$WORK/phys/src-evidence.json" "$EVIDENCE/src-evidence.json"
cp "$WORK/phys/src.log" "$EVIDENCE/src.log"

python3 - "$EVIDENCE" "$MODEL" "$RELEASE" "$ABI" "$HW" "$MEM" "$ORACLE" "$COMPOSITE" "$DEV_SHA" "$PORT" "$MAC_IP" <<'EOF'
import json, sys
ev = sys.argv[1]
model, release, abi, hw, mem, oracle, csrc, cdev, port, mac_ip = sys.argv[2:12]
src = json.load(open(f"{ev}/src-evidence.json"))
dst = json.load(open(f"{ev}/dest-evidence.json"))
bundle = {
  "experiment": "physical cross-device PhotoCraft carry (mac -> Galaxy A04)",
  "scope": "PHYSICAL (two machines, two NICs; LAN TLS 1.3)",
  "photocraft_pin": "1f7fc0f53696e54da79b95015abb37d7c22cdd1a",
  "transport": {"port": int(port), "source_lan_ip": mac_ip, "tls": "rustls TLS 1.3, pin-based"},
  "device": {"model": model, "android": release, "abi": abi,
              "hardware": hw, "meminfo": mem},
  "source": src,
  "destination": dst,
  "oracle": {
    "composite_pixel_sha256_source": csrc,
    "composite_pixel_sha256_device": cdev,
    "computed_on_device_by": "photocraft-cli convert (aarch64-linux-android, Phase-1 build)",
    "result": oracle,
  },
}
json.dump(bundle, open(f"{ev}/bundle.json", "w"), indent=2)
print("   bundle:", f"{ev}/bundle.json")
EOF

kill -9 $APP_PID 2>/dev/null || true
if [ "$ORACLE" != "match" ]; then
    echo ">> FAIL: composite hash mismatch"
    exit 2
fi
echo ">> PHYSICAL CARRY COMPLETE: composite-pixel oracle MATCH on the device"
