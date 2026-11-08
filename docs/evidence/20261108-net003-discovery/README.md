# NET-003 — engine-level discovery, physical mac ⇄ Galaxy A04 (2026-11-08)

Stage-4 evidence bundle for the discovery gate. **Two machines, two NICs, one
LAN (192.168.3.0/24), real multicast** — every fact below is labeled by how it
was actually observed; nothing upgrades a platform to "supported" (§30 unrun).

## Devices and roles

| Device | Role | Discovery side |
|---|---|---|
| MacBook Air (macOS, `192.168.3.32`, en0 Wi-Fi) | source (serves the file) | engine-side advertise + browse (`mdns-sd` in `carryon-net`, ABI 1.2) |
| Samsung Galaxy A04 (SM-A045F, Android 13, `192.168.3.3`, Wi-Fi) | destination (receives) | platform NSD registers/browses (app layer); TXT built/validated by the engine over JNI |

TXT contract (§18.3, minimal set, engine-owned): `pm` (protocol major),
`id` (8-hex instance), `pr` (pairing required), `dc` (desktop|mobile|unknown),
`ch` (connection hint = bound listener port or `none`). Foreign keys are
ignored; malformed records fail closed (`E_PLATFORM_DISCOVERY`) and are dropped,
never repaired.

## NET-003 gate — app/screen open → first validated peer (gate < 10 s)

Both measurements use a monotonic clock (§18.10) and the SAME code path the app
UI uses (`EngineDiscovery` on the mac; `NsdDiscovery` + engine validation on the
A04). `first peer` means the first peer that passed the engine's fail-closed
TXT filter (rejected records never count).

| Direction | Browse start | First validated peer | Wall time | Gate |
|---|---|---|---|---|
| mac → (find) A04 | engine browse start | `carryon-f69056e0 dc=mobile` → `192.168.3.3:36017` | **539 ms** | **PASS** (< 10 s) |
| A04 → (find) mac | Topology screen browse start | `carryon-141dda1d dc=desktop` → `192.168.3.32:0` | **177 ms** | **PASS** (< 10 s) |

- mac-side: `mac-discovery-gate-a04.log`, `mac-discovery-gate-a04-rerun.log`
  (`carryon-discovery-smoke --expect-dc mobile`, engine browse → NSD-advertised
  receiver; an earlier arm resolved at 537 ms in `mac-discovery-gate-a04.log`).
- A04-side: `a04-topology-discovery-net003.png` — the phone's own UI shows
  `NET-003 first peer in 177 ms (gate < 10 s) — PASS`, the discovered mac
  (`dc=desktop pm=1`), and the honest `platform supported: NO` label.
- A repeat measurement (receiver re-armed): 537 ms (`-rerun.log`) — consistent.

## Full L0/L1 handoff to the DISCOVERED address (PHYSICAL)

`mac-serve-discovered-address.log`: the mac served a real 167-byte file to
`192.168.3.3:36017` — the address obtained from the ENGINE's browse output
(`--json`), **never typed by hand on the mac**; the phone's display address
(manual fallback) was not used. TLS 1.3 with the manually-typed trust pin
(pairing UX is stage 5 — discovery never replaces pairing):

- connect: `192.168.3.3:36017` (mutual certificate pinning §18.5)
- serve → destination reported ImportComplete; L0 ▪
- receiver-side CORE-004 gate: measured sha256 == declared (byte-for-byte)
- L1: ACTION_VIEW chooser appeared on the A04 (`a04-l1-opened-chooser.png`), the
  user could open the carried file in a real installed app
- receiver ladder screenshot: `a04-receive-ladder-ok-discovery.png` (▪ L0 ▪ L1)
- evidence bundles: `mac-source-evidence.json` (source session, verify
  `{"ok":true,"message":"section hashes match"}`),
  `a04-receiver-evidence.json` (receiver session pulled from the device;
  `content_sha256_verified: true`, `action_view_resolved: true`).
- carried file: `carried-file.txt` (sha256 declared == measured == 049fcb81…2525).

## Honest labels (§2)

- The mac⇄A04 discovery facts above are **OBSERVED (PHYSICAL)** — two real
  machines, real multicast, engine-validated TXT both directions.
- The owner-machine self-discovery run (one machine: advertise+browse finds
  itself, `mdns_live_advertise_then_browse_finds_self`, ~0.8–1.0 s) is
  **OBSERVED (LOCAL)**.
- No platform is "supported" (§30 unrun). Discovery is a network fact, not a
  pairing: connections still pin the peer's certificate fingerprint, typed by
  hand (stage-5 PIN/QR pairing will replace the typing, not the pinning).
- Rejections seen during the runs: none (both peers advertised the canonical
  §18.3 TXT; the fail-closed filter is exercised by unit/instrumented tests:
  `carryon-net` TXT suite, `carryon-ffi` ABI suite, Swift
  `EngineDiscoveryContractTests` 5/5, Android `NsdTxtPairsTest` 3/3 (JVM) +
  `DiscoveryContractInstrumentedTest` 4/4 on the A04).

## Reproduction procedure (owner machine — CI runners cannot multicast)

```sh
# Rust (all gates; the live test is #[ignore]d on purpose):
export CARGO_TARGET_DIR="/Volumes/ADATA ELITE SE880/compilation/target-discovery"
cargo test --workspace --features carryon-ffi/test-panic --exclude carryon-adapter-kiwix
cargo test -p carryon-net -- --ignored mdns_live --nocapture   # LOCAL self-discovery

# macOS app side:
apps/macos/Scripts/build_engine.sh && cd apps/macos && swift build && swift test
.build/debug/carryon-discovery-smoke --expect-dc mobile --timeout 10 --json

# Android app side (A04 attached via adb; engine .so fetched first):
apps/android/Scripts/fetch_engine_lib.sh && cd apps/android && \
  ./gradlew :app:testDebugUnitTest assembleDebug && \
  ./gradlew :app:connectedDebugAndroidTest   # then adb install -r (test-runner removes it)

# Physical run: open the A04 app → Receive (arms + NSD-advertises its port);
# run the discovery smoke above (mac engine browse finds it; NET-003 printed);
# open the A04 app → Topology while `carryon-discovery-smoke --hold 60` runs on
# the mac (A04's own NET-003 appears in its Discovery section).
```
