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
| Android project build-ready (Gradle Kotlin-DSL / Compose / JNI) | `TARGET` — **not compiled** (no Android SDK on the authoring machine) | NDK r27c clang syntax check of the JNI shim exit 0; 49/49 Kotlin↔C JNI symbols cross-checked; real aarch64 `libcarryon_ffi.so` fetched by `Scripts/fetch_engine_lib.sh` |
| One-device demo = physical cross-device | **NO — `OBSERVED (LOCAL)` only** | Loopback TLS 1.3 on one machine, two in-process cores (spec §2/§30) |
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
  either app exercises a physical network path or a second NIC.
- The macOS app is ad-hoc signed (`codesign --sign -`) — LOCAL-ONLY, explicitly
  not PLAT-006 distribution signing.
- The Android app has never been compiled: no Android SDK exists on the
  authoring machine. The Kotlin sources are hand-reviewed, not built.
- Engine-side discovery, signed packaging, and §30 platform acceptance are all
  still `TARGET` in [`docs/FRONTIER.md`](../docs/FRONTIER.md). These wrappers
  are pilots: the Swift binding and the Kotlin/JNI shim are the first slices of
  the Phase-4 SDK items — the SDKs themselves remain unchecked.
