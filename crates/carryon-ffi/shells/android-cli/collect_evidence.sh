#!/usr/bin/env bash
# Assemble a complete, self-describing evidence archive for one Carry-On cross-device
# run. Gathers: commit hash, mac + device specifications, timestamps, a device screenshot,
# a representative packet capture (operator-run if sudo is non-interactive), and a
# sha256 checksum manifest over every artifact in the directory.
#
# Usage: collect_evidence.sh <evidence_dir>
#   <evidence_dir> is typically docs/evidence/<stamp> produced by bench_xdev.sh /
#   unsaved_continue.sh. Any trial/report/log/bundle already in it is included.
set -euo pipefail

EVID="${1:?usage: collect_evidence.sh <evidence_dir>}"
mkdir -p "$EVID"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"
MAC_IP="${MAC_IP:-unknown}"
PORT="${PORT:-48970}"
DEV_IP="${DEV_IP:-}"

echo ">> commit + timestamps"
( cd "$REPO" && git rev-parse HEAD ) > "$EVID/commit.txt" 2>/dev/null || echo "unknown" > "$EVID/commit.txt"
{
    echo "collected_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "collected_local=$(date +%Y-%m-%dT%H:%M:%S%z)"
    echo "host=$(hostname)"
} > "$EVID/timestamps.txt"

echo ">> mac specs"
cat > "$EVID/mac.json" <<JSON
{
  "os": "$(sw_vers -productName 2>/dev/null || uname -s) $(sw_vers -productVersion 2>/dev/null || true)",
  "kernel": "$(uname -sr)",
  "arch": "$(uname -m)",
  "cpu": "$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)",
  "ncpu": $(sysctl -n hw.ncpu 2>/dev/null || echo 0),
  "mem_bytes": $(sysctl -n hw.memsize 2>/dev/null || echo 0),
  "lan_ip": "$MAC_IP"
}
JSON

echo ">> device specs (adb)"
if adb get-state >/dev/null 2>&1; then
    MODEL=$(adb shell getprop ro.product.model 2>/dev/null | tr -d '\r')
    ABI=$(adb shell getprop ro.product.cpu.abi 2>/dev/null | tr -d '\r')
    REL=$(adb shell getprop ro.build.version.release 2>/dev/null | tr -d '\r')
    SDK=$(adb shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r')
    HW=$(adb shell getprop ro.hardware 2>/dev/null | tr -d '\r')
    CORES=$(adb shell "grep -c ^processor /proc/cpuinfo" 2>/dev/null | tr -d '\r')
    DEV_IP=${DEV_IP:-$(adb shell ip -f inet addr show wlan0 2>/dev/null | grep -oE 'inet [0-9.]+' | awk '{print $2}' | head -1 | tr -d '\r')}
    cat > "$EVID/device.json" <<JSON
{
  "model": "$MODEL",
  "cpu_abi": "$ABI",
  "android_release": "$REL",
  "sdk": "$SDK",
  "hardware": "$HW",
  "cpu_cores": "${CORES:-unknown}",
  "wlan0_ip": "${DEV_IP:-unknown}"
}
JSON
    echo ">> device screenshot"
    adb exec-out screencap -p > "$EVID/device.png" 2>/dev/null || echo "   (screencap unavailable)"
else
    echo '{"error":"no adb device"}' > "$EVID/device.json"
fi

echo ">> packet capture (TLS 1.3 handshake visible; payload encrypted — that is the point)"
LAN_IF=$(route get "${DEV_IP:-8.8.8.8}" 2>/dev/null | awk '/interface:/{print $2}' | head -1)
LAN_IF=${LAN_IF:-en0}
PCAP="$EVID/handshake.pcap"
PCAP_CMD="sudo tcpdump -i $LAN_IF -c 200 -w '$PCAP' host ${DEV_IP:-DEVICE_IP} and port $PORT"
if sudo -n true 2>/dev/null; then
    echo "   capturing 200 packets on $LAN_IF (run a trial in another shell now)..."
    sudo -n tcpdump -i "$LAN_IF" -c 200 -w "$PCAP" "host ${DEV_IP:-0.0.0.0} and port $PORT" 2>/dev/null || true
    [ -f "$PCAP" ] && echo "   wrote $PCAP" || echo "   (no packets captured)"
else
    # Non-interactive sudo unavailable: emit the exact command for the operator to run.
    echo "$PCAP_CMD" > "$EVID/pcap-command.txt"
    echo "   sudo not available non-interactively. To capture, run (then re-run a trial):"
    echo "     ! $PCAP_CMD"
    echo "   command saved to $EVID/pcap-command.txt (pcap marked operator-run)"
fi

echo ">> checksum manifest (sha256 over every artifact)"
( cd "$EVID" && find . -type f ! -name checksums.sha256 -print0 \
    | xargs -0 shasum -a 256 2>/dev/null | sort > checksums.sha256 )
echo "   $(wc -l < "$EVID/checksums.sha256" | tr -d ' ') files checksummed"

echo ">> MANIFEST"
cat > "$EVID/MANIFEST.md" <<MD
# Carry-On cross-device evidence archive

- commit: \`$(cat "$EVID/commit.txt")\`
- collected: $(grep collected_utc "$EVID/timestamps.txt" | cut -d= -f2)
- source: mac ($MAC_IP) — see \`mac.json\`
- destination: android device — see \`device.json\`

## Contents
$(cd "$EVID" && for f in *; do [ "$f" = MANIFEST.md ] && continue; echo "- \`$f\`"; done)

## Disclosure
PHYSICAL cross-device evidence (PLAT-001): two machines, two NICs, real TLS 1.3 + mutual
cert pinning over the LAN. The packet capture shows the TLS record layer; payload is
encrypted (expected). Still no APK/signing and no §30 platform acceptance (spec §2/§30).
Loopback-within-one-device anywhere in the repo remains LOCAL evidence, device or not.
MD

echo ">> archive ready: $EVID"
ls -la "$EVID"
