#!/usr/bin/env bash
# photocraft_ux_local.sh — stage-10 desktop flagship UX, LOCAL integration run.
#
# Starts TWO live PhotoCraft desktop apps on this machine, prepares a real photo
# document in A (PNG import -> edits -> native `.pcraft` save -> more edits), then
# runs `photocraft_ux_live`:
#   capture A over its loopback control channel -> split cut (PhotoCraft tiles)
#   -> budget-window transfer into a second core (the UI's per-tile progress)
#   -> restore into live app B -> engine oracle + independent position check.
#
# LABELS ARE LAW: this is OBSERVED (LOCAL) — two app processes and two cores on
# ONE machine, loopback only. There is no second Mac here, so nothing is
# PHYSICAL and nothing upgrades a platform to "supported" (§30 unrun).
#
# Requirements: the pinned PhotoCraft binary (PHOTOCRAFT_BIN), a Cargo toolchain.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
WORK="${WORK:-/Volumes/ADATA ELITE SE880/compilation/photocraft-ux-work}"
EVID="${EVID:-$REPO/docs/evidence/20261108-photocraft-ux-local}"
PHOTOCRAFT_BIN="${PHOTOCRAFT_BIN:-/Volumes/ADATA ELITE SE880/compilation/target-host/debug/photocraft}"
PORT_A="${PORT_A:-7898}"
PORT_B="${PORT_B:-7899}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/Volumes/ADATA ELITE SE880/compilation/target-pairing}"

[ -x "$PHOTOCRAFT_BIN" ] || { echo "missing PhotoCraft binary: $PHOTOCRAFT_BIN" >&2; exit 1; }
rm -rf "$WORK"
mkdir -p "$WORK/a" "$WORK/b" "$EVID"

echo ">> generating a real photo source (2048x1536 PNG)"
python3 - "$WORK/a/photo.png" <<'PY'
import math, struct, sys, zlib
w, h = 2048, 1536
rows = bytearray()
for y in range(h):
    rows.append(0)
    for x in range(w):
        # smooth gradients + a walking band: compresses like a photo, not noise
        r = (x * 255) // w
        g = (y * 255) // h
        b = int(127 + 120 * math.sin((x + y) / 90.0))
        band = 40 if ((x // 64 + y // 64) % 5 == 0) else 0
        rows += bytes(((r + band) % 256, (g + band) % 256, (b + band) % 256))
def chunk(t, d):
    c = struct.pack(">I", len(d)) + t + d
    return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(bytes(rows), 6))
png += chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
print(f"wrote {sys.argv[1]} {len(png)} bytes")
PY

echo ">> launching two live PhotoCraft apps ($PORT_A / $PORT_B)"
"$PHOTOCRAFT_BIN" --control "$PORT_A" --control-token-file "$WORK/a/token" \
    --automation-read-root "$WORK/a" --automation-write-root "$WORK/a" \
    > "$WORK/a/app.log" 2>&1 &
PID_A=$!
"$PHOTOCRAFT_BIN" --control "$PORT_B" --control-token-file "$WORK/b/token" \
    --automation-read-root "$WORK/b" --automation-write-root "$WORK/b" \
    > "$WORK/b/app.log" 2>&1 &
PID_B=$!
trap 'kill -9 $PID_A $PID_B 2>/dev/null || true' EXIT
for _ in $(seq 1 240); do
    [ -s "$WORK/a/token" ] && [ -s "$WORK/b/token" ] && break
    sleep 0.5
done
[ -s "$WORK/a/token" ] && [ -s "$WORK/b/token" ] || { echo "apps never exposed control tokens" >&2; exit 1; }
echo ">> waiting for both control listeners"
python3 - "$PORT_A" "$PORT_B" <<'PY'
import socket, sys, time
ports = [int(sys.argv[1]), int(sys.argv[2])]
for port in ports:
    for _ in range(240):
        try:
            socket.create_connection(("127.0.0.1", port), timeout=1).close()
            break
        except OSError:
            time.sleep(0.5)
    else:
        raise SystemExit(f"control port {port} never accepted")
    print(f"port {port} ready")
PY

echo ">> preparing app A: open the photo, edit, save a native document, edit again"
python3 - "$WORK/a/token" "$PORT_A" <<'PY'
import json, socket, sys
tok = open(sys.argv[1]).read().strip()
port = int(sys.argv[2])
s = socket.create_connection(("127.0.0.1", port)); f = s.makefile("rwb")
n = 0
def call(method, params=None):
    global n; n += 1
    f.write((json.dumps({"id": f"p{n}", "method": method, "params": params or {}}) + "\n").encode()); f.flush()
    r = json.loads(f.readline().decode())
    if not r.get("ok"): raise SystemExit(f"{method} failed: {r}")
    return r.get("result")
call("auth", {"token": tok})
print("open:", call("app.open", {"path": "photo.png"}))
call("engine.execute", {"command": "paint.stroke",
     "params": {"points": [[200, 200, 1.0], [600, 400, 1.0]], "size": 90, "color": "#e8452c"}})
call("engine.execute", {"command": "paint.stroke",
     "params": {"points": [[900, 300, 1.0], [1400, 700, 1.0]], "size": 60, "color": "#2c6ee8"}})
print("save:", call("app.save", {"path": "photo.pcraft"}))
# CLEAN=1 keeps the document saved: the carry then opens the reconstructed
# tile bundle directory instead of the dirty snapshot. Default: leave it DIRTY
# so the snapshot object is exercised too.
import os
if os.environ.get("CLEAN", "0") != "1":
    call("engine.execute", {"command": "paint.stroke",
         "params": {"points": [[1500, 900, 1.0], [1800, 1200, 1.0]], "size": 120, "color": "#f2c14e"}})
call("engine.execute", {"command": "layer.select",
     "params": {"name": "Background", "occurrence": 0, "mode": "replace"}})
call("ui.set", {"tool": "brush", "zoom": 1.35, "rotation": 4.0, "center": [1024.0, 768.0]})
print("session:", call("engine.execute", {"command": "session.inspect"}))
PY

cat > "$WORK/config.json" <<EOF
{
  "source": {"addr":"127.0.0.1:$PORT_A","token_file":"$WORK/a/token","read_root":"$WORK/a","write_root":"$WORK/a"},
  "dest":   {"addr":"127.0.0.1:$PORT_B","token_file":"$WORK/b/token","read_root":"$WORK/b","write_root":"$WORK/b"},
  "evidence_dir":"$EVID",
  "window_bytes": 262144,
  "encoding": "split"
}
EOF

echo ">> 1/2 engine-level live carry (adapter example)"
cd "$REPO"
cargo run -q -p carryon-adapter-photocraft --example photocraft_ux_live -- "$WORK/config.json"

echo ">> 2/2 the APP path (CarryOnKit send/receive, headless)"
cat > "$WORK/smoke.json" <<EOF
{
  "source": {"addr":"127.0.0.1:$PORT_A","token_file":"$WORK/a/token","read_root":"$WORK/a","write_root":"$WORK/a"},
  "dest":   {"addr":"127.0.0.1:$PORT_B","token_file":"$WORK/b/token","read_root":"$WORK/b","write_root":"$WORK/b"},
  "data_dir": "$WORK/smoke-cores",
  "window_bytes": 262144
}
EOF
rm -rf "$WORK/smoke-cores"
( cd "$REPO/apps/macos" && swift run -q carryon-photocraft-smoke "$WORK/smoke.json" "$EVID/run-app.json" )

echo ">> collecting evidence"
cp "$WORK/a/app.log" "$EVID/app-a.log" 2>/dev/null || true
cp "$WORK/b/app.log" "$EVID/app-b.log" 2>/dev/null || true
ls -l "$EVID"
echo ">> done (LOCAL — not a physical or platform claim)"
