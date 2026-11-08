# Stage 6 — cross-device handoff in the UI: mac ⇄ Galaxy A04 (2026-11-08)

The full chain on real hardware: **discovery → pairing → import → action**,
with the source (macOS app code path, driven headlessly through the same
CarryOnKit pipeline) and the destination (the Android app's Receive screen,
driven on the device). Two machines, two NICs, one LAN (192.168.3.0/24), real
multicast + TLS 1.3.

**Honest labels (§2):** every run below is `OBSERVED (PHYSICAL)` — two real
machines. The on-device loopback of the same receive flow is `OBSERVED
(LOCAL)` (instrumented, device-local). No platform is "supported" (§30 unrun).

## 1 · Fresh pairing (stage-5 UX, prerequisite of the chain)

The instrumented test runner had reinstalled the app, wiping the
Keystore-sealed identity — an honest fresh start.

- Phone: Pairing tab → **Show my QR (destination)** → code `111597`,
  addr `192.168.3.3:43655`; then **Accept pairing**.
- Mac: `carryon-pairing-smoke join` with the phone's `carryon://pair` payload
  (Keychain-durable identity `48ec632e…`); both sides confirmed the SAME
  verification string `297c20`.
- Result: phone pinned `carryon-mac` (48ec632e…, Keystore-sealed); mac pinned
  the phone (2681dfde…, Keychain-sealed). `mac-pairing-join.log`.

## 2 · Happy path — discovery → paired trust → import → action

Phone Receive armed the DURABLE Keystore identity, NSD-advertised the listener
(engine §18.3 TXT, port 33867), paired_sources=1. Mac
`carryon-filehandoff-smoke serve /tmp/xdevice-carried.png --discovered
--expect-dc mobile --paired --legs 3`:

| Fact | Value |
|---|---|
| discovery (engine browse → A04) | **308 ms** (NET-003 gate < 10 s — PASS) |
| trust | PERSISTED paired store (2 records) — **no pin typed** |
| address | 192.168.3.3:33867 — **from discovery, never typed** |
| transfer | 500,008 B file; wire sent 667,880 B / recv 882 B; **989 ms** |
| legs | 1 |
| receiver gate | sha256 measured == declared (CORE-004, byte-for-byte) |
| L1 | ACTION_VIEW → `com.sec.android.gallery3d/...GalleryActivity` |
| receiver `bytes_before_first_action` | **667,880** (wire bytes before the first action) |
| evidence | both bundles `ok:true` (`section hashes match`) |

Files: `mac-transfer-happy.log`, `a04-receiver-evidence-happy.json`,
`a04-receive-ladder-ok-happy.png`, `a04-l1-opened-gallery.png` (Gallery).

## 3 · Crash drill A — cooperative suspend/resume SAVES the transfer (§11.2)

Phone: 4 KiB chunks + **⏸ suspend mid-import**; 5,000,008 B file.

- leg 1 closed mid-transfer (`PROTO_FRAMING: connection closed` at a chunk
  boundary) — the source **re-served** (leg 2), the receiver resumed on a fresh
  connection exactly as a cooperative suspend demands.
- completed: **legs=2**, wire sent 7,072,023 B / recv 312,177 B (the retried
  leg is on the wire — honest measured bytes), receiver verified the object
  byte-for-byte, Gallery opened, both bundles `ok:true`.
- Phone console: `suspended: leg 1 checkpointed (§11.2) — accepting resume leg 2…`
  → `import: completed (mirror_cut=0; legs=2)`.

Files: `mac-transfer-suspend-resume.log`, `a04-receiver-evidence-suspend.json`
(`serve_legs_used: 2`), `a04-receive-ladder-ok-suspend.png`,
`a04-console-suspend-resume.png`.

## 4 · Crash drill B — kill -9 of the receiver mid-import (non-cooperative)

`adb shell am force-stop com.carryon.app` during the import:

- The receiver process died with the partial staged bytes — **interrupted ≠
  success** (§19.3.9/EVD-003): nothing was published, no action ran, the
  source never claimed ImportComplete.
- Honest source-side finding: `serve_cut` has **no read timeout**, so the
  source BLOCKED in TLS recv on the dead peer (`mac-kill-drill-source-blocked.log`,
  `sample` shows `carryon_core::transfer::serve_cut` parked) until the
  operator tore it down. The Android side has a watchdog; the macOS source
  driver needs one (recommendation for the §30 harness, not a stage-6 fix).
- Recovery: app restart → fresh arm (new identity handle, same Keystore pin,
  paired trust intact) → the SAME transfer completed again
  (`mac-transfer-recovery-after-kill.log`, `a04-receiver-evidence-recovery.json`,
  discovery 612 ms, transfer 1,034 ms, `ok:true`).

## 5 · Gates (this tree)

- Rust: fmt clean · clippy `-D warnings` clean · 69 test targets ok.
- macOS: swift build · **swift test 24/24** (5 new `ServeLegPlannerTests`) ·
  demo smokes (editor/graph/toggles incl. --suspend) · make_app bundle OK.
- Android: engine .so rebuilt from merged main (ABI 1.2 = discovery + pairing) ·
  **JVM 32/32** (5 new `ReceiveFlowStatesTest`) · assembleDebug ·
  **instrumented 22/22 on the A04** — including the new
  `loopback_cooperative_suspend_resumes_and_completes` (§11.2 on real
  hardware, device-local).

## Reproduction procedure

```sh
# mac (same CarryOnKit pipeline as the app; Keychain prompt: approve once)
cd apps/macos && swift build
./.build/debug/carryon-filehandoff-smoke serve <file> --discovered --expect-dc mobile --paired --legs 3 --json

# phone: Receive tab (durable identity, NSD-advertised) → <chunk chips> → [⏸] → ACCEPT
# pairing (fresh device): phone Pairing → Show my QR → Accept pairing;
#   mac: ./.build/debug/carryon-pairing-smoke join --addr <a> --code <c> --payload '<uri>'
```

Server-side notes: chunk size is the destination's knob (§18.6); the tiny
budget chip is the §6.8 refusal drill; discovery never replaces pairing — the
TLS pin comes from the stage-5 trust or the typed fallback.
