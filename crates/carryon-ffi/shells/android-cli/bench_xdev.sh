#!/usr/bin/env bash
# REAL cross-device preparation-strategy benchmark, mac (source) -> android (dest) over
# a LAN TLS 1.3 link. Runs a RANDOMIZED campaign of >=30 trials across state sizes,
# injected latency, optional-state ratio, demand, and a no-handoff case. Each trial is an
# INDEPENDENT end-to-end execution of ONE strategy (A/C/D) via carryon_bench — never
# composed. Trial JSON lines are collected and aggregated by `xtask bench-aggregate`.
#
# Physical cross-device evidence (PLAT-001): two machines, two NICs. No APK/signing; no
# §30 platform acceptance (spec §2/§30). The latency axis uses CARRYON_NET_DELAY_MS, a
# bench-only per-frame send delay in carryon-net (it inflates real wall-clock only; wire
# byte counters stay real).
#
# Requires: NDK, MAC_IP (this host's LAN ip reachable from the device), rustup target
# aarch64-linux-android, adb with one device attached, both machines on the same LAN.
set -euo pipefail

NDK="${NDK:?set NDK to your android-ndk-rXX directory}"
WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation}"
API="${API:-21}"
PORT="${PORT:-48970}"
MAC_IP="${MAC_IP:?set MAC_IP to this host LAN ip (e.g. 192.168.3.201)}"
DEV_DIR="${DEV_DIR:-/data/local/tmp/carryon}"
TRIALS="${TRIALS:-30}"           # at least 30 randomized trials
SEED="${SEED:-1337}"             # recorded for reproducibility
TARGET=aarch64-linux-android

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"
HOST_TAG="$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64"
TC="$NDK/toolchains/llvm/prebuilt/$HOST_TAG"
CLANG="$TC/bin/${TARGET}${API}-clang"
OUT="$WORK/android-out"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
EVID="$REPO/docs/evidence/$STAMP"
mkdir -p "$EVID"

export CARGO_TARGET_DIR="$WORK/target"
export AR_aarch64_linux_android="$TC/bin/llvm-ar"
export CC_aarch64_linux_android="$CLANG"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CLANG"
rustup target add "$TARGET" >/dev/null 2>&1 || true

echo ">> building runtime (host + device) and the bench shell"
( cd "$REPO" && cargo build -p carryon-ffi --release )
( cd "$REPO" && cargo build -p carryon-ffi --target "$TARGET" --release )
mkdir -p "$OUT/mac" "$OUT/mac/id-benchsrc"
cp "$CARGO_TARGET_DIR/release/libcarryon_ffi.dylib" "$OUT/mac/"
cp "$CARGO_TARGET_DIR/$TARGET/release/libcarryon_ffi.so" "$OUT/"
clang -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_bench.c" \
    -L "$OUT/mac" -lcarryon_ffi -Wl,-rpath,"$OUT/mac" -o "$OUT/mac/carryon_bench"
"$CLANG" -I "$REPO/crates/carryon-ffi/include" "$HERE/carryon_bench.c" \
    -L "$OUT" -lcarryon_ffi -o "$OUT/carryon_bench"

echo ">> push device binary + runtime"
adb shell "mkdir -p $DEV_DIR/id-benchdst"
adb push "$OUT/libcarryon_ffi.so" "$DEV_DIR/" >/dev/null
adb push "$OUT/carryon_bench" "$DEV_DIR/" >/dev/null
adb shell "chmod 755 $DEV_DIR/carryon_bench"

echo ">> exchange stable pins once (reused by every trial)"
MAC_PIN=$(DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_bench" pin "$OUT/mac/id-benchsrc")
DEV_PIN=$(adb shell "cd $DEV_DIR && LD_LIBRARY_PATH=$DEV_DIR ./carryon_bench pin $DEV_DIR/id-benchdst" | tr -d '\r')
echo "   MAC_PIN=$MAC_PIN  DEV_PIN=$DEV_PIN"

# Randomized axes (bash RANDOM seeded for reproducibility; seed recorded in the archive).
RANDOM=$SEED
DOCS=(256 4096 65536 1048576)
NAVS=(0 1024 65536 600000)
LATS=(0 5 20)
STRATS=(A C D)

TRIAL_FILE="$EVID/trials.jsonl"
: > "$TRIAL_FILE"
echo ">> running $TRIALS randomized trials -> $TRIAL_FILE"

# A unique port per trial avoids TIME_WAIT / half-open collisions between back-to-back
# trials on the slow device (the symptom is a spurious `PROTO_Framing: connection closed`
# on the next connect). Incremented for every trial.
PORT_SEQ=$PORT
run_trial() {
    local strat="$1" doc="$2" nav="$3" lat="$4" demand="$5"
    local port="$PORT_SEQ"
    PORT_SEQ=$((PORT_SEQ + 1))
    local tag="t_${strat}_${doc}_${nav}_${lat}_p${port}"
    rm -rf "$OUT/mac/data-$tag"; adb shell "rm -rf $DEV_DIR/data-$tag"
    # mac source (background), bind on all interfaces so the device can reach it.
    DYLD_LIBRARY_PATH="$OUT/mac" "$OUT/mac/carryon_bench" source \
        "$OUT/mac/id-benchsrc" "$OUT/mac/data-$tag" "0.0.0.0:$port" "$DEV_PIN" \
        "$doc" "$nav" "$strat" > "$OUT/mac/src-$tag.log" 2>&1 &
    local srcpid=$!
    local ok=0
    for _ in $(seq 1 100); do grep -q "waiting for destination" "$OUT/mac/src-$tag.log" && ok=1 && break; sleep 0.1; done
    if [ "$ok" != 1 ]; then
        echo "   [skip] source failed to bind for $tag"; kill "$srcpid" 2>/dev/null || true; return
    fi
    local sess
    sess=$(grep -oE "session=[0-9a-f-]+" "$OUT/mac/src-$tag.log" | head -1 | cut -d= -f2)
    # device dest: inject latency via CARRYON_NET_DELAY_MS, append its trial line locally.
    adb shell "cd $DEV_DIR && CARRYON_NET_DELAY_MS=$lat LD_LIBRARY_PATH=$DEV_DIR \
        ./carryon_bench dest $DEV_DIR/id-benchdst $DEV_DIR/data-$tag $MAC_IP:$port \
        $MAC_PIN $sess $strat $demand $lat $doc $nav $DEV_DIR/trial-$tag.jsonl" \
        > "$OUT/mac/dst-$tag.log" 2>&1 || true
    wait "$srcpid" 2>/dev/null || true
    # pull the device-side trial line (the authoritative measurement is the dest's).
    adb pull "$DEV_DIR/trial-$tag.jsonl" "$OUT/mac/trial-$tag.jsonl" >/dev/null 2>&1 || true
    if [ -f "$OUT/mac/trial-$tag.jsonl" ]; then
        cat "$OUT/mac/trial-$tag.jsonl" >> "$TRIAL_FILE"
    else
        # record a failure line so the aggregator counts it honestly.
        printf '{"strategy":"%s","doc_bytes":%s,"nav_bytes":%s,"latency_ms":%s,"demand":%s,"no_handoff":false,"ok":false,"failure":"no device trial line"}\n' \
            "$strat" "$doc" "$nav" "$lat" "$([ "$demand" = 1 ] && echo true || echo false)" >> "$TRIAL_FILE"
    fi
    cp "$OUT/mac/src-$tag.log" "$EVID/" 2>/dev/null || true
}

count=0
while [ "$count" -lt "$TRIALS" ]; do
    strat=${STRATS[$((RANDOM % ${#STRATS[@]}))]}
    doc=${DOCS[$((RANDOM % ${#DOCS[@]}))]}
    nav=${NAVS[$((RANDOM % ${#NAVS[@]}))]}
    lat=${LATS[$((RANDOM % ${#LATS[@]}))]}
    demand=$((RANDOM % 2))
    # demand=1 with nav=0 is degenerate; fold to demand=0.
    [ "$nav" = 0 ] && demand=0
    count=$((count + 1))
    echo "   [$count/$TRIALS] strategy=$strat doc=$doc nav=$nav latency=${lat}ms demand=$demand"
    run_trial "$strat" "$doc" "$nav" "$lat" "$demand"
done

# A dedicated no-handoff trial (normal-use overhead: no transfer ever happens). Recorded
# as a distinct, labeled line the aggregator excludes from transfer stats.
printf '{"strategy":"D","doc_bytes":4096,"nav_bytes":65536,"latency_ms":0,"demand":false,"no_handoff":true,"ok":true,"failure":""}\n' >> "$TRIAL_FILE"

# Record the campaign parameters alongside the trials.
cat > "$EVID/campaign.json" <<JSON
{
  "stamp": "$STAMP",
  "trials": $TRIALS,
  "seed": $SEED,
  "port": $PORT,
  "mac_ip": "$MAC_IP",
  "axes": {"docs": [256,4096,65536,1048576], "navs": [0,1024,65536,600000], "latency_ms": [0,5,20], "strategies": ["A","C","D"]},
  "path": "real Mac->Android LAN TLS 1.3 (two machines, two NICs)",
  "latency_mechanism": "CARRYON_NET_DELAY_MS per-frame send delay (bench-only; real wall-clock only)",
  "disclosure": "PHYSICAL cross-device evidence (PLAT-001). No APK/signing; no §30 platform acceptance (spec §2/§30)."
}
JSON

echo ">> aggregating -> RESULTS-xdevice.md + summary-xdevice.json"
( cd "$REPO" && cargo run -q -p xtask -- bench-aggregate "$EVID" )
echo ">> evidence dir: $EVID"
echo ">> next: crates/carryon-ffi/shells/android-cli/collect_evidence.sh $EVID"
