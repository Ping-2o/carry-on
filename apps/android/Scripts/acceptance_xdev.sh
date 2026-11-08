#!/usr/bin/env bash
# acceptance_xdev.sh — the DEVICE-PHASE §30 acceptance runner (stage 8).
#
# It is intentionally DISARMED by default: the Galaxy A04 is booked by the
# stage-6 (handoff e2e) session until the orchestrator signals otherwise.
# The script refuses to touch adb unless the operator explicitly sets:
#
#   CARRYON_ACCEPTANCE_DEVICE=1
#
# What it runs once armed:
#   1. the on-device instrumented suites (FFI smoke / demo / handoff / pairing /
#      discovery / Keystore) via gradle connectedDebugAndroidTest;
#   2. the NET-003 discovery timing gate: app-open → discovered peer < 10 s,
#      measured on the real LAN with the mac advertising (stage-4 pilots);
#   3. the engine-level acceptance harness against the phone over adb-forwarded
#      loopback is NOT used — §30 wants the real LAN, so the phone-side app
#      drives its own engine and the mac side is the peer (see README).
#
# LABELS ARE LAW (§2/§30): results from here are PHYSICAL only when two real
# machines/NICs are involved; a run with the phone in the same process is
# LOCAL (device-local). This script never upgrades a platform to "supported" —
# PLAT-001 passes only after the full matrix (incl. PLAT-006 signed artifacts,
# which stage 7 skipped) is green.
#
# Usage:
#   CARRYON_ACCEPTANCE_DEVICE=1 Scripts/acceptance_xdev.sh [out_dir]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
OUT="${1:-$REPO/target/acceptance-xdev}"

if [ "${CARRYON_ACCEPTANCE_DEVICE:-0}" != "1" ]; then
  cat >&2 <<'EOF'
REFUSED: the A04 is reserved for the stage-6 handoff session.

Set CARRYON_ACCEPTANCE_DEVICE=1 ONLY when the orchestrator releases the
device. Nothing was executed; adb was not touched.
EOF
  exit 3
fi

ADB="${ADB:-adb}"
SERIAL="${ANDROID_SERIAL:-}"
if [ -z "$SERIAL" ]; then
  SERIAL="$("$ADB" devices | awk 'NR>1 && $2=="device" {print $1; exit}')"
fi
[ -n "$SERIAL" ] || { echo "FAIL: no adb device (set ANDROID_SERIAL)" >&2; exit 1; }

mkdir -p "$OUT"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
echo ">> device: $SERIAL  out: $OUT  stamp: $STAMP"

# ---- 1. on-device instrumented suites --------------------------------------
echo ">> instrumented suites (connectedDebugAndroidTest)"
JAVA_HOME="${JAVA_HOME:-/opt/homebrew/Cellar/openjdk@17/17.0.19/libexec/openjdk.jdk/Contents/Home}"
export JAVA_HOME
( cd "$REPO/apps/android" && ANDROID_SERIAL="$SERIAL" ./gradlew --no-daemon --console=plain \
    :app:connectedDebugAndroidTest ) 2>&1 | tee "$OUT/instrumented.log" | tail -3

# ---- 2. NET-003 discovery timing gate -------------------------------------
# The mac runs the stage-4 discovery pilot (Bonjour advertise) and the phone
# app browses; time from app-open to first peer row. The exact procedure is
# documented in the stage-6/stage-8 runbooks — measured by hand + screenshots
# because a wall-clock UI gate cannot be claimed from a headless script alone.
echo ">> NET-003: measure app-open → first discovered peer (< 10 s) on the LAN"
echo "   (manual stopwatch + screenshot; record below)"
printf 'net003_seconds=%s\n' "${NET003_SECONDS:-UNMEASURED}" >> "$OUT/net003.txt"

cat > "$OUT/README.txt" <<EOF
§30 device-phase run — $STAMP
device: $SERIAL
instrumented: see instrumented.log (PHYSICAL only if the mac was the peer)
NET-003: see net003.txt (gate: < 10 s)
NOT run yet by design: PLAT-006 signed artifacts (stage 7 skipped); final
platform verdict requires that matrix — §30 is NOT complete until then.
EOF

echo ">> wrote $OUT"
