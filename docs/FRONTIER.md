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

### Phase 4 — SDK + flagship integrations · `TARGET` (Phase 1 done on both flagship branches — unmerged; Phase 2 dispatched)

- [ ] Swift Package over the C ABI (macOS/iOS).
- [ ] Kotlin/JNI bindings (Android).
- [ ] `carryon-adapter-photocraft`:
  - [x] Phase 1 — research, adapter design, skeleton (16 tests, orchestrator-verified green) on branch `artcraft-bridge` (unmerged). Engine + CLI cross-builds **OBSERVED (LOCAL cross-compile, no device)**: aarch64-linux-android (NDK r27c) and aarch64-apple-ios, exit 0, logs in-repo. Native mobile GUI verdict: not realistic in months; realistic mobile editor path = the shipped WASM build (usability untested); adapter path = the headless engine.
  - [x] Phase 2 — ControlChannelDriver: **live L3 carry OBSERVED (LOCAL)** (live app → loopback TLS 57,659 B / 0.199 s → second live app, GUI-verified zoom/rotation/tool/selection restore); InProcessDriver: journal-mode carry + fail-closed authorize + composite-pixel replay oracle (IMPLEMENTED); journal-vs-snapshot measurement **OBSERVED (LOCAL)**: journal = 19–36% of snapshot across the tested grid (target #4 <50% holds on every document); unsaved object 90 B vs 7,419,132 B in the photo case. `app.save`-destroys-state finding recorded; EVD-006 verified (bearer tokens absent from all evidence).
  - [x] Phase 3 — photo journal replay (397 B journal vs 7,022,517 B snapshot, composite oracle match); **L4 + source-off proof with two LIVE apps (21/21 checks, OBSERVED, LOCAL — kill -9 source, dest keeps editing, restarted source reports AUTH_ReadOnly)**; **PHYSICAL mac→Galaxy A04 carry (OBSERVED, physical — 57,663 B LAN TLS, on-device composite render hash MATCH)**; edit-stream: journal ~117 B/step, no crossover < ~100k steps (extrapolation, labeled).
  - [ ] Phase 4 — **tile-splitting series (owner-APPROVED option B)**: design-vs-spec first, two-phase publish per tile, progressive budget admission, then adapter use + re-measurement; physical L4 promotion (device becomes single writer); (stretch) harness CI-ification. **In progress in a fresh session** (`ses_e79422fa2ffe1LV5Tv7aLNgJqN`).
- [ ] `carryon-adapter-kiwix`:
  - [x] Phase 1 — research, adapter design, skeleton (8 tests, orchestrator-verified green) on branch `kiwix-bridge` (unmerged). Sources pinned: libzim 9.8.2 `ad888b4`, libkiwix 14.3.0 `c8a4b8a`, kiwix-android `8fd5b93`, kiwix-apple `f410135`. Session models mapped (Android DataStore/Room, iOS CoreData `Tab.interactionState`); libzim chunk API confirmed (`getData(offset,size)` + direct access) → progressive bulk feasible. Builds **IMPLEMENTED**: libzim/libkiwix macOS, libzim→Android arm64, libzim→iOS staticlib via kiwix-build (SSD sparse image `/Volumes/kbwork`). libkiwix→Android **UNSUPPORTED** (Linux-host-only per kiwix-build) — honest negative, non-blocking (reading needs only libzim). Surface: standalone companion reader, no fork.
  - [x] Phase 2 — libzim C-ABI backend (C++ shim → C ABI → Rust FFI) **IMPLEMENTED** (host 12/12 tests incl. 4 real-ZIM; Android arm64 static link; iOS compile); **on-device probe + real session capture OBSERVED (physical)** on the Galaxy A04 — real Kiwix app session (27 MB ZIM, uuid cross-check passed), Chromium bundle carried but DROPPED-labeled; flagship measurement **OBSERVED (LOCAL, real bytes)**: A = 27,041,691 B vs D = 31,205 B → **D/A = 0.1154% (866.6×)** — target #4 passes by ~434×.
  - [x] Phase 3 — **physical LAN carry of the real session (45,522 B, OBSERVED, physical)**; **L4 over the reading position ON HARDWARE + reader.navigate on device after transfer (physical)**; **progressive ZIM materialization: article readable at 23.787% (physical)**; portable authority hash (uuid+size+sha256, cross-path regression); GB-scale math on WikiMed 1.86 GB: D/A = 0.0007% (138,769×, LOCAL); ZIM bulk correctly lives as L1 outside the cut.
  - [ ] Phase 4 — iOS simulator run (closes the 3rd platform at OBSERVED, simulator); companion-shell rendered first paint; materialization refinement (title-listing blob only, ~126 KB vs 3.23 MB cluster); physical scale-up with the 1.7 GB WikiMed; engine decision points (partial-fetch API, tiered caps) REPORT-ONLY — coordinated with the parallel tile-splitting core work. **In progress in a fresh session** (`ses_e79422fa8ffeBjkSZOXcJ7I4o0`).

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
| **PhotoCraft** (`storytold/photocraft`, 74k★) — ArtCraft suite of 13 pure-Rust apps | **primary L3/L4 desktop flagship** | same-language: crate-level or via their JSON control channel + MCP (no fork); layered doc = prerequisites (layers) / optional (pixels) / immediate (selection+tool) | macOS/Win/Linux + Web (WASM); **no native mobile** | Apache-2.0 | **Phase 1 done** (`artcraft-bridge`, unmerged) | early alpha, API churn; WASM-in-browser is not a §30 platform |
| **Kiwix** (libkiwix) | **mobile mass flagship** | adapter: ZIM path (L1) + current article + position (L3); giant content vs tiny "needed now" shows `bytes_before_first_action` | **macOS/iOS/Android**/Win/Linux | open source | **Phase 1 done** (`kiwix-bridge`, unmerged) | libkiwix→Android UNSUPPORTED (reading needs only libzim) |
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
| 2026-10-28 | **ArtCraft Phase 1 complete** (`artcraft-bridge`, unmerged, orchestrator-verified): RESEARCH/ADAPTER-DESIGN/MOBILE-FEASIBILITY + `carryon-adapter-photocraft` skeleton (16 tests green). Engine + CLI cross-compile OBSERVED (LOCAL) for Android + iOS; native mobile GUI verdict: not in months (realistic mobile path: WASM build). Integration rec: in-process engine + control channel. |
| 2026-10-28 | **Kiwix Phase 1 complete** (`kiwix-bridge`, unmerged, orchestrator-verified): RESEARCH/BUILD-EVIDENCE/ADAPTER-DESIGN + `carryon-adapter-kiwix` skeleton (8 tests green, fail-closed libzim backend). libzim builds IMPLEMENTED (macOS/Android/iOS); libkiwix→Android UNSUPPORTED (honest negative). Session models mapped on both apps; surface rec: companion reader, no fork. |
| 2026-10-28 | **Phase 2 dispatched** to both sessions: artcraft → ControlChannelDriver + InProcessDriver + journal-vs-snapshot measurement; kiwix → libzim C-ABI backend + real Android capture (Galaxy A04) + first real-ZIM D/A measurement. |
| 2026-10-28 | **ArtCraft Phase 2 complete** (`artcraft-bridge`, orchestrator-verified where checked): first LIVE L3 carry with GUI-verified restore (OBSERVED, LOCAL); InProcessDriver + replay oracle (IMPLEMENTED); journal-vs-snapshot: journal 19–36% of snapshot, target #4 holds on the grid. `app.save` state-destruction finding + non-destructive duplicate-snapshot workaround. Pending: 16 MiB cap — owner decision. |
| 2026-10-28 | **Kiwix Phase 2 complete** (`kiwix-bridge`, orchestrator-verified 12/12): libzim backend real on host + Android static link + on-device probe (OBSERVED, physical, Galaxy A04); real session capture from the real app (physical); D/A = 0.1154% (866.6×) on a 27 MB ZIM. libkiwix→Android UNSUPPORTED stands. |
| 2026-10-28 | **Phase 3 dispatched** to both sessions: artcraft → photo journal replay + live-app L4 + source-off proof + physical LAN carry + real-edit-stream; kiwix → physical LAN carry + on-device L4 + progressive ZIM materialization + iOS link + GB-scale measurement. |
| 2026-10-28 | **Owner decision: 16 MiB core object cap → option B (per-tile splitting)** — manifest as one object, BLAKE3 tiles as parent-linked content-addressed objects. Queued into the artcraft session after its Phase 3 scope. |
| 2026-10-29 | **ArtCraft Phase 3 complete** (`artcraft-bridge` 4585ec5, orchestrator-verified 32/32): photo journal replay (oracle match); LIVE L4 + source-off proof with the real app (OBSERVED, LOCAL); **PHYSICAL mac→Galaxy carry with on-device composite render match (OBSERVED, physical)**; edit-stream journal ~117 B/step, crossover ≈100k steps away (extrapolation). |
| 2026-10-29 | **Kiwix Phase 3 complete** (`kiwix-bridge`, orchestrator-verified 14/14): physical LAN carry (45,522 B) + on-device L4 navigate (OBSERVED, physical); progressive materialization readable at 23.787% (physical); portable authority-hash fix; WikiMed GB-scale: D/A = 0.0007% (LOCAL). iOS simulator cut from scope — compile-level only, honestly labeled. |
| 2026-10-29 | **Context handoff**: Phase 4 dispatched to two FRESH sessions in the same worktrees/branches (artcraft → tile-splitting + physical L4; kiwix → iOS sim + first paint + scale-up). Engine decision points from kiwix (partial-fetch API, tiered caps) routed REPORT-ONLY to avoid two branches editing core concurrently. |
