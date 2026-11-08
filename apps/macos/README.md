# Carry-On for macOS

A native SwiftUI wrapper over the **real** Carry-On engine C ABI
(`crates/carryon-ffi/include/carryon.h`, ABI 1.2 — 1.1 plus the stage-4 discovery entries, append-only). One device, two in-process
cores, real TLS 1.3 loopback, mutual pin pairing, L4 single-writer authority
transfer — the full 8-stage demo pipeline from `plan.md`, driven end-to-end
through the FFI.

## What this is (claim labels, §2 law)

| Piece | Label | Why |
|---|---|---|
| Swift binding (`CarryOnKit` over `CCarryon`) | **IMPLEMENTED** | Binds the existing C ABI only; no Rust changes, no new `#[no_mangle]` symbols. Verified by `swift build` + the smoke run. |
| Demo pipeline (8 stages incl. L4 authority transfer) | **OBSERVED (LOCAL)** | One device, loopback TLS 1.3, two temp-dir cores. Proves the FFI binding and the engine pipeline. NOT physical cross-device transfer. |
| **"Carry File" screen — L0/L1 ladder (stage 3, manual-IP)** | **OBSERVED (PHYSICAL)** when the receiver is a real second device | NSOpenPanel pick (last dir cached) → file adapter cut → hand-entered receiver `IPv4:port` (cached) + pin → serve. The descriptor (session/cut/hash/name/mime/evidence-class) rides the Hello/Welcome features — data only (§3.8). Evidence class derived from the dialled address per run (loopback → LOCAL, LAN → PHYSICAL). Physical evidence: [`apps/android/evidence/20261108-a04-l0l1/`](../android/evidence/20261108-a04-l0l1/). Headless driver: `swift run carryon-filehandoff-smoke serve <file> <addr> <pin> [--tamper]`. |
| macOS platform | **supported: NO** | §30 acceptance never ran; no notarized/Developer-ID packaging. The bundle is ad-hoc signed, LOCAL-ONLY (explicitly not PLAT-006 distribution signing). |
| Discovery (engine-side §18.3, stage 4) | **engine + app UI** | The engine owns the TXT contract: `carryon_discovery_txt_encode`/`_validate` (ABI 1.2) + engine-side advertise/browse (`mdns-sd`) exposed to the app via `EngineDiscovery`. The NWListener/NWBrowser `DiscoveryPilot.swift` remains as a stage-3 pilot artifact (unused). NET-003 PHYSICAL: engine browse found the A04 in 537 ms (2026-11-08, `docs/evidence/20261108-net003-discovery/`). TXT records carry only `pm`, `id`, `pr`, `dc`, `ch` — foreign keys ignored, malformed records dropped (fail closed). |
| **Swift SDK (stage 9): `sdk/swift`** | `IMPLEMENTED` + contract tests | The same binding, distributed as a source package with the engine as a fetched static archive (no `unsafeFlags` → consumable as a dependency), DocC catalog, semver following the ABI (1.2 → SDK 1.2.0), ABI gate at startup. `Scripts/sync_from_app.sh` keeps the SDK sources identical to this package (drift-checked). See [`sdk/swift/README.md`](../../sdk/swift/README.md). |
| Secret handling | **IMPLEMENTED (law)** | Identity key DER is SECRET (EVD-006/§22.3): never logged, displayed, or bundled. Only the SHA-256 pin hex appears, truncated, in UI. |

The fixed footer every run ends with (verbatim, `plan.md`):

```
DISCLOSURE: LOCAL run on ONE DEVICE via loopback TLS 1.3. Proves the FFI binding and
engine pipeline, NOT physical cross-device transfer or a supported platform (spec §2/§30).
```

## Layout

```
apps/macos/
├── Package.swift              # tools 5.9 (Swift 5 language mode), macOS 14.0, @Observable
├── Sources/
│   ├── CCarryon/              # C target: vendored carryon.h + module map
│   ├── CarryOnKit/            # Swift binding: core/net/evidence/demo pipeline/discovery
│   ├── CarryOnApp/            # SwiftUI app (onboarding, demo, topology, sessions, evidence)
│   └── carryon-demo-smoke/    # headless verification executable
├── Scripts/
│   ├── sync_header.sh         # copy carryon.h from the crate (drift-checked; --force to re-sync)
│   ├── build_engine.sh        # cargo build -p carryon-ffi --release + vendor dylib (+ install_name)
│   └── make_app.sh            # assemble dist/CarryOn.app + ad-hoc codesign + headless check
└── Vendor/                    # libcarryon_ffi.dylib (gitignored — fetched, never committed)
```

## Build & verify

```sh
# 1. sync the ABI header (fails on drift; re-run with --force after reviewing)
apps/macos/Scripts/sync_header.sh

# 2. build the real engine and vendor the dylib
apps/macos/Scripts/build_engine.sh

# 3. build everything (must be clean)
cd apps/macos && swift build

# 4. headless verification: full L3 demo pipeline, exits 0 iff evidence verifies ok:true
swift run carryon-demo-smoke
#    variants: graph | --chunk 4096 | --budget | --suspend

# 5. assemble + codesign + launch-check the app bundle
apps/macos/Scripts/make_app.sh
open dist/CarryOn.app
```

`build_engine.sh` defaults `CARGO_TARGET_DIR` to the repo's canonical external
dir (AGENTS.md); override by exporting your own before calling.

## What the demo does (stages 1–8)

1. **OPEN** — `carryon_core_open` ×2 in temp dirs (+ recovery report fields).
2. **IDENTITY** — `carryon_identity_generate` ×2; truncated pin fingerprints shown.
3. **PAIR** — `carryon_pair_devices` → `carryon_trust_from_json` each side.
4. **SESSION** — `org.carryon.editor` with `{"session_v1":true}` (or
   `org.carryon.graph` `{"sample":true}`), `carryon_create_session` + `carryon_create_cut`.
5. **TRANSFER** — `carryon_listener_bind` → connect/accept (TLS 1.3, mutual pin)
   → negotiate → `carryon_serve_cut` ⇄ `carryon_import_cut` (live byte counters).
6. **AUTHORITY** — `carryon_may_mutate` false on the mirror →
   `carryon_request_authority_transfer` ⇄ `carryon_serve_authority_transfer`
   (durable receipts, `new_epoch`) → may_mutate flips true on dest, false on source.
   The graph scenario skips this stage honestly (`supports_authority_transfer:false`).
7. **EDIT** — transport disconnected; dest `carryon_execute_action_as`
   (`document.edit` / `graph.shortest_path`); oracle `agreed` shown.
8. **EVIDENCE** — `carryon_export_evidence_with` (continuation metrics mirroring
   `carryon_l3.c`) → `carryon_verify_evidence` → `{"ok":true,...}`.

Advanced toggles: destination chunk size (§18.6), budget rejection shown as a
NORMAL fail-closed outcome (§6.8 — partial admission or a typed refusal, code
500), suspend/resume (§11.2), and recovery-report fields (§19.3) at OPEN.

## Honest notes

- The two "devices" are two cores in the same process on the same machine.
  Nothing here exercises a physical network path or a second NIC.
- `org.carryon.graph` declares `supports_authority_transfer:false`; the AUTHORITY
  stage is labeled skipped for that scenario — labels stay law (§2).
- Bundle signing is ad-hoc (`codesign --sign -`): LOCAL-ONLY, not PLAT-006.
- Discovery is ENGINE-CONTRACT (stage 4): the engine builds/validates the §18.3
  TXT and (on macOS) advertises/browses itself (`mdns-sd`); the app renders what
  the engine reports. TXT records carry no user/session/app
  names, no object hashes, and no key material (§18.3).
