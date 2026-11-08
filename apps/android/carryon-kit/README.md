# carryon-kit — the Android SDK (stage 9)

The distributable **AAR** over the real Carry-On engine C ABI
(`carryon.h`, **ABI 1.2**, append-only within major 1). It packages:

- the JNI shim (`src/main/cpp/carryon_jni.c` — pure translation over the ABI),
- the engine `libcarryon_ffi.so` for arm64-v8a (fetched, never committed —
  `Scripts/fetch_engine_lib.sh`),
- the typed Kotlin wrappers in `dev.carryon.kit` (`Core`, `Identity`, `Trust`,
  `Listener`, `Session`, `DiscoveryAdvertiser`/`DiscoveryBrowser`, the
  `Err` table, JSON parsers),
- kdoc on every public entry, consumer ProGuard rules, and the published ABI
  metadata in the merged manifest (`dev.carryon.kit.ABI_MAJOR` /
  `ABI_MINOR` / `SDK_VERSION`).

The app (`:app`) consumes it as a normal project dependency — the same AAR a
third-party customer gets.

## Build

```sh
# 1. Fetch the engine .so into this module (cross-compiled, gitignored)
apps/android/Scripts/fetch_engine_lib.sh

# 2. Build the AAR
cd apps/android && ./gradlew :carryon-kit:assembleRelease
# -> carryon-kit/build/outputs/aar/carryon-kit-release.aar
```

## Use

```kotlin
// build.gradle.kts: implementation(files("libs/carryon-kit-release.aar")) — or a Maven repo
import dev.carryon.kit.*

// The ABI gate: fail with an actionable message, never a random E_PROTO.
CarryOnKit.requireCompatibleEngine()

Core.open(dataDir).use { core ->
    core.registerAdapter("org.carryon.file", """{"path":"/abs/path"}""")
    val session = core.createSession(
        """{"adapter_id":"org.carryon.file","title":"carry a file","privacy":"personal","authority_mode":"read_only_replica"}"""
    )
    val cut = core.createCut(session)
    // ... transport (Session.connect/accept + serveCut/importCut), evidence
}
```

## Versioning (semver, follows the ABI)

The SDK version **follows the engine ABI major.minor** — ABI 1.2 → SDK
**1.2.0** (`version` in `build.gradle.kts`, `CarryOnKit.SDK_VERSION`, and the
manifest metadata). Major tracks the ABI major (breaking), minor tracks the
ABI minor (append-only additions), patch is SDK-only fixes.

`CarryOnKit.requireCompatibleEngine()` compares `abiVersion()` against the
constants this AAR was built with and throws `CoreException` on a mismatch
(an engine older than the SDK's minor is refused; a newer minor is fine —
append-only).

## Tests

```sh
# Pure JVM (always): ABI compatibility suite — header ↔ shim ↔ externals ↔
# manifest metadata, including the exact JNI entry-point names.
./gradlew :carryon-kit:testDebugUnitTest

# + engine-level contract parity on the host JVM (desktop JNI shim):
apps/android/Scripts/build_desktop_shim.sh
./gradlew :carryon-kit:testDebugUnitTest \
    -PcarryonDesktopLibDir=$PWD/carryon-kit/build/desktop-lib
```

The same scenario table runs through the Swift SDK
(`sdk/swift/Tests/CarryOnKitTests/ContractParityTests.swift`):
`carry-file/loopback-positive`, `carry-file/tampered-refused`,
`session/transfer-evidence`, `abi/version-gate`.

## Labels (spec §2)

Host-JVM and loopback runs are contract proof, not platform claims. Device
evidence (instrumented) stays the platform-level record; no platform is
"supported" until §30 acceptance passes on real hardware. Private keys never
leave the engine (`EVD-006`) — the wrappers expose only the SHA-256 pin.
