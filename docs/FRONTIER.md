# Frontier system (ecosystem) — roadmap & progress

Living document. The **frontier target** is a continuity ecosystem: the Carry-On
engine + zero-config discovery + signed apps on **macOS, iOS and Android** + an
open SDK + flagship application integrations.

Claim labels are law ([`AGENTS.md`](../AGENTS.md) §2): everything on this page is
**TARGET** unless a status row cites evidence. No platform is "supported" until §30
passes on real hardware; loopback evidence is LOCAL, not physical. When a phase
completes, its rows move to `docs/TARGETS.md` with real evidence — not before.

## 1. Foundation (already real — evidence-backed)

| Milestone | Label | Evidence |
|---|---|---|
| Typed continuation engine, L0–L4, two-phase publish, crash recovery | PROVED (invariants) / IMPLEMENTED | `crates/carryon-core`; green gate (`cargo test --workspace --features carryon-ffi/test-panic`, clippy `-D warnings`, fmt) |
| L4 single-writer authority transfer + split-brain recovery, physical | OBSERVED (physical) | `xdev_handoff.sh`, `unsaved_continue.sh` (mac→Galaxy A04, `kill -9` source) |
| Real external-app continuation (L0 launcher) | OBSERVED (physical) | `appcont_handoff.sh` (file carried, opened in Samsung Gallery) |
| L3 structured session continuation | OBSERVED (physical) | `l3_handoff.sh` (document + unsaved + cursor/selection/viewport/tab + meta) |
| Machine-checked authority protocol | MODEL-level (not implementation proof) | `prototype/authority-model`: Kani 11/11 loop-free harnesses (exhaustive), stateright exhaustive interleavings (§19.3.6 crash windows, loss, duplication, §21.3 flagged recovery) — commit `e8f47af` |
| Statistical preparation-strategy benchmarks | OBSERVED | `xtask bench` (mac 88×30 loopback; Galaxy A04 14×8 on-device), `bench_xdev.sh` (real cross-device campaign) |
| Endpoint-Hamming dilation theorem + application benchmark | PROVED (paper, uncommitted) | `main.tex` — arXiv-ready: PET/FEC related work, terminology map, acknowledgments, Kani/stateright section |

## 2. Frontier phases

### Phase 1 — iOS bring-up · `TARGET` (not started)

Highest technical risk first: the transport model (source listens; destination
connects) collides with iOS background restrictions.

- [ ] Cross-compile engine to `aarch64-apple-ios`; `cargo test` on simulator.
- [ ] Transport-role redesign: iOS as destination-first (connects out), handoff
      while foregrounded; document the role contract as a spec change if needed.
- [ ] Minimal iOS destination app (Swift shell over the C ABI).
- [ ] First physical Mac→iPhone handoff, evidence-bundled (`collect_evidence.sh`).
- [ ] App-suspension survival: `request_suspend`/`resume_import` on real iOS lifecycle.

### Phase 2 — Real discovery · `TARGET` (not started)

- [ ] Bonjour browse on macOS/iOS (Local Network permission flow, Info.plist services).
- [ ] NSD discovery on Android.
- [ ] PIN/QR pairing UX; zero-config < 10 s, NET-003 minimal metadata.

### Phase 3 — Packaging + §30 × 3 platforms · `TARGET` (not started)

- [ ] Signed iOS app (TestFlight), identities recorded (PLAT-006).
- [ ] Signed APK, identities recorded (PLAT-006).
- [ ] PLAT-002 secure storage: Keychain / Android Keystore for device credentials.
- [ ] §30 acceptance harness, run on all three platforms on real hardware →
      only then may a platform be called "supported".

### Phase 4 — SDK + flagship integrations · `TARGET` (research done, no code)

- [ ] Swift Package over the C ABI (macOS/iOS).
- [ ] Kotlin/JNI bindings (Android).
- [ ] `carryon-adapter-photocraft` (L3/L4 desktop flagship — see candidates).
- [ ] `carryon-adapter-kiwix` (mobile mass flagship — see candidates).

### Phase 5 — Theorem into the engine · `TARGET` / `RESEARCH`

Make the `main.tex` block-sum theorem load-bearing instead of parallel:

- [ ] Choose the encoding site (sealed cut / hash-chained journal / store layout).
- [ ] Implement a view-pure checkpointed encoding attaining `q+1`.
- [ ] Measure endpoint dilation on a real hierarchy (leading candidate domain:
      layered-image resolution pyramids, e.g. PhotoCraft documents).
- [ ] Record the connection honestly (update-cost theorem ≠ delivery).

## 3. Flagship application candidates (all `TARGET`, researched 2026-10-27)

| Candidate | Role | Integration path | Platforms | License | Status | Main risks |
|---|---|---|---|---|---|---|
| **PhotoCraft** (`storytold/photocraft`, 74k★) — ArtCraft suite of 13 pure-Rust apps | **primary L3/L4 desktop flagship** | same-language: crate-level or via their JSON control channel + MCP (no fork); layered doc = prerequisites (layers) / optional (pixels) / immediate (selection+tool) | macOS/Win/Linux + Web (WASM); **no native mobile** | Apache-2.0 | research complete, no code | early alpha, API churn; WASM-in-browser is not a §30 platform |
| **Kiwix** (libkiwix) | **mobile mass flagship** | adapter: ZIM path (L1) + current article + position (L3); giant content vs tiny "needed now" shows `bytes_before_first_action` | **macOS/iOS/Android**/Win/Linux | open source | Phase 1 done: research (`docs/kiwix-bridge/RESEARCH.md`), design (`ADAPTER-DESIGN.md`), build feasibility (`BUILD-EVIDENCE.md`), skeleton crate (`crates/carryon-adapter-kiwix`, libzim backend UNSUPPORTED) | C++/JNI bridging; libkiwix Android cross needs Linux host |
| **Delta Chat** (Rust core, C-FFI) | backup L4 candidate | patch core (Rust) or wrap C FFI; session = draft + chat + scroll | iOS/Android/desktop | open source | reserve | someone else's core; messenger UX expectations |
| **Zotero** (open-source iOS app) | academic alternative | Swift-side adapter: PDF + annotations + reading position | iOS/Android/desktop | open source | reserve | desktop app is JS; narrow audience |
| **Godot / KiCad** | optional desktop "wow" | editor session adapter (scene/PCB + unsaved edits) | desktop only | MIT | reserve | no mobile; large codebases |
| **KOReader** | reserve | position + highlights session | Android/desktop (no iOS) | AGPL | reserve | breaks 3-platform story |
| **Logseq / Joplin** | reserve | notes + draft | all 3 | open source | reserve | TS/RN integration; small files (nothing progressive to shine) |

L0/L1 works with **any** file-based app today (no integration) — physically
demonstrated via Samsung Gallery (`appcont_handoff.sh`).

Flagship pairing rationale: **PhotoCraft shows depth** (L4 authority, Rust-native
integration, theorem-adjacent domain), **Kiwix shows breadth** (native iOS+Android,
massive content, mass audience).

## 4. Progress log

| Date | Entry |
|---|---|
| 2026-10-27 | Machine-checked authority protocol landed: `prototype/authority-model` — Kani 11/11 exhaustive harnesses + stateright exhaustive interleavings (commit `e8f47af`). MODEL-level evidence added to TARGETS.md #3. |
| 2026-10-27 | Paper (`main.tex`) updated to arXiv-ready: PET/FEC related work (Albanese et al. 1996; Lin & Costello) per A. Rudra's pointer, terminology map per M. Wootters' feedback, acknowledgments (non-endorsing), Kani/stateright verification note (§9). Uncommitted by design. |
| 2026-10-27 | Flagship candidate research completed (table above); frontier direction + phases fixed in `AGENTS.md` + this file. |
| 2026-10-28 | Kiwix bridge Phase 1 (branch `kiwix-bridge`): repos cloned+pinned, session models researched (Android Room/DataStore + libkiwix XML; iOS CoreData + UserDefaults), libzim chunk API confirmed (`Item::getData(offset,size)`, `getDirectAccessInformation`), build feasibility PROVED for libzim native + Android arm64 (NDK r27c) + iOS arm64 staticlib (manual meson AND canonical kiwix-build, xapian+icu incl.); libkiwix cross-build from macOS honest-negative (needs Linux CI or kiwix-build Apple path); `carryon-adapter-kiwix` skeleton landed (tests/clippy/fmt green, libzim backend UNSUPPORTED by design). Evidence: `docs/kiwix-bridge/`. |
