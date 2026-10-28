# `carryon-adapter-photocraft` — adapter design (Phase 1)

> Design doc. Nothing here is implemented yet except the Phase-1 skeleton crate
> (§6); every claim about PhotoCraft behavior cites
> [`RESEARCH.md`](RESEARCH.md) (pin `1f7fc0f5…`, see
> [`PHOTOCRAFT-PIN.md`](PHOTOCRAFT-PIN.md)). Status labels follow
> [`../TARGETS.md`](../TARGETS.md): the design is **TARGET**, the skeleton is
> **IMPLEMENTED** (stubbed), driving the real PhotoCraft is **UNSUPPORTED**
> until a build log says otherwise.

Pattern: follows `crates/carryon-adapter-editor` (the reference L3/L4 adapter)
exactly — same object taxonomy, same action vocabulary, same authority-binding
discipline. Divergences from the editor pattern are called out explicitly.

## 1. Object model

A PhotoCraft working session (one open document being edited) maps to four
typed objects, mirroring the editor adapter's four:

| object id | kind | dependency role | content | parents |
|---|---|---|---|---|
| `photocraft.document.v1` | Authoritative | prerequisite | the saved document as single-file `.pcraft` (zip) bytes — lossless native format; carries layers, pixels, masks, effects, channels, guides, metadata **and the selection** (RESEARCH §1.2, §1.5) | — |
| `photocraft.unsaved_edits.v1` | Authoritative | prerequisite | Phase 1: `.pcraft` snapshot of the *dirty* state (autosave-style, RESEARCH §1.3.3). Phase 2 (in-process driver only): the command journal `[[id, params], …]` sliced from the last saved revision (RESEARCH §1.3.1) | `photocraft.document.v1` |
| `photocraft.meta.v1` | Authoritative | prerequisite + provenance | JSON: original path, document name, `revision`, `saved_revision`, `dirty`, PhotoCraft version pin, format choice (`pcraft`/`psd`), journal mode (`snapshot`/`journal`) | — |
| `photocraft.workspace.v1` | Ephemeral | optional | JSON workspace position: active document index, selected/target layers (names + occurrence, RESEARCH §2 of control protocol — portable across documents), viewport (`zoom`, `center`, `rotation`), active tool, panel state | — |

Rationale for the split:

- Only `Authoritative` objects seal into the cut (core-enforced, same as the
  editor adapter); `workspace` is published-but-optional so a progressive /
  action-conditioned carry can defer it (§6.6).
- **Selection is deliberately inside the document object, not the workspace
  object**: PhotoCraft persists the selection in `.pcraft`
  (`DocM.selection`, RESEARCH §1.2). This differs from the editor adapter,
  where selection is ephemeral navigation. A selection that exists only in the
  dirty state arrives with the unsaved-edits object; restoring it needs no
  extra object. (A *future* journal-mode caveat: selection steps replay only if
  the selection is derived from commands — PhotoCraft journalizes them
  because selection changes are history steps, RESEARCH §1.2.)
- **History/snapshots are not carried**: not serializable in PhotoCraft
  (`Arc<Document>` snapshots, RESEARCH §1.2) — same as the editor adapter, which
  also carries no undo history. Honest loss, documented.
- Content-addressed tiles inside `.pcraft` (BLAKE3, RESEARCH §1.5) make the
  document object the natural future encoding site for Phase-5 theorem work
  (endpoint dilation on layered-image pyramids) — noted, not claimed.

## 2. Actions

Two actions, mirroring the editor adapter's vocabulary:

### 2.1 `document.edit` (mutating, L4 single-writer)

Params: `{"steps": [["<command id>", {params}], …]}` — the exact
`[[id, params], …]` shape PhotoCraft's actions/batch/droplets consume
(RESEARCH §1.3.2); each step dispatched by command id with JSON params.

Semantics: replay the steps against the session's document (the destination's
reconstructed dirty state), then report:

```json
{"revision": <new>, "document_hash": <sha256 of new dirty snapshot bytes>,
 "steps_ran": <n>, "steps_failed": [{"step": <i>, "id": "...", "error": "..."}]}
```

Oracle: re-read the document state after replay (`session.list` /
`document.inspect`) and verify `revision` advanced by the number of applied
steps and the reported hash matches a fresh serialization — checked=true,
agreed only if they match. Partial application is reported, not rolled back
(PhotoCraft `command_batch` is explicitly not atomic — RESEARCH §2.4).

The core gates whether this device *may* mutate (single-writer, AUTH-001..005);
the adapter refuses `document.edit` when it does not hold authority — fail
closed, same as the editor adapter relies on the core for.

### 2.2 `session.restore` (read-only)

Params: `{}`. The destination proves it holds the same logical session:

```json
{"document_hash": ..., "revision": ..., "dirty": true/false,
 "selected_layers": [...], "has_selection": ..., "workspace": {...}}
```

Oracle: the restored authoritative-state hash (§3) must equal the one sealed
in the cut — agreed only on exact match.

## 3. Authority transfer binding (§21.2)

Identical discipline to the editor adapter: the proposal binds to the cut and
the **authoritative-state hash**

```
auth_hash = SHA256( len(document) || document || len(unsaved) || unsaved || meta_bytes )
```

never to a bearer token or a process. The destination re-derives `auth_hash`
from the state it imported (document + unsaved + meta) and must agree before
authority commits. `prepare_authority_transfer` (source) /
`accept_authority_transfer` (destination re-derivation check) /
`commit_authority_transfer` (source relinquish, receipt-bound) map 1:1 to the
editor adapter's implementation; only the byte sources differ.

Power asymmetry documented: PhotoCraft's control token grants *full* editor
control (RESEARCH §2.4); the Carry-On proposal deliberately carries no such
capability — data only (§3.8), and the adapter never forwards PhotoCraft-side
capabilities across the carry.

## 4. Integration path options

The adapter (compiled-in, in the Carry-On host process) must reach PhotoCraft
through one of three supported seams (RESEARCH §2.6 — no fork in any of them):

| | **A. Desktop control channel** (`photocraft --control`) | **B. Headless serve** (`photocraft-cli serve`, stdio) | **C. In-process engine dep** (`photocraft_engine::Session` embedded) |
|---|---|---|---|
| What it drives | the live desktop app, incl. unsaved documents, UI, viewport | a headless engine session the adapter owns | the engine directly in the host process |
| Reach unsaved edits | yes — `app.save {path}` snapshots the dirty doc under the write root | yes — `doc.save {path}` | **best** — direct `Session.journal` slice (fine-grained `[[id,params]]`, RESEARCH §1.3.1) |
| Restore workspace (viewport/panels) | yes — `ui.set` / `ui.menu.invoke` | no (no UI exists; only session-level state) | no (engine only; UI state lives in ui-egui, RESEARCH §1.2) |
| Coupling | none at compile time (JSON protocol, pinned by docs) | none at compile time | compile-time: git dep on the alpha crate (pinned rev), heavy dep tree |
| Transport ceilings | 1 MiB req / 8 MiB reply; bytes must travel via automation-root files | same | none (in-memory) |
| Trust | bearer token = full app control; loopback only; adapter must whitelist methods, never proxy raw | stdio inherited access; capability roots fail-closed | `Session::authorize` hook gives per-command fail-closed policy in-process (RESEARCH §2.5) |
| Version churn risk | method/command surface (documented, fairly stable) | same | Rust API surface (moves fastest in an alpha) |
| Process boundary | adapter and app must share a machine (loopback) | adapter spawns the CLI child | none — same process as Carry-On host |

Trade-off summary: **A** is the only path that restores the *live editor
experience* (viewport, panels, the actual app the human sees). **C** is the only
path that reaches the command journal, which is where progressive transfer
(_targets #4_, the <50% pre-action-bytes story) and Phase-5 theorem encoding
get their leverage. **B** is the degenerate middle: as capable as C for
document work, but still behind a pipe and without journal access.

## 5. Recommendation

**Phase 2 primary: C (in-process engine dep), behind a driver seam; A
(control channel) as the first milestone and permanent fallback.**

Reasoning:

1. The Carry-On flagship story for PhotoCraft is *depth*: L4 authority over a
   real editor with unsaved edits. Both A and C can carry unsaved state as a
   snapshot `.pcraft`; only C can carry it as a **journal** — smaller
   pre-action bytes (target #4), incremental growth, and the action-conditioned
   planning the engine exists to exploit. Snapshot-only mode makes the
   unsaved object nearly as big as the document and wastes the layered
   structure.
2. C's costs are real but bounded: a pinned git dependency on an alpha crate
   (compile-time churn) is acceptable for a *research flagship* inside Carry-On,
   and the adapter isolates it behind one module (`driver.rs`, §6).
3. A must exist anyway to restore the live workspace object (viewport/panels)
   on a machine where the human continues in the real app — and it proves the
   no-coupling story first, with the smallest possible Phase-2 change.
4. B adds no capability A or C lack; it is a debugging convenience (spawn a
   clean headless session for tests) and should be used exactly that way in
   the adapter's test harness.

Concretely: the skeleton (§6) defines `trait PhotocraftDriver` with
`InProcessDriver` (Phase 2b, UNSUPPORTED stub) and `ControlChannelDriver`
(Phase 2a, UNSUPPORTED stub) implementations; the object model, hashing,
manifest, and authority logic are driver-independent and land first.

## 6. Phase-1 skeleton (`crates/carryon-adapter-photocraft`)

**IMPLEMENTED** as in-memory stubs over the object/action/authority model
above; every PhotoCraft-touching operation is `UNSUPPORTED` and fails closed
(`AdapterError::Incompatible`/`ActionUnsupported` naming the missing driver).
It compiles, tests, clippy `-D warnings`, and fmt in the Carry-On workspace
without any PhotoCraft dependency — the value is the contract: schema ids,
action names, `auth_hash` construction, dependency plans, and the
state-binding of authority transfer, all pinned by tests the real driver must
keep green.

Layout (mirroring `carryon-adapter-editor`):

```text
crates/carryon-adapter-photocraft/
├── Cargo.toml        # deps: carryon-adapter-api, serde, serde_json, sha2 (same as editor)
├── src/
│   ├── lib.rs        # PhotocraftAdapter: Adapter impl (stub driver, real object model)
│   ├── model.rs      # object schemas, meta/workspace JSON types, auth_hash
│   └── driver.rs     # PhotocraftDriver trait + StubDriver (UNSUPPORTED), Phase-2 seams
└── tests/
    ├── structured_session.rs   # L3: four objects seal/publish correctly, session.restore
    └── authority_binding.rs    # L4: auth_hash binding accepts/refuses correctly
```

Explicitly out of scope for the skeleton (labeled in code): any real file I/O,
control-channel client, engine embedding, journal capture, workspace restore.

## 7. Phase 2 results (status notes; the orchestrator owns FRONTIER/TARGETS)

### 7.1 ControlChannelDriver — OBSERVED (LOCAL)

The live L3 carry works end to end, evidence in
[`evidence/l3-live/transcript.json`](evidence/l3-live/transcript.json) +
screenshots: live app A (dirty document: new layer, paint stroke, selection,
tool Brush, zoom 0.5, view rotation 15°) → driver pulls document/unsaved/meta/
workspace → source core seals the cut (57,659 B over loopback TLS, 0.199 s) →
destination core imports → reconstructs the three authoritative objects from
its own store by digest → registers the reconstructed adapter with a live
driver → `session.restore` drives app B (open carried `.pcraft`, activate,
tool, viewport, layer select) → independent `ui.inspect` verification: zoom
0.5==0.5, rotation 15==15, tool Brush==Brush, selection bounds
[160,120,320,200] (arrived inside the carried document bytes — selection is
document state), carried layer present. Oracle agreed, `live_restore=true`.

Discoveries that shaped the implementation (all hit live, all recorded):

1. **A layered `app.save` marks the document CLEAN and rebinds its path**
   (`session.list` `dirty:false` after save, RESEARCH §1.4). Saving the
   original to obtain "the unsaved state" would destroy the very state being
   carried. The driver therefore snapshots via `image.duplicate` +
   `app.save`-the-duplicate + `file.close`, verifying the duplicate is active
   before the save and failing closed otherwise.
2. **The optional split is load-bearing**: `import_cut` transfers only the
   authoritative cut (`cut_manifest` filters `role='authoritative'`,
   `crates/carryon-core/src/transfer.rs`); the ephemeral workspace object is
   published on the source but never sealed into the cut — verified live
   (`has_object_hex` false on the destination). Selection needed no separate
   object because `.pcraft` serializes it as document state.
3. `app.save`/`app.open`/`ui.set` are **control methods**, not engine
   commands; `engine.execute` refuses them (`unknown command app.save`). The
   driver separates the two layers explicitly.
4. `app.save` requires the target's **parent directory to already exist**
   under the automation write root; the driver creates the scratch dir.

### 7.2 InProcessDriver — IMPLEMENTED; end-to-end replay proven in tests

Feature `inprocess` (pinned git dependency `photocraft-engine`, rev
`1f7fc0f5…` — the local clone as a `file://` git source; the rev pin is what
matters, `PHOTOCRAFT-PIN.md`). Adds what the control channel cannot give:

- **Journal-mode unsaved edits**: `Session.journal` sliced from a checkpoint
  taken at the last save, serialized in PhotoCraft's native actions shape
  `[[id, params], …]` — replayable by `Session::execute`, actions, droplets.
- **True `saved_revision`** in the meta object (the control surface hides it).
- **Fail-closed `Session::authorize`** (`authorize_policy`): read-only
  commands, a curated edit allowlist, and `file.new`/`save`/`open`/`duplicate`
  only under the explicitly granted work root (traversal denied) — the
  in-process analogue of PhotoCraft's own `AuthorizedWorkspace`.
- **Replay oracle**: journal replay on a clean session must land on the same
  revision and produce a dirty-state snapshot that hashes equal to the
  source's (pinned by `journal_slice_and_replay_semantics_end_to_end`).

Honest limits: headless means no viewport — `restore` activates the document
and selects the layers by name+occurrence, but the viewport fragment is
carried, not applied (the engine has no view state; `ui-egui` owns it).

### 7.3 The flagship measurement — see `evidence/journal-vs-snapshot.json`

(Filled after the measurement run; numbers quoted in the Phase-2 final report.)

### 7.4 The 16 MiB object cap — PROPOSAL ONLY (orchestrator decision)

(Filled after real `.pcraft` sizes are measured; the options are in §7.4
below. The core's cap is NOT changed unilaterally.)
