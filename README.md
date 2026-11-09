# Carry-On

[![CI](https://github.com/Ping-2o/carry-on/actions/workflows/ci.yml/badge.svg)](https://github.com/Ping-2o/carry-on/actions/workflows/ci.yml)
![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)
![C ABI 1.3](https://img.shields.io/badge/C%20ABI-1.3-informational)

**A cooperative cross-platform continuation engine, written in Rust.** Start supported
work on one device and continue a *declared, typed action* on another — unsaved edits,
view state and single-writer authority included. Typed continuation, **never**
process/memory migration: no executable code ever travels in state.

> **Claim labels are law.** Every claim in this repository carries a label —
> `PROVED` / `IMPLEMENTED` / `OBSERVED` / `TARGET` / `OPTIONAL` / `RESEARCH` /
> `UNSUPPORTED` — and points at its evidence. A loopback run is `OBSERVED (LOCAL)`,
> not cross-device evidence. **No platform is "supported" until the §30 acceptance
> run passes on real hardware, and it has not been run yet.**

## Why

Cloud save moves a *saved* file and drops the unsaved buffer and view state. rsync
moves bytes with no notion of a session. CRIU and VM migration move opaque
process/memory images and need the same kernel and CPU architecture. Carry-On carries
a **typed session** across heterogeneous devices (macOS ⇄ Android arm64, verified on
real hardware), moves only what the first action needs, and never allows two
writers. Full comparison: [`docs/COMPARISON.md`](docs/COMPARISON.md).

## How it works

- **Continuation levels L0–L4** — from "open this file in a real app on the other
  device" (L0/L1) through structured session continuation with unsaved edits,
  cursor, selection and viewport (L3) to single-writer **authority transfer** (L4).
- **Progressive transfer** — the authoritative closure the first action needs moves
  up front; optional payload is deferred. An over-budget import is refused before a
  single byte moves (§6.8).
- **Integrity** — content-addressed objects are staged, whole-object
  digest-verified, then atomically published (CORE-004). A corrupt, partial or
  tampered byte is never published as valid.
- **Single writer** — epoch-based authority handoff; an interrupted commit leaves an
  ambiguous epoch that blocks writes until explicit recovery (AUTH-001..005).
- **Transport and trust** — TLS 1.3 (rustls, no custom crypto), PIN/QR pairing with
  a mutual verification string, identities and trust sealed in Keychain/Keystore,
  zero-config discovery (engine-side mDNS on macOS, platform NSD on Android; the TXT
  contract is owned and validated by the engine).

## Status

Scope: **macOS + Android** (iOS dropped by owner decision, 2026-10-29). This is a
snapshot of `main` at ABI 1.3; the authoritative tables are
[`docs/TARGETS.md`](docs/TARGETS.md), [`docs/FRONTIER.md`](docs/FRONTIER.md) and
[`apps/README.md`](apps/README.md).

| Claim | Label | Headline evidence |
|---|---|---|
| Zero authoritative-data loss (two-phase publish) | `PROVED` | invariant + fault-injection suite (`xtask fault-injection`) |
| Single-writer authority | `PROVED` (invariant) / `OBSERVED` (physical) | fault injection; Kani 11/11 + stateright interleavings — **model-level**, not an implementation proof |
| Source-off execution | `OBSERVED` (physical) | mac source `kill -9` after handoff; the Galaxy A04 keeps editing, the restarted source reports `may_mutate == false` |
| Pre-action bytes < 50 % of a full transfer | `OBSERVED` | Kiwix D/A 0.1154 % (27 MB ZIM) and 0.0007 % (1.86 GB WikiMed), both LOCAL; holds on the tested grid only |
| Discovery → pairing → handoff in the apps | `OBSERVED` (physical) | NET-003 discovery 177 ms / 537 ms (gate < 10 s); paired mac ⇄ A04 transfer |
| Kiwix companion reader | `OBSERVED` (physical carries) | reading position carried mac ⇄ A04; 45,522 B on the wire for a 27 MB ZIM |
| PhotoCraft desktop flagship | `OBSERVED` (LOCAL) | live session carried into a second live PhotoCraft — two instances on one Mac |
| Swift Package + Kotlin AAR SDKs | `IMPLEMENTED` | ABI 1.3 → SDK 1.3.0; ABI + contract-parity suites green; not published |
| Block-sum checkpoint index | `RESEARCH` | opt-in `blocksum-checkpoint` feature; no transfer path consumes it |
| Signed APK (PLAT-006), §30 platform acceptance | `TARGET` | no platform is "supported" |
| Process/memory migration | `UNSUPPORTED` | by design |

## Repository layout

| Path | What |
|---|---|
| `crates/carryon-core` | the engine: object store + two-phase publish, sealed cuts, authority transfer, budget admission |
| `crates/carryon-net` | networking: TLS 1.3 transport, engine-side mDNS discovery |
| `crates/carryon-ffi` | the C ABI (`include/carryon.h`, ABI 1.3, append-only) |
| `crates/carryon-adapter-api` | the typed adapter contract |
| `crates/carryon-adapter-*` | adapters: file, launcher, editor, image, graph, PhotoCraft, Kiwix, unsupported |
| `prototype/authority-model` | authority-protocol model (Kani + stateright) |
| `prototype/math-core` | block-sum construction (Kani harnesses) |
| `apps/macos` | SwiftUI app over the C ABI |
| `apps/android` | Jetpack Compose app + `carryon-kit` (Kotlin SDK, AAR) |
| `sdk/swift` | Swift Package (engine as a fetched static archive) |
| `xtask` | benchmarks, fault injection, block-sum bench, demo harness |
| `docs` | targets, roadmap, benchmark, comparison, evidence bundles |

## Build and verify

Rust workspace — the same green gate CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets \
  --features carryon-ffi/test-panic,carryon-core/blocksum-checkpoint \
  --exclude carryon-adapter-kiwix -- -D warnings
cargo test --workspace \
  --features carryon-ffi/test-panic,carryon-core/blocksum-checkpoint \
  --exclude carryon-adapter-kiwix
```

`carryon-adapter-kiwix` is excluded because its default `zim-backend` feature links a
locally built libzim; CI checks its pure-Rust lib with `--no-default-features`.

Evidence tooling (LOCAL):

```sh
cargo run -p xtask -- fault-injection docs/evidence/tmp   # -> fault-report.json
cargo run -p xtask -- bench docs/benchmark --reps 30      # independent A/C/D strategies
cargo test -p carryon-authority-model                     # stateright interleavings
```

Apps and SDKs:

- **macOS app** — `apps/macos/Scripts/build_engine.sh`, then `swift build` and
  `swift run carryon-demo-smoke editor` in `apps/macos/`
  ([details](apps/macos/README.md)).
- **Android app** — `apps/android/Scripts/fetch_engine_lib.sh` (NDK r27c), then
  `./gradlew assembleDebug` in `apps/android/` ([details](apps/android/README.md)).
- **Swift SDK** — `sdk/swift/Scripts/build_engine_static.sh`, then
  `swift build && swift test` in `sdk/swift/`.

Physical cross-device campaigns (a Mac and an Android device on one LAN) are listed in
[`docs/TARGETS.md` → How to re-verify](docs/TARGETS.md#how-to-re-verify).

## Documentation

- [`CROSS-PLATFORM-ENGINE-SPEC.md`](CROSS-PLATFORM-ENGINE-SPEC.md) — the specification (the § sections and requirement ids used throughout)
- [`docs/TARGETS.md`](docs/TARGETS.md) — engineering targets, labels and evidence
- [`docs/FRONTIER.md`](docs/FRONTIER.md) — roadmap and progress log
- [`docs/COMPARISON.md`](docs/COMPARISON.md) — vs cloud save, rsync, CRIU, VM migration, demand loading
- [`docs/BENCHMARK.md`](docs/BENCHMARK.md) — preparation-strategy benchmark
- [`apps/README.md`](apps/README.md) — the native apps and their claim table
- [`docs/evidence/`](docs/evidence/) — evidence bundles

## License

Licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](LICENSE-MIT))

at your option, as declared in [`Cargo.toml`](Cargo.toml).

**Third-party note (Kiwix):** builds with the Kiwix backend link
[libzim](https://github.com/openzim/libzim), which is licensed under GPL-2.0. That
covers the Android engine built with `CARRYON_KIWIX=1` (static libzim inside
`libcarryon_ffi.so`, so any APK that bundles it), the macOS `carryon_kiwix`
companion driver, and `carryon-adapter-kiwix` built with its default `zim-backend`
feature. Distributing such binaries is subject to the GPL's terms. The default
engine, the CI builds and the SDKs do not include libzim. Full artifact list and
the proposed release policy:
[`docs/kiwix-bridge/LICENSING.md`](docs/kiwix-bridge/LICENSING.md).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual
licensed as above, without any additional terms or conditions.
