# Platform shells — TARGET reference only (NOT built, NOT supported)

This directory is a **reference** for the native platform shells that will link the
Carry-On C ABI (`../include/carryon.h`, exported by `carryon-ffi` as a `cdylib` for
Android `.so` and a `staticlib` for an iOS XCFramework).

## Status: TARGET (spec §2, §9, §30, §53, PLAT-001)

Nothing here is compiled, tested, signed, or run in this repository. No platform is
"supported" and none may be described as such until its Section 30 acceptance matrix
passes on **physical hardware of each claimed architecture**. This environment has no
Xcode, no Android SDK, no devices, and no signing identity, so a shell cannot be
built here — and a shell that cannot be built is not evidence of anything.

What IS proven here (green gate, `cargo test -p carryon-ffi`): the C ABI itself —
the full import→carry→relaunch lifecycle driven by a Rust-as-C caller over real
TLS 1.3 on loopback, panic-safety at the boundary, mobile small-chunk transfer,
cooperative suspend→resume, budget refusal, and stable device-identity persistence.
That is LOCAL evidence (§2/§30), not physical cross-device evidence.

## The honest "portable program container" model

Carry-On is **not** LiveContainer: it does not run foreign app binaries, inject
dylibs, use JIT, or require private entitlements (forbidden by §11.2, §4.4, §3.8 and
by store policy). It imports a program's **declared cooperative state** (files +
typed objects via a compiled-in adapter), carries it cross-platform as a
content-addressed bundle over authenticated TLS, and relaunches via activation. The
shell selects an adapter by id + JSON params through `carryon_register_adapter` — it
passes data, never behavior.

## What each shell must add (native, per platform)

- **iOS/iPadOS (Swift)** — link the `staticlib` as an XCFramework; App Intents /
  Universal Links / document picker for activation (§11); Keychain for the device
  key read back via `carryon_identity_key_der` (SECRET) and restored with
  `carryon_identity_from_der`; BackgroundTasks calling `carryon_core_request_suspend`
  before yielding and `carryon_resume_import` on the next opportunity (§11.2); local
  networking permission + Bonjour for discovery.
- **Android (Kotlin/JNI)** — load the `cdylib` `.so`; Intents / App Links / SAF for
  activation (§13); Keystore for the device key; a foreground service +
  `request_suspend`/`resume_import` for policy-compliant long transfers; NSD for
  discovery.

Both call the SAME C ABI; the lifecycle logic lives once in the Rust core.

## ABI compatibility

Check `carryon_abi_version()` against the header's `CARRYON_ABI_MAJOR`/`MINOR` at
load. The shell must build the core with the unwind panic strategy (not
`panic=abort`) so the `catch_unwind` boundary (§8.1) actually catches.
