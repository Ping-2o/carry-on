# CarryOnKit — the Swift SDK (stage 9)

The distributable Swift Package over the real Carry-On engine C ABI
(`carryon.h`, **ABI 1.2**, append-only within major 1). This is the same
binding the macOS app uses (`apps/macos/Sources/CarryOnKit`), carried as a
distribution surface: `Scripts/sync_from_app.sh` keeps the sources identical
(drift-checked), and the ABI compatibility suite pins the three-way contract
(header ↔ engine archive ↔ bindings).

## Distribution model: source + fetched static engine

The package is **source**, and the engine ships as a **fetched static
archive** wrapped in an xcframework (`Vendor/CarryOnEngine.xcframework`,
gitignored):

- Committing a binary dylib would violate the repo law (engine binaries are
  fetched, never committed) and would ship an ad-hoc-signed artifact —
  LOCAL-ONLY, explicitly not PLAT-006 distribution signing.
- A static archive is a supported SwiftPM binary target that links **without
  `unsafeFlags`**, so this package is a real dependency for clients (a root
  package with `unsafeFlags` cannot be consumed).
- Clients rebuild the engine per platform with one command — the ABI is pinned
  by the vendored header, so the rebuild is deterministic.

```sh
# 1. Build + vendor the engine (fetched, never committed)
sdk/swift/Scripts/build_engine_static.sh

# 2. Build / test / run the examples
cd sdk/swift
swift build
swift test          # ABI compatibility suite + contract parity scenarios
swift run CarryFileExample   # L0/L1 file handoff, in-process loopback
swift run DemoExample editor # the 8-stage demo, in-process loopback
```

Use it from a client package:

```swift
.package(path: "../carry-on/sdk/swift")   // or a git dependency on this dir
```

## Versioning (semver, follows the ABI)

The SDK version **follows the engine ABI major.minor** — ABI 1.2 → SDK
**1.2.0** (see `Sources/CarryOnKit/KitVersion.swift`). Major tracks the ABI
major (breaking), minor tracks the ABI minor (append-only additions), patch is
SDK-only fixes. `CarryOnSDK.requireCompatibleEngine()` is the startup gate: an
engine older than the SDK's ABI minor fails with a clear message.

## Contents

| Path | What |
|---|---|
| `Sources/CarryOnKit/` | the binding (synced from apps/macos; `KitVersion.swift` is SDK-local) |
| `Sources/CCarryon/` | vendored `carryon.h` + module map (the C contract) |
| `Sources/CarryFileExample/` | example: carry a file end-to-end (loopback) |
| `Sources/DemoExample/` | example: the 8-stage demo pipeline (loopback) |
| `Tests/CarryOnKitTests/` | ABI compatibility suite + contract parity scenarios |
| `Sources/CarryOnKit/CarryOnKit.docc/` | DocC catalog (getting started, ABI policy; rendered by Xcode — `swift build`/`swift test` ignore it) |
| `Scripts/` | `sync_from_app.sh`, `build_engine_static.sh` |

## Contract parity (shared with the Kotlin SDK)

The same scenario table runs on both SDKs (`Tests/.../ContractParityTests.swift`
and `apps/android/carryon-kit/src/test/.../ContractParityTest.kt`):

| Scenario | Assertion |
|---|---|
| `carry-file/loopback-positive` | file → cut → TLS → descriptor → import → CORE-004 gate → document.open; measured sha256 == declared |
| `carry-file/tampered-refused` | tampered declared hash → REFUSED, no success, nothing opens |
| `session`/`demo` verify | two cores, loopback transfer, evidence bundle verifies `ok:true` |
| `abi/version-gate` | SDK version follows the ABI major.minor; the gate refuses an old engine |

## Labels (spec §2)

Loopback = `OBSERVED (LOCAL)`. The physical cross-device run of the same
ladder is `OBSERVED (PHYSICAL)` (see
`apps/android/evidence/20261108-a04-l0l1/`). No platform is "supported" until
§30 acceptance passes on real hardware. Private keys never leave the engine
(`EVD-006`).
