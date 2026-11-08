# L0/L1 file handoff — mac ⇄ Galaxy A04, PHYSICAL (2026-11-08)

Stage-3 evidence: the L0/L1 onboarding ladder run END-TO-END across two real
machines — the macOS source (`carryon-filehandoff-smoke`, driving the SAME
`FileHandoffPipeline` the "Carry File" screen uses) and the Galaxy A04
destination (the real "Receive" screen, adb-driven UI).

## Claim labels (§2 law — honest)

| Claim | Label | Evidence in this directory |
|---|---|---|
| File carried mac→A04 over LAN TLS 1.3, byte-for-byte, sha256 measured == declared on the receiver | `OBSERVED (PHYSICAL)` | `a04-receive-evidence.json` (metrics: `content_sha256_verified: true`, `evidence_class: "physical"`), `a04-received-file-sha256.txt` == `carried-file.png` sha256 == `content_sha256_declared` |
| L1 launcher activation: the carried file OPENED in a real installed Android app via ACTION_VIEW | `OBSERVED (PHYSICAL)` | `a04-gallery-opened.png` (run 1: `com.sec.android.gallery3d/.app.GalleryActivity` foreground, image displayed), `a04-gallery-opened-final.png` (final run: `com.sec.android.mimage.photoretouching/.SPEActivity`) |
| Integrity negative: tampered declared hash → the receiver REFUSES, fail closed, nothing opens | `OBSERVED (PHYSICAL)` | `a04-receive-refusal.png` (`FAIL: E_OBJECT_DIGEST_MISMATCH … REFUSED (CORE-004)`, ladder `✗ L0`, no chooser/gallery), `mac-source-negative-console.log` |
| The engine's `document.open` activation record ran on the receiver (§6.5 activation-only) | `OBSERVED (PHYSICAL)` | bundle `actions[]` (`document.open`, oracle activation-only) |
| On-device loopback of the same receive flow (no mac involved) | `OBSERVED (LOCAL — device-local)` | `connectedDebugAndroidTest`: `HandoffLoopbackInstrumentedTest.loopback_positive_carry_verify_open` + `loopback_negative_tampered_hash_refused` (7/7 green on the A04) |
| Platform "supported" | **NO** | §30 acceptance unrun (spec §2/§30) |
| Discovery / zero-config | **NO — manual IP** | pre-discovery stage: hand-entered `IPv4:port` + mutual pins (stage-4 work, FRONTIER) |

## Run facts (final positive run)

- Source: macOS, `carryon-filehandoff-smoke serve` — session
  `9680524c-bac9-4cb6-9611-6c774e4b59b2`, cut 0, declared sha256
  `4247072a6bda8ebdaca926d52bb343764ca5d2280be474d77bbfb3cf26a18063`,
  `evid=physical` (LAN address dialed: `192.168.3.3:45045`).
- Destination: Galaxy A04 (SM-A045F, Android 13, arm64), Receive screen,
  device stamp `20261108-084533` (see `run-stamp.txt`).
- Wire: 708,328 B sent / 882 B recv (mac counters); receiver
  `time_to_action_ready_ms` ≈ 595.
- The receiver verified the store object (`file.document.v1`) by measured
  sha256 == declared (CORE-004 app-level gate) BEFORE any open.

## Files

| File | What it is |
|---|---|
| `carried-file.png` | the real file carried (530,347 B test PNG) |
| `a04-receive-evidence.json` | the RECEIVER's engine evidence bundle (mirror session, journal, document.open action, metrics) |
| `a04-received-file-sha256.txt` | on-device sha256 of the reconstructed file (== declared) |
| `mac-source-evidence.json` | the SOURCE's engine evidence bundle |
| `mac-source-console.log` / `mac-source-console-final-ladder-run.log` / `mac-source-negative-console.log` | mac smoke consoles (positive / ladder-screenshot run / tamper-negative) |
| `a04-receive-ladder-ok.png` | the Receive screen after success: `▪ L0`, `▪ L1`, `ActionReady`, `opened_in` |
| `a04-gallery-opened.png` | run 1: the carried image displayed in Samsung Gallery (foreground) |
| `a04-gallery-opened-final.png` | final run: the carried image open in Samsung Photo Editor (the OS chooser routed to a different real handler this time — both runs prove a real app opened the verified file) |
| `a04-receive-refusal.png` | the integrity negative: `E_OBJECT_DIGEST_MISMATCH … REFUSED (CORE-004)`, ladder `✗ L0`, nothing opened |

## Honest notes

- The mac side of the ladder runs through `carryon-filehandoff-smoke`, which
  drives the exact `FileHandoffPipeline` the SwiftUI "Carry File" screen calls
  (the established app-parity pattern of `carryon-demo-smoke` ↔ DemoView). The
  SwiftUI screen was exercised in dev; the recorded runs are smoke-driven.
- The engine's `document.open` action record echoes the receiver's registered
  placeholder uri (the compiled-in file adapter is not the launch channel —
  the reference shell registers a placeholder the same way); the REAL launch
  is the OS ACTION_VIEW on the MediaStore content uri, recorded in
  `metrics.content_uri`.
- The descriptor (session/cut/hash/name/mime/evidence-class) rides the
  engine's Hello/Welcome feature negotiation — data only (§3.8); unknown
  features ignored; malformed → refuse.
- Manual-IP mode is honest pre-discovery UX (stage 4 brings mDNS/NSD); pins
  are exchanged by the human channel (clipboard/copy — they are public
  fingerprints; private keys never leave the engine, EVD-006).
