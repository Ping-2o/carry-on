# plan.md — Carry-On app wrappers (macOS + Android)

Working plan for the `apps/native-wrappers` branch. This file is the shared contract for
every agent building the wrappers. Read it before writing any code.

> Status note: this document describes work in flight on a feature branch. Claim labels
> below follow §2 law. Nothing here upgrades any label elsewhere in the repo.

## Goal

Two native-feeling app wrappers over the **real** Carry-On engine C ABI
(`crates/carryon-ffi/include/carryon.h`, ABI 1.1, append-only):

| App | Stack | Location |
|---|---|---|
| Carry-On for macOS | Swift 5 language mode (swift-tools 5.9, Swift 6.4.2 compiler), SwiftUI, @Observable, macOS 14+ | `apps/macos/` |
| Carry-On for Android | Kotlin, Jetpack Compose, Material 3 (dynamic color), minSdk 29 (spec §9), targetSdk 35 | `apps/android/` |

Hard rules for both apps:

- **No Rust changes.** `crates/`, `xtask/`, and the green gate must stay untouched.
  Apps bind the existing C ABI only. No new `#[no_mangle]` symbols.
- **Claim labels are law (§2).** Demo mode = `OBSERVED (LOCAL)` — one device, loopback
  TLS, two in-app cores. It is NOT physical cross-device evidence. No platform is
  "supported" (§30 unrun). No signed APK (PLAT-006). Engine-side discovery is `TARGET`
  (Phase 2); the app-level Bonjour/NSD pilot is app-level only and must be labeled as
  such.
- **Disclosure wording is copied verbatim** from the repo (see below).
- **Secrets never leak**: the identity key DER is SECRET (§22.3); apps must never log,
  display, or bundle it. Only the SHA-256 pin hex (fingerprint) may be shown, truncated.
- ASCII art uses the repo style: box-drawing characters (`─ │ ┌ ┐ └ ┘ ═ ▶`), byte blocks
  (`▪▪▪▫▫▫`), monospace `key:  value` console lines, uppercase trailing `DISCLOSURE:` line.
- Animations must honor Reduce Motion (macOS `@Environment(\.accessibilityReduceMotion)`,
  Android `Settings.Global.ANIMATOR_DURATION_SCALE` == 0 or `ANIMATOR_DURATION_SCALE`).

## Feature spec (both apps, feature parity)

### 1. Continuity onboarding (first launch)

3 animated steps, then lands on the home screen:

1. "Begin here → continue there" — the product idea. Animated ASCII logo banner
   (`CARRY-ON` in a box-drawing frame), per-character reveal (typewriter).
2. "What carries" — the L0–L4 ladder as ASCII:
   ```
   L4 ─ single-writer authority transfer
   L3 ─ structured session (document + unsaved + meta)
   L2 ─ typed objects + actions
   L1 ─ launcher activation
   L0 ─ files
   ```
3. "The lifecycle" — animated ASCII pipeline:
   `CREATE CUT ─▶ SEAL ─▶ TLS 1.3 ─▶ VERIFY ─▶ PUBLISH ─▶ ACTION READY`
   with the honest note: single-writer, two-phase publish, interrupted ≠ success.
   ASCII style: box-drawing arrows, no block-character banners.

### 2. Demo Mode — one device only (the flagship feature)

Runs the **real engine** through the C ABI: two independent cores in temp dirs on the
same machine, real TLS 1.3 loopback (RFC: `carryon_listener_bind("127.0.0.1:0")`),
mutual pin pairing. Label shown in UI: `OBSERVED (LOCAL) — one device, loopback TLS 1.3;
not physical cross-device evidence (spec §2/§30)`.

Two scenarios, switchable:
- **L3 editor handoff** (flagship): `org.carryon.editor` registered with
  `{"session_v1":true}` (the adapter's sample: document + unsaved edits + cursor 42,
  selection 20–25, scroll, tab "draft.md").
- **Graph handoff**: `org.carryon.graph` with `{"sample":true}`, action
  `graph.shortest_path`.

Stage pipeline (8 stages, each animated as an ASCII block that lights up; live
`carryon_session_bytes` counters polled on a timer):

1. `OPEN` — `carryon_core_open` × 2 (temp data dirs)
2. `IDENTITY` — `carryon_identity_generate` × 2; show truncated pin fingerprints
3. `PAIR` — `carryon_pair_devices` → `carryon_trust_from_json` each side
4. `SESSION` — register adapter, `carryon_create_session` (+`create_cut`)
5. `TRANSFER` — `carryon_listener_bind` → accept/connect → negotiate →
   source `carryon_serve_cut` ⇄ dest `carryon_import_cut` (live bytes; outcome JSON:
   `{"completed":true,"mirror_cut":N}`)
6. `AUTHORITY` — `carryon_may_mutate` (false on mirror) →
   `carryon_request_authority_transfer` → receipt (`new_epoch`) → `may_mutate` true;
   source `carryon_serve_authority_transfer` → `may_mutate` false
7. `EDIT` — dest `carryon_execute_action_as` with
   `{"class":"document.edit","params":{...}}` (source-off independent edit); oracle
   `agreed` shown
8. `EVIDENCE` — `carryon_export_evidence_with` (+ extra metrics JSON mirroring
   `carryon_l3.c`: `level`, `bytes_transferred_sent/recv`,
   `bytes_recv_to_action_ready`, `time_to_action_ready_ms`, …) →
   `carryon_verify_evidence` → `{"ok":true,...}`

Advanced showcase toggles (Demo settings):
- **Chunk size** (`carryon_core_set_chunk_size`) — show progressive transfer.
- **Budget** (`carryon_core_set_budget` with a Budget JSON; tiny net budget → import
  returns `{"partial_budget":true,...}` or code 500) — displayed honestly as a NORMAL
  outcome (§6.8: budget rejection SHALL be visible).
- **Suspend/resume** — `carryon_core_request_suspend` mid-import →
  `{"suspended":true,"resume_token":…}` → `carryon_resume_import` completes.
- **Recovery report** — `carryon_core_recovery_report_json` fields
  (`interrupted_transfers, hidden_incomplete_objects, ambiguous_sessions,
  journal_truncated, clean`).

Every demo run ends with a monospace engine console panel (repo style `key:  value`
lines) and the fixed footer:

```
DISCLOSURE: LOCAL run on ONE DEVICE via loopback TLS 1.3. Proves the FFI binding and
engine pipeline, NOT physical cross-device transfer or a supported platform (spec §2/§30).
```

### 3. Live Device Topology

- **Local node card**: device identity name + truncated pin hex, ABI version, listener
  address when listening.
- **Trust records**: device_name, truncated pin, trust_generation, revoked,
  paired_utc; revoke action (`carryon_trust_revoke`). Trust JSON persisted in app
  storage via `carryon_trust_to_json` / `carryon_trust_from_json`.
- **Sessions**: id, title, exact §16.2 states (`Idle, Preparing, CutProposed, CutSealed,
  Importing, ActionReady, ReadOnlyActive, Failed, Inconclusive, Aborted`) — never the
  bare word "Ready" — plus authority_epoch and `may_mutate`.
- **Discovery pilot (app-level, labeled)**: macOS advertises/browses `_carryon._tcp`
  via NWListener/NWBrowser; Android via NSD register + browse. TXT keys per §18.3
  minimal metadata only: `pm`, `id`, `pr`, `dc` (desktop|mobile|unknown), `ch` — never
  user/session/app names or keys. UI copy: "App-level discovery pilot — engine-side
  discovery is TARGET (FRONTIER Phase 2)."
- ASCII device graph: boxes for nodes, `═══ TLS ══` links, live byte counters on links.

### 4. Evidence viewer

Parses `EvidenceBundle` JSON: `bundle_version, session_id, created_utc,
verifier_version, build{crate_version,schema_version,phase}, session_json, journal[],
actions[], metrics{}, section_hashes[[name,sha256]], disclosure`. Shows section hashes,
journal timeline, metrics, and the `disclosure` string verbatim. No bundle content is
ever executed (EVD-005).

## macOS specifics (`apps/macos/`)

- Swift Package, `swift-tools-version:5.9` (compiler 6.4.2 runs Swift 5 language mode —
  avoids Swift 6 strict-concurrency friction). Deployment target macOS 14.0 (`@Observable`).
- Targets: `CCarryon` (C target; carries the copied `carryon.h`; links
  `libcarryon_ffi`), `CarryOnKit` (Swift binding), `CarryOnApp` (SwiftUI executable),
  `carryon-demo-smoke` (headless verification executable — runs the full L3 demo
  pipeline, prints the console, exits 0 on success).
- Linking: dylib from `Scripts/build_engine.sh`
  (`cargo build -p carryon-ffi --release`, `CARGO_TARGET_DIR` = the repo's external
  dir) copied to `apps/macos/Vendor/`, `install_name_tool -id @rpath/libcarryon_ffi.dylib`.
  App binary gets `-Wl,-rpath,@executable_path/../Frameworks`; dev/smoke builds get an
  absolute `-Wl,-rpath` to `Vendor/` (via linkerSettings unsafeFlags in Package.swift —
  acceptable for a root package) so `swift run` works without env hacks.
- `Scripts/sync_header.sh` copies `crates/carryon-ffi/include/carryon.h` →
  `Sources/CCarryon/include/` and fails if the copy would differ from a stale copy
  (drift check).
- `Scripts/make_app.sh` assembles `CarryOn.app` (Info.plist: CFBundleDisplayName,
  LSMinimumSystemVersion 14.0, `NSLocalNetworkUsageDescription`,
  `NSBonjourServiceTypes` `_carryon._tcp`, dylib in `Contents/Frameworks`), ad-hoc
  codesign (`codesign --sign -` — LOCAL ONLY, explicitly not PLAT-006 signing).
- HIG: `NavigationSplitView` sidebar, SF Symbols, materials (`.ultraThinMaterial`),
  Dark Mode adaptive colors, Dynamic Type, keyboard shortcuts (⌘N new session, ⌘D demo,
  ⌘1…⌘4 navigation), Settings scene, spring animations
  (`.spring(response:0.35, dampingFraction:0.8)`), `matchedGeometryEffect` in
  onboarding, rolling byte counters, reduce-motion honored.
- Blocking FFI (`serve_cut`, `import_cut`, `serve_authority_transfer`) runs on
  `Task.detached` off the main actor. `carryon_session_bytes` polled with a timer.
- Verification gate: `swift build` clean; `swift run carryon-demo-smoke` exits 0 with
  evidence verified; `Scripts/make_app.sh` produces a launchable bundle.

## Android specifics (`apps/android/`)

- Kotlin-DSL Gradle project, Compose BOM, Material 3 (dynamic color on API 31+),
  minSdk 29, targetSdk 35, Kotlin 2.x (`kotlin-android` + `compose-compiler` plugin),
  coroutines/StateFlow MVVM. No Hilt (manual DI).
- JNI shim `app/src/main/cpp/carryon_jni.c` + `CMakeLists.txt` (IMPORTED prebuilt
  `libcarryon_ffi.so`) — the shim is app-local; the crate's FFI surface is untouched.
  `external fun`s in `CarryOnFfi.kt` mirror the C ABI; error codes mirrored from
  `crates/carryon-ffi/src/errors.rs`.
- `Scripts/fetch_engine_lib.sh` cross-compiles
  (`cargo build -p carryon-ffi --target aarch64-linux-android --release` with the NDK
  env from `crates/carryon-ffi/shells/android-cli/build_and_run.sh`) and copies the
  `.so` into `app/src/main/jniLibs/arm64-v8a/` (gitignored; fetched, not committed).
- `NsdDiscovery.kt`: NSD register + browse `_carryon._tcp`, same TXT contract.
- **No Android SDK on this machine** — verified. Ship the project build-ready and label
  it: "build-ready TARGET — not compiled on the authoring machine (no Android SDK
  present); compiles with Android Studio / `gradle assembleDebug` where the SDK exists."
  If a Gradle wrapper jar can be sourced from the existing
  `/Volumes/ADATA ELITE SE880/compilation/kiwix-android` checkout, copy
  `gradlew` + `gradle/wrapper/*` so `./gradlew` works out of the box; otherwise document
  `gradle wrapper`.
- Syntax-verify `carryon_jni.c` with the NDK clang
  (`$TC/bin/aarch64-linux-android29-clang -fsyntax-only` against the NDK sysroot +
  `-I crates/carryon-ffi/include`) — real verification without the SDK.
- Kotlin: no kotlinc available — do NOT claim it compiles; the README states the
  honest status.

## Files to create

```
plan.md                          # this file
apps/README.md                   # overview + §2 claim table (Agent C)
apps/macos/**                    # Agent A (see macOS specifics)
apps/android/**                  # Agent B (see Android specifics)
docs/FRONTIER.md                 # Phase-4 pilot note (Agent C)
CONTRIBUTING.md                  # apps pointer in workspace layout (Agent C; originally the private maintainer notes)
```

## Verification (final gate, orchestrated)

1. Cargo green gate (workspace unchanged, must stay green): `cargo test --workspace
   --features carryon-ffi/test-panic`, `cargo clippy --workspace --all-targets
   --features carryon-ffi/test-panic -- -D warnings`, `cargo fmt --all -- --check`.
2. macOS: swift build + demo smoke exit 0 + app bundle assembled.
3. Android: NDK syntax check of the JNI shim; project build-ready (no SDK).
4. `crates/carryon-ffi/tests/header_sync.rs` untouched.

## Delivery

Branch `apps/native-wrappers` → push → PR to `main` explaining all apps, features,
verification results, and honest claim labels.
