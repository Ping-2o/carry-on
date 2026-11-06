# Carry-On for Android

Native-feeling app wrapper over the **real** Carry-On engine C ABI
(`crates/carryon-ffi/include/carryon.h`, ABI 1.1, append-only). Kotlin,
Jetpack Compose, Material 3 (dynamic color on API 31+), minSdk 29 (spec §9),
targetSdk 35. One Kotlin module (`:app`), manual DI (no Hilt), coroutines +
StateFlow MVVM.

> **Read the claim table first.** Claim labels are law (spec §2). Nothing in
> this app upgrades a label anywhere in the repo.

## Claim table (honest)

| Claim | Label | Why |
|---|---|---|
| This Android project builds (`./gradlew assembleDebug`) | `TARGET` — build-ready | **Not compiled on the authoring machine** (no Android SDK present). Compiles with Android Studio / `gradlew assembleDebug` where the SDK + JDK 17 exist. Kotlin sources were hand-reviewed, never compiled. |
| JNI shim (`app/src/main/cpp/carryon_jni.c`) | verified | Syntax-checked clean with the NDK r27c clang (`aarch64-linux-android29-clang -fsyntax-only -Wall -Wextra`) against the real `carryon.h`. |
| Engine binary (`libcarryon_ffi.so`, arm64-v8a) | `OBSERVED` | Cross-compiled by `Scripts/fetch_engine_lib.sh` from this checkout (cargo, aarch64-linux-android). Fetched artifact — gitignored, never committed. |
| Demo Mode (two cores, loopback TLS 1.3, full 8-stage pipeline) | `OBSERVED (LOCAL)` — **once run on a device** | One device, loopback TLS, two in-app cores. NOT physical cross-device evidence (spec §2/§30). The UI shows the verbatim disclosure footer on every run. |
| Physical cross-device handoff, discovery, signing | `TARGET` / `UNSUPPORTED` | No signed APK (PLAT-006), no §30 platform acceptance — no platform is "supported". NSD here is an **app-level pilot**; engine-side discovery is `TARGET` (FRONTIER Phase 2). |

Secrets: the identity key DER is SECRET (§22.3, EVD-006) — never logged,
displayed, or persisted by this app. Only the SHA-256 pin hex may be shown,
truncated. The ABI's cert/key DER entries exist in the raw mirror for a future
Keystore implementation (PLAT-002, `TARGET`); the app-level API exposes only the
pin and never calls them.

## Build

Requirements: Android Studio (Ladybug or newer) or a command-line Android SDK
(platform 35, build-tools 35), CMake 3.22.1+, NDK r27+ (for the prebuilt engine
you can skip the NDK — the `.so` is fetched prebuilt), **JDK 17** to run Gradle
(AGP 8.7 requirement), arm64-v8a device or emulator (API 29+).

```sh
# 1. Fetch the engine binary (cross-compiles crates/carryon-ffi for Android).
#    Needs the NDK; defaults are the authoring machine's paths — override:
#      NDK=/path/to/android-ndk-rXX WORK=/fast/external/dir
apps/android/Scripts/fetch_engine_lib.sh

# 2. Build (Android Studio: just open apps/android and Run).
cd apps/android
./gradlew assembleDebug
./gradlew installDebug          # on a connected device/emulator
```

The wrapper (`gradlew`, `gradlew.bat`, `gradle/wrapper/gradle-wrapper.jar`) is
the verified stock Gradle 8.9 wrapper from
`github.com/gradle/gradle` (tag `v8.9.0`); `distributionUrl` and
`distributionSha256Sum` are pinned to the official Gradle 8.9 release
(`gradle-8.9-bin.zip`, sha256 `d725d707bfabd4dfdc958c624003b3c80accc03f7037b5122c4b1d0ef15cecab`)
to match AGP 8.7.3. Provenance hashes (apps/ security audit):

- `gradlew`     sha256 `9cbbb4d68ff7fb5211c4d58f598ac9d8664c05fdcd1e5f59b7f2c3ac1ee00af0`
- `gradlew.bat` sha256 `0f3ed8f03b50934cb8c48b15a470d5c20a30a5385825e48b55bcc8ea3d8f8e18`
- `gradle-wrapper.jar` sha256 `498495120a03b9a6ab5d155f5de3c8f0d986a449153702fb80fc80e134484f17`

(The previous wrapper copy — sourced from a local kiwix-android checkout and a
different wrapper version than the pinned distribution — was replaced by the
verified 8.9 artifacts.) If your environment lacks the wrapper jar, run
`gradle wrapper --gradle-version 8.9` once with any local Gradle 8.x.

Without the fetched `.so`, the app installs but any engine call fails on
`UnsatisfiedLinkError` — the fetch step is part of the build recipe, not
optional.

## What the app does

1. **Onboarding** — 3 steps (typewriter `CARRY-ON` banner, the L0–L4 ladder, the
   lifecycle pipeline) with the honest note: single-writer, two-phase publish,
   interrupted ≠ success. All reveals honor Reduce Motion
   (`ANIMATOR_DURATION_SCALE == 0` → instant).
2. **Demo Mode** (flagship) — runs the real engine through the C ABI: two
   independent cores in temp dirs, real TLS 1.3 loopback (`listener_bind
   "127.0.0.1:0"`), mutual pin pairing, and the 8-stage pipeline (OPEN,
   IDENTITY, PAIR, SESSION, TRANSFER, AUTHORITY, EDIT, EVIDENCE) with live
   `carryon_session_bytes` counters, the `may_mutate` single-writer flip, and
   the l3-style measured metrics in the evidence bundle. Two scenarios:
   `org.carryon.editor` `{"session_v1":true}` (L3 structured session) and
   `org.carryon.graph` `{"sample":true}` (`graph.shortest_path`).
   Advanced toggles: chunk size 256 B, budget rejection (§6.8 — displayed as a
   NORMAL outcome), suspend/resume (§11.2, fresh-connection resume),
   recovery report.
3. **Topology** — local node card (identity pin truncated, ABI version,
   listener address), trust records with revoke (`carryon_trust_revoke`,
   persisted via `carryon_trust_to_json`/`from_json`), an ASCII device graph,
   and the **labeled** NSD discovery pilot (`_carryon._tcp`, TXT `pm id pr dc
   ch` per §18.3 — never user/session/app names or keys).
4. **Sessions** — observed sessions with exact §16.2/plan.md readiness states
   (`Idle, Preparing, CutProposed, CutSealed, Importing, ActionReady,
   ReadOnlyActive, Failed, Inconclusive, Aborted`) — never the bare "Ready" —
   plus authority_epoch and `may_mutate`.
5. **Evidence** — renders the `EvidenceBundle` JSON: section hashes, journal
   timeline, metrics, and the `disclosure` string verbatim. Bundle content is
   never executed (EVD-005).

## Layout

```
apps/android/
├── build.gradle.kts / settings.gradle.kts / gradle.properties   # Kotlin DSL
├── gradlew, gradle/wrapper/                                     # wrapper (verified Gradle 8.9, hash-pinned)
├── Scripts/fetch_engine_lib.sh        # cross-compiles the engine -> jniLibs (gitignored)
└── app/
    ├── build.gradle.kts               # minSdk 29, target/compile 35, CMake, Compose BOM
    ├── proguard-rules.pro             # keep com.carryon.app.ffi.**
    └── src/main/
        ├── AndroidManifest.xml        # INTERNET only; MainActivity exported
        ├── cpp/carryon_jni.c          # app-local JNI shim over carryon.h (1:1)
        ├── cpp/CMakeLists.txt         # IMPORTED prebuilt libcarryon_ffi.so
        ├── cpp/include/carryon.h      # vendored copy (drift-checked by fetch script)
        ├── jniLibs/arm64-v8a/         # fetched libcarryon_ffi.so (gitignored) + .gitkeep
        └── java/com/carryon/app/
            ├── ffi/CarryOnFfi.kt      # raw external funs (1:1 ABI mirror)
            ├── ffi/CarryonCore.kt     # wrappers, error-code mirror (errors.rs),
            │                          # *len protocol + out-string helpers
            ├── demo/DemoRunner.kt     # 8-stage pipeline (Dispatchers.IO)
            ├── discovery/NsdDiscovery.kt
            └── ui/                    # Compose: theme, ASCII components, screens
```

## JNI shim conventions (carryon.h, mirrored)

- Every fallible call returns the ABI i32 (`0` = OK, negative = FFI-local,
  positive = family_base + ordinal). Kotlin mirrors the full table from
  `crates/carryon-ffi/src/errors.rs` in `ffi/CarryonCore.kt` (`Err`).
- Out-strings: the shim copies the Rust-allocated UTF-8 buffer into a
  `jbyteArray` and frees the original with `carryon_string_free` — ownership
  never crosses to Kotlin.
- Out-buffers (cert/key DER, last_error, read_object) use the `*len` query
  protocol (null buffer → required size in `len[0]`, then fetch).
- Blocking calls (`serve_cut`, `serve_authority_transfer`, `import_cut`,
  `accept`, `connect`) run on `Dispatchers.IO` only.
- Handles are opaque `jlong`s; each Kotlin wrapper is `Closeable` and frees its
  handle exactly once.
