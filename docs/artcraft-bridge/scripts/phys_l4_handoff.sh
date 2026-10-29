#!/usr/bin/env bash
# PHYSICAL L4 AUTHORITY TRANSFER, mac -> Samsung Galaxy A04 (SM-A045F,
# Android 13), over real LAN TLS 1.3 (two machines, two NICs).
#
# The Phase-4 gap-closer: §21.2 single-writer authority moves to the DEVICE.
# The source is a LIVE PhotoCraft app (control channel); the cut — with the
# SPLIT document encoding (manifest + tiles, TILE-SPLITTING.md) — is served
# over LAN TLS; the device imports, reconstructs from its own store by digest,
# proves the composite-pixel oracle ON-DEVICE, durably accepts authority
# (epoch 0->1), and then — in a FRESH process, with the mac's app killed and
# its core reopened read-only — keeps editing through its own core+engine.
#
# Prerequisites (prebuilt, phys_handoff.sh convention):
#   SRC_BIN  = $CARGO_TARGET_DIR/debug/examples/phys_l4_source        (host)
#   DEST_BIN = $PHYS_TARGET_DIR/aarch64-linux-android/debug/examples/phys_l4_dest
#   PHOTOCRAFT_BIN = the desktop app (host build of the pin)
# Cross-build the device binary with the NDK (see logs/build-android-cli.sh
# for the toolchain env):
#   cargo build -p carryon-adapter-photocraft --features inprocess \
#     --target aarch64-linux-android --example phys_l4_dest
#
# Honest scoping: no APK/signing (PLAT-006 out of scope); the destination
# verification is the headless engine path, not a GUI (MOBILE-FEASIBILITY §4).
set -euo pipefail

WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation/photocraft-live}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/Volumes/ADATA ELITE SE880/compilation/target-carryon}"
PHYS_TARGET_DIR="${PHYS_TARGET_DIR:-/Volumes/ADATA ELITE SE880/compilation/target-phys}"
PORT="${PORT:-48974}"
DEV_DIR="${DEV_DIR:-/data/local/tmp/carryon-photocraft-l4}"
MAC_IP="${MAC_IP:?set MAC_IP to this host LAN ip}"
PHOTOCRAFT_BIN="${PHOTOCRAFT_BIN:-/Volumes/ADATA ELITE SE880/compilation/target-host/debug/photocraft}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
EVIDENCE="$REPO/docs/artcraft-bridge/evidence/phys-l4"

SRC_BIN="$CARGO_TARGET_DIR/debug/examples/phys_l4_source"
DEST_BIN="$PHYS_TARGET_DIR/aarch64-linux-android/debug/examples/phys_l4_dest"

mkdir -p "$WORK/phys-l4" "$EVIDENCE"
adb shell "mkdir -p $DEV_DIR/id-src $DEV_DIR/id-dst $DEV_DIR/data $DEV_DIR/work"

echo ">> cleanup previous run"
adb shell "rm -rf $DEV_DIR/data/*" >/dev/null 2>&1 || true
pkill -9 -f "photocraft-live/phys-l4" 2>/dev/null || true
rm -rf "$WORK/phys-l4"

echo ">> launch the live source app (control channel on 7878)"
APP_ROOT="$WORK/phys-l4/app-a"
mkdir -p "$APP_ROOT"
"$PHOTOCRAFT_BIN" --control 7878 --control-token-file "$WORK/phys-l4/a.token" \
    --automation-read-root "$APP_ROOT" --automation-write-root "$APP_ROOT" \
    > "$WORK/phys-l4/app-a.log" 2>&1 &
APP_PID=$!
trap 'kill -9 $APP_PID 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do [ -f "$WORK/phys-l4/a.token" ] && break; sleep 0.5; done

cat > "$WORK/phys-l4/control-cfg.json" <<EOF
{"addr":"127.0.0.1:7878","token_file":"$WORK/phys-l4/a.token","read_root":"$APP_ROOT","write_root":"$APP_ROOT"}
EOF

echo ">> push the device binary"
adb push "$DEST_BIN" "$DEV_DIR/phys_l4_dest" >/dev/null
adb shell "chmod 755 $DEV_DIR/phys_l4_dest"

echo ">> pass 1: exchange stable pins"
MAC_PIN="$("$SRC_BIN" pin "$WORK/phys-l4/id-src")"
DEV_PIN="$(adb shell "cd $DEV_DIR && ./phys_l4_dest pin $DEV_DIR/id-dst" | tr -d '\r')"
echo "   MAC_PIN=${MAC_PIN:0:16}…  DEV_PIN=${DEV_PIN:0:16}…"

echo ">> pass 2: source pulls the live dirty session, seals, serves cut + authority on the LAN"
"$SRC_BIN" serve "$WORK/phys-l4/id-src" "$WORK/phys-l4/data-src" "0.0.0.0:$PORT" "$DEV_PIN" \
    "$WORK/phys-l4/src-evidence.json" "$WORK/phys-l4/digests.json" "$WORK/phys-l4/control-cfg.json" \
    > "$WORK/phys-l4/src.log" 2>&1 &
SRC_PID=$!
for _ in $(seq 1 100); do grep -q "waiting for destination" "$WORK/phys-l4/src.log" && break; sleep 0.2; done
SESS=$(grep -oE 'session=[0-9a-f-]+' "$WORK/phys-l4/src.log" | head -1 | cut -d= -f2 || true)
echo "   source session=$SESS"

echo ">> push digests; device imports over LAN TLS, proves the oracle on-device, takes authority"
adb push "$WORK/phys-l4/digests.json" "$DEV_DIR/digests.json" >/dev/null
adb shell "cd $DEV_DIR && ./phys_l4_dest take $DEV_DIR/id-dst $DEV_DIR/data $MAC_IP:$PORT $MAC_PIN $SESS $DEV_DIR/digests.json $DEV_DIR/take-evidence.json"
wait "$SRC_PID" || true

echo ">> SOURCE-OFF: SIGKILL the live app; the mac core must reopen read-only"
kill -9 "$APP_PID" || true
wait "$APP_PID" 2>/dev/null || true
MAC_COMPOSITE=$(grep -oE 'COMPOSITE_SHA=[0-9a-f]+' "$WORK/phys-l4/src.log" | head -1 | cut -d= -f2 || true)
"$SRC_BIN" reopen "$WORK/phys-l4/data-src" "$WORK/phys-l4/reopen-evidence.json"
cat "$WORK/phys-l4/reopen-evidence.json"

echo ">> relaunch the source app (alive, read-only-paired: its core refuses)"
APP_ROOT2="$WORK/phys-l4/app-a2"
mkdir -p "$APP_ROOT2"
"$PHOTOCRAFT_BIN" --control 7879 --control-token-file "$WORK/phys-l4/a2.token" \
    --automation-read-root "$APP_ROOT2" --automation-write-root "$APP_ROOT2" \
    > "$WORK/phys-l4/app-a2.log" 2>&1 &
APP2_PID=$!
for _ in $(seq 1 120); do
    [ -f "$WORK/phys-l4/a2.token" ] && nc -z 127.0.0.1 7879 2>/dev/null && break
    sleep 0.5
done
cat > "$WORK/phys-l4/control-cfg2.json" <<EOF
{"addr":"127.0.0.1:7879","token_file":"$WORK/phys-l4/a2.token","read_root":"$APP_ROOT2","write_root":"$APP_ROOT2"}
EOF
"$SRC_BIN" probe "$WORK/phys-l4/control-cfg2.json" "shot-a-restarted.png"
cp "$APP_ROOT2/shot-a-restarted.png" "$EVIDENCE/shot-a-restarted.png" 2>/dev/null     || echo "   NOTE: shot-a-restarted.png not captured (screenshot request failed; the alive proof is the probe evidence)"
kill -9 "$APP2_PID" 2>/dev/null || true
wait "$APP2_PID" 2>/dev/null || true

echo ">> device continues editing in a FRESH process, source gone"
adb shell "cd $DEV_DIR && ./phys_l4_dest continue $DEV_DIR/data $DEV_DIR/work $SESS $DEV_DIR/digests.json $DEV_DIR/continue-evidence.json"

echo ">> compare the on-device oracle with the source's composite"
DEV_COMPOSITE=$(adb shell "cat $DEV_DIR/take-evidence.json" | tr -d '\r' | grep -oE '"composite_png_sha256_on_device": "[0-9a-f]+' | cut -d'"' -f4 || true)
echo "   source composite sha256=$MAC_COMPOSITE"
echo "   device composite sha256=$DEV_COMPOSITE"
if [ "$DEV_COMPOSITE" = "$MAC_COMPOSITE" ]; then
    echo "   COMPOSITE PIXEL HASH: MATCH"
    ORACLE=match
else
    echo "   COMPOSITE PIXEL HASH: MISMATCH (expected $MAC_COMPOSITE)"
    ORACLE=mismatch
fi

echo ">> record device specs + pull evidence"
MODEL=$(adb shell getprop ro.product.model | tr -d '\r')
RELEASE=$(adb shell getprop ro.build.version.release | tr -d '\r')
ABI=$(adb shell getprop ro.product.cpu.abi | tr -d '\r')
HW=$(adb shell "cat /proc/cpuinfo | grep -m1 Hardware" | tr -d '\r')
MEM=$(adb shell "grep MemTotal /proc/meminfo" | tr -d '\r')
adb pull "$DEV_DIR/take-evidence.json" "$EVIDENCE/take-evidence.json" >/dev/null
adb pull "$DEV_DIR/continue-evidence.json" "$EVIDENCE/continue-evidence.json" >/dev/null
cp "$WORK/phys-l4/src-evidence.json" "$EVIDENCE/src-evidence.json"
cp "$WORK/phys-l4/reopen-evidence.json" "$EVIDENCE/reopen-evidence.json"
cp "$WORK/phys-l4/control-cfg2.probe-evidence.json" "$EVIDENCE/probe-evidence.json"
cp "$WORK/phys-l4/src.log" "$EVIDENCE/src.log"

python3 - "$EVIDENCE" "$MODEL" "$RELEASE" "$ABI" "$HW" "$MEM" "$ORACLE" "$MAC_COMPOSITE" "$DEV_COMPOSITE" "$PORT" "$MAC_IP" <<'EOF'
import json, sys
ev = sys.argv[1]
model, release, abi, hw, mem, oracle, csrc, cdev, port, mac_ip = sys.argv[2:12]
src = json.load(open(f"{ev}/src-evidence.json"))
take = json.load(open(f"{ev}/take-evidence.json"))
cont = json.load(open(f"{ev}/continue-evidence.json"))
reopen = json.load(open(f"{ev}/reopen-evidence.json"))
probe = json.load(open(f"{ev}/probe-evidence.json"))
bundle = {
  "experiment": "physical L4 authority transfer (mac -> Galaxy A04)",
  "scope": "PHYSICAL (two machines, two NICs; LAN TLS 1.3, pin-based)",
  "photocraft_pin": "1f7fc0f53696e54da79b95015abb37d7c22cdd1a",
  "transport": {"port": int(port), "source_lan_ip": mac_ip, "tls": "rustls TLS 1.3, pin-based"},
  "device": {"model": model, "android": release, "abi": abi,
              "hardware": hw, "meminfo": mem},
  "flow": [
    "live app (control channel) -> source core seals split cut (manifest + tiles)",
    "LAN TLS serve: cut + §21.2 authority transfer (same session)",
    "device imports, reconstructs from its own store by digest, on-device composite oracle, accepts epoch 1",
    "mac SIGKILLs the live app; reopened mac core: may_mutate=false, guard refused",
    "relaunched mac app alive (read-only-paired: the CORE refuses)",
    "device FRESH process: re-owns, document.edit source-off, 2 steps",
  ],
  "source": src,
  "destination_take": take,
  "destination_continue": cont,
  "source_reopen_read_only": reopen,
  "source_relaunch_probe": probe,
  "oracle": {
    "composite_pixel_sha256_source": csrc,
    "composite_pixel_sha256_device_before_edits": cdev,
    "computed_on_device_by": "photocraft-engine embedded in phys_l4_dest (aarch64-linux-android)",
    "result": oracle,
  },
}
json.dump(bundle, open(f"{ev}/bundle.json", "w"), indent=2)
print("   bundle:", f"{ev}/bundle.json")
EOF

if [ "$ORACLE" != "match" ]; then
    echo ">> FAIL: composite hash mismatch"
    exit 2
fi
echo ">> PHYSICAL L4 COMPLETE: authority on the device, source-off edit OK, oracle MATCH"
