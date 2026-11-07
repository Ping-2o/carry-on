# Carry-On apps — native wrappers (macOS + Android)

Two native-feeling app wrappers over the **real** Carry-On engine C ABI
(`crates/carryon-ffi/include/carryon.h`, ABI 1.1, append-only): **Carry-On for
macOS** (Swift 5 language mode, SwiftUI, `apps/macos/`) and **Carry-On for
Android** (Kotlin, Jetpack Compose, Material 3, `apps/android/`). Both bind the
existing C ABI only — no Rust changes, no new `#[no_mangle]` symbols — and ship
the same feature parity: Continuity onboarding, one-device **Demo Mode** that
drives the real engine end-to-end (8 stages incl. L4 authority transfer), Live
Device Topology with an app-level discovery pilot, an evidence viewer, and
ASCII art + animations that honor Reduce Motion.

> **Labels are law (spec §2).** Nothing in either app upgrades a label anywhere
> in the repo. Read the claim table before quoting any of this.

## Claim table (honest)

| Claim | Label | Evidence |
|---|---|---|
| macOS app builds + runs the full L3 loopback demo through the C ABI | `IMPLEMENTED` / `OBSERVED (LOCAL)` | `swift build` clean; `swift run carryon-demo-smoke` exit 0; `dist/CarryOn.app` assembled, `codesign --verify` OK (ad-hoc) |
| Android app builds (`./gradlew assembleDebug`: Kotlin compile, JNI shim via CMake against the cross-compiled engine `.so`) | `IMPLEMENTED` | CI `android-app` job green (run 42373070251, head `4023e0f`; NDK r27c 27.2.12479018) + local build. The CI APK's first compile surfaced 3 latent resolution errors (77bd3ec) and its first device install surfaced a link bug — both fixed in-branch (see below) |
| Android Demo Mode runs the full 8-stage pipeline through the C ABI **on a real device** | `OBSERVED (LOCAL — device-local)` | First device run 2026-11-07, Samsung Galaxy A04 (SM-A045F, Android 13, arm64), driven via adb: both scenarios (L3 editor + graph) and all four toggles pass; 5/5 device regression tests green (`connectedDebugAndroidTest`). Bundles + metrics: [`apps/android/evidence/20261107-a04-demo/`](android/evidence/20261107-a04-demo/) — baseline `time_to_action_ready_ms` 586–692 ms (< 1 s), §21.2 `may_mutate` flips verified, `verify_evidence` ok on every run. **This is still loopback within ONE device** — device-local evidence, NOT physical cross-device transfer |
| One-device demo = physical cross-device | **NO — `OBSERVED (LOCAL)` only** | Loopback TLS 1.3 on one machine, two in-process cores (spec §2/§30) — true on the A04 exactly as on the mac |
| Any platform "supported" | **NO** | §30 acceptance never run on real hardware |
| Signed APK / distribution signing | `UNSUPPORTED` here | PLAT-006 out of scope; macOS ad-hoc codesign is LOCAL-ONLY |
| Engine-side discovery | `TARGET` (FRONTIER Phase 2) | The Bonjour/NSD pilot is **app-level only**, §18.3 minimal TXT |
| Process/memory migration | `UNSUPPORTED` | Typed continuation only (spec §1) |

Fixed footer every demo run ends with (verbatim, `plan.md`):

```
DISCLOSURE: LOCAL run on ONE DEVICE via loopback TLS 1.3. Proves the FFI binding and
engine pipeline, NOT physical cross-device transfer or a supported platform (spec §2/§30).
```

## The two apps

| App | Stack | Build & verify | Details |
|---|---|---|---|
| **Carry-On for macOS** (`apps/macos/`) | Swift 5 language mode (tools 5.9), SwiftUI, macOS 14+ | `apps/macos/Scripts/build_engine.sh` → `swift build` → `swift run carryon-demo-smoke` → `apps/macos/Scripts/make_app.sh` | [`apps/macos/README.md`](macos/README.md) |
| **Carry-On for Android** (`apps/android/`) | Kotlin, Jetpack Compose, Material 3, minSdk 29 (spec §9), targetSdk 35 | `apps/android/Scripts/fetch_engine_lib.sh` → `./gradlew assembleDebug` (Android SDK + JDK 17 required) | [`apps/android/README.md`](android/README.md) |

## Shared rules (both apps, from `plan.md`)

- **No Rust changes.** `crates/`, `xtask/`, and the green gate stay untouched;
  apps bind the existing C ABI only.
- **Secrets never leak (EVD-006/§22.3).** The identity key DER is SECRET — never
  logged, displayed, or bundled; only the truncated SHA-256 pin hex is shown.
- **Session states use the exact §16.2 names** (`Idle, Preparing, CutProposed,
  CutSealed, Importing, ActionReady, ReadOnlyActive, Failed, Inconclusive,
  Aborted`) — never the bare word "Ready".
- **Discovery TXT** carries §18.3 minimal metadata only (`pm`, `id`, `pr`, `dc`,
  `ch`) — never user/session/app names or keys.
- **Evidence is rendered, never executed** (EVD-005).
- **ASCII style:** box-drawing characters (`─ │ ┌ ┐ └ ┘ ═ ▶`), byte blocks
  (`▪▪▪▫▫▫`), monospace `key:  value` console lines, uppercase trailing
  `DISCLOSURE:` line.
- **Reduce Motion honored** on both platforms.

## Honest notes

- Demo Mode is one device, two in-process cores, loopback TLS 1.3. Nothing in
  either app exercises a physical network path or a second NIC — **including
  the 2026-11-07 Galaxy A04 device run**: the A04 proves the Kotlin/JNI
  binding, the packaging, and the full pipeline on real Android hardware and a
  real ARM CPU, but it is still `OBSERVED (LOCAL)`, device-local.
- The macOS app is ad-hoc signed (`codesign --sign -`) — LOCAL-ONLY, explicitly
  not PLAT-006 distribution signing.
- The Android app's **first device run** (2026-11-07, Galaxy A04) worked only
  after a bugfix round — the CI APK had never actually executed its FFI
  binding before. What the first run surfaced, all fixed in-branch with
  regression coverage:
  1. **DT_NEEDED device crash** — the cargo-built engine `.so` has no SONAME,
     so the JNI shim recorded the *build machine's absolute path* as its
     engine dependency; the APK `dlopen`-failed at the first FFI call.
     Fixed by `IMPORTED_NO_SONAME` in `CMakeLists.txt` + a build-time
     `verify_dt_needed` guard + an instrumented `FfiSmokeTest`.
  2. **Chunk-size toggle** sent 256 B into an engine that admits
     [4096, 8 MiB] (§18.6) → run died at OPEN. Now clamped (parity with
     DemoPipeline.swift), pinned by unit test.
  3. **Recovery report** was typed as ints; the engine emits id ARRAYS →
     JSONException at OPEN. Shape fixed + pinned against the verbatim engine
     JSON.
  4. **Budget showcase silently no-op'd + wire-round desync** — the toggle's
     tiny budget (256 B) was above the editor closure actually carried
     (197 B), so §6.8 never fired and the deterministic round accounting
     diverged from the source's bounded serve loop (120 s watchdog hang).
     Budget re-tuned to macOS parity (128 B), suspend arming ordered AFTER the
     §6.8 round, the serve loop tolerates the expected mid-serve wire close
     on non-final rounds (`round_trip.rs` ignores serve_cut results for
     exactly this reason), and a dest-side failure now fails fast (one
     throwaway connection un-parks the accept()) instead of burning the
     watchdog.
  5. **Lost engine error codes** — the JNI out-string wrappers returned NULL
     on error and Kotlin fabricated `UNKNOWN` (INT_MIN), so the typed
     `E_BUDGET_NETWORK` (500) refusal could never be matched (the graph +
     budget run wedged on-device). The shim now records the real rc in a
     pthread key (same-thread discipline as `carryon_last_error`); an
     engine-level instrumented test pins the 500.
  Device regression suite: `./gradlew :app:connectedDebugAndroidTest` (needs
  one connected device; 5 tests: FFI load + baseline/all-toggles editor +
  baseline/all-toggles graph + engine-level budget refusal). JVM units cover
  the pure logic of every fix; run in CI via `:app:testDebugUnitTest`.
- Engine-side discovery, signed packaging, and §30 platform acceptance are all
  still `TARGET` in [`docs/FRONTIER.md`](../docs/FRONTIER.md). These wrappers
  are pilots: the Swift binding and the Kotlin/JNI shim are the first slices of
  the Phase-4 SDK items — the SDKs themselves remain unchecked.
