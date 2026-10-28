# PhotoCraft research — ArtCraft bridge, Phase 1

> Status: **RESEARCH** (document/model claims) unless a row cites a build log or a
> live run. All file/line citations are against pin `1f7fc0f53696e54da79b95015abb37d7c22cdd1a`
> ([`PHOTOCRAFT-PIN.md`](PHOTOCRAFT-PIN.md)); the project is an early alpha that
> moves fast — dozens of merges per day — so re-verify before use.

PhotoCraft is the primary L3/L4 desktop flagship candidate of the Carry-On
frontier ecosystem ([`../FRONTIER.md`](../FRONTIER.md) §3). This document answers
the three Phase-1 research questions: (a) the document model, (b) the external
control surfaces, (c) continuation semantics. Mobile feasibility evidence is in
[`MOBILE-FEASIBILITY.md`](MOBILE-FEASIBILITY.md); the adapter design built on
these findings is in [`ADAPTER-DESIGN.md`](ADAPTER-DESIGN.md).

Scale at pin: 25 library crates + 4 apps, ~539,000 lines of Rust, ~8,040 tests
(`docs/architecture.md` §"Current state", lines 10–14). Engine: ~700 registered
commands dispatched by id from UI, CLI, JSON control channel and MCP
(`docs/architecture.md` lines 16–19).

## 1. Document model

### 1.1 What a document is

`photocraft-doc` (L1 in the enforced layering, `docs/architecture.md` §3) holds
the model. `Document` (`crates/doc/src/lib.rs:867-930`) derives only
`Clone, Debug, PartialEq` — **no serde derives**; persistence goes through a
deliberately separate versioned manifest model (`crates/format`, §1.5). Fields:
`id: DocId`, `name`, `size`, `resolution_dpi`, `mode: ColorMode`,
`depth: SampleType`, `icc_profile: Option<Arc<Vec<u8>>>`, `layers: Vec<Layer>`
(bottom-to-top), `channels: Vec<AlphaChannel>`, `guides`, `ruler_origin`,
**`selection: Option<Surface>`** ("Active selection as a grayscale coverage
surface", line 885), `metadata`, `global_light`, `paths`/`work_path`/
`clipping_path`, `quick_mask`, `patterns`, `layer_comps`, `measurement`,
`notes`, `text_styles`, `slices`, `variables`, `timeline`, `file_encoding`.

`LayerContent` (`crates/doc/src/lib.rs:585-594`) is the non-destructive enum:
`Raster(Surface) | Group(Group) | Adjustment(Adjustment) | Fill(Fill) |
Text(TextLayer) | Shape(ShapeLayer) | Smart(SmartObject)`. Groups nest via
`Group.children: Vec<Layer>` (`children[0]` is the bottom layer, `lib.rs:3-4`).
`Layer` (`lib.rs:618-670`) holds per-layer authoritative data: name, visible,
locks, blend, opacity, fill_opacity, clipped, `mask: Option<LayerMask>`,
`vector_mask`, `effects`, `psd_blocks` (lossless PSD round-trip), `blend_if`,
plus caches. `SmartObject` carries a smart-filter stack where a `SmartFilter`
(`lib.rs:562-570`) is literally "a command id plus its parameters (data, not
code)": `pub command: String, pub params: serde_json::Value`.

Pixels: 256×256 copy-on-write `Arc`-shared tiles (`docs/architecture.md` §5
line 303); every large payload in the document model is `Arc`-shared
(`docs/architecture.md` §1 principle 5).

### 1.2 Authoritative vs transient

| State | Class | Evidence |
|---|---|---|
| Layer tree (content, masks, vector masks, effects, blend, opacity, locks, smart filters), channels, guides, metadata, paths, slices, comps, colour mode/depth/profile, quick mask | **authoritative** — serialized to `.pcraft` | `DocM` mirrors `Document` field-for-field (`crates/format/src/manifest.rs:36-105`); written `convert.rs:243`, read back `convert.rs:656` |
| **Selection** | **document state, serialized** — it *is* part of `Document` and *is* saved | `Document.selection` (`doc/src/lib.rs:884-885`); on-disk `DocM.selection: Option<SurfaceM>` (`manifest.rs:52`). Selection changes are history steps but explicitly *not* edits: `DocState::keeps_clean` ("neither marks the file modified", `crates/engine/src/doc_state.rs:125-131`, #6865) |
| History (undo states) | session-only, **not serializable** | `HistoryState { label, doc: Arc<Document>, layers, visibility, id }` — whole-doc snapshots, no serde derives (`crates/ops/src/lib.rs:43-77`), bounded by count+byte budget |
| History-panel snapshots | session-only | `Snapshot` on `DocState.snapshots`, "for the session only … not saved with the document" (`crates/engine/src/cmd/history_cmds.rs:7-14`; `doc_state.rs:50-53`) |
| Active/target/selected layers, layer anchor | **transient session** (`DocState.active_layer`, `selected_layers`, `layer_anchor` — `crates/engine/src/doc_state.rs:4-59`) | not serialized; `document.inspect` reports them (`selectedLayers`, `activeLayer`) |
| Tool state (brush, colours, symmetry) | **transient session** (`ToolState`, `doc_state.rs:146-184`; `DocState.symmetry` "tool state, not document pixels") | not persisted |
| Viewport (zoom, pan, rotation, flip, aspect) | **transient UI** — `ViewXform` at `crates/ui-egui/src/canvas/view_xform.rs:4-18`, per-view in `app.ui.views`; nothing in `Document`/`DocState` stores it | readable/settable only over the control channel (`ui.inspect` / `ui.set {zoom, center, rotation}`) |
| Channel view, isolated layers, collapsed groups, show-only, floating selection, colour samplers | **transient view state** in `DocState` (marked "view state, not history", `doc_state.rs:4-59`; samplers "never saved to .pcraft or PSD") | not persisted |

So the adapter's split is:

- **authoritative**: the `.pcraft` document (or PSD) — layers, pixels, masks,
  effects, channels, guides, metadata, selection included — plus unsaved edits
  and document identity (path, revision, saved_revision).
- **ephemeral (carry for UX, never for correctness)**: active document/tab,
  active/target layer(s), viewport, panel state, tool state. All readable and
  settable over the control channel — exactly what a Carry-On optional object
  needs.

### 1.3 Are unsaved edits serializable?

Three mechanisms exist (`crates/engine`, `crates/format`):

1. **Command journal — replayable, in-memory only.**
   `Session.journal: Vec<(String, Value)>` ("Log of executed commands (for
   action recording and debugging)", `crates/engine/src/lib.rs:195-196`);
   every command with `CommandSpec.journal == true` appends one `(id, params)`
   entry in `after_command` (`crates/engine/src/jobs.rs:933-956`; one command =
   one entry, nested wrapper commands truncated away). The `cmd!` macro
   defaults `journal: true` (`crates/engine/src/cmd.rs:46-50`), so **every
   edit command — paint strokes, filters, layer ops — is journaled**, not just
   menu commands. **No code serializes or persists the journal**; it is read
   only by action recording and tests.
2. **Recorded actions — the same shape, persisted, opt-in.**
   `Action` derives Serialize/Deserialize with `steps: Vec<(String, Value)>` —
   "the `[[id, params], …]` shape `file.automate.batch` and droplets already
   accept" (`crates/engine/src/cmd/actions_cmds.rs:29-51`). `actions.record`
   marks the journal position; recording copies
   `journal.iter().skip(from).filter(replayable)` into the action
   (`actions_cmds.rs:332-356`), where `replayable()` accepts any journaled
   command minus `actions.*` self-edits and shell wrappers (:126-144). Recorded
   params are normalized for portability (sole-selected-layer refs become
   target-relative, `layer.select` by name; layer *names*, not ids, so
   recordings replay on other documents — `docs/control-protocol.md`
   lines 627–635). Playback re-dispatches each step via `Session::execute`
   (`actions_cmds.rs:393`). Actions persist to `actions.json` in the preset
   store (`crates/engine/src/preset_store.rs:71-72`) — but headless sessions
   have no store, so their actions stay in memory (`preset_store.rs:40`).
3. **Autosave / crash recovery — full snapshot, persisted, automatic.**
   `crates/format/src/autosave.rs` writes the *current* `Arc<Document>` snapshot
   (unsaved edits included) as an incremental `.pcraft` directory bundle
   `<recovery_dir>/<key>.pcraft/` plus a `RecoveryInfo` JSON sidecar
   `{key, document_name, original_path, saved_at, revision}`
   (`autosave.rs:27-36`). The UI triggers it every `fileHandling.autosave_minutes`
   (default 10, `crates/engine/src/prefs.rs:521`), **only for dirty documents**
   (`crates/ui-egui/src/prefs/prefs_ui/save.rs:283-289`). Recovery entries are
   discarded only when the document is saved or closed (`autosave.rs:9-10`).

The honest answer for the adapter: unsaved edits can be carried as
(a) an **autosave-style `.pcraft` snapshot of the dirty state** — lossless,
already implemented, coarse (full snapshot semantics); or (b) a **command
journal from the last saved revision** — fine-grained, progressive-transfer
friendly, replayable through `Session::execute`, but PhotoCraft never persists
the journal, so an adapter must capture it (record from session start, or read
`Session.journal` in-process) and own its serialization itself.

### 1.4 Revision, dirty flag, identity

- `DocState.revision` — "Increments on every change; UIs re-render when it
  moves" (`crates/engine/src/doc_state.rs:20-21`); `DocState.saved_revision`.
- `is_dirty() == revision != saved_revision` (`doc_state.rs:122-124`); cleared
  by `saved_to` which records the path and sets `saved_revision = revision`
  (`crates/engine/src/cmd/file_cmds.rs:100-110`).
- Both surface externally: `session.list` → `dirty`, `revision`
  (`crates/engine/src/inspect.rs:10-27`); `document.inspect` → `revision`,
  history entries, `hasSelection`, `selectionBounds` (`inspect.rs:29-68`).
- **The `.pcraft` manifest has no revision field** — only `format_version`
  (schema version, `manifest.rs:18-23`). Document revision is session state;
  the only on-disk revision is the autosave sidecar's `RecoveryInfo.revision`.

### 1.5 The native format (`.pcraft`)

A `.pcraft` file is a zip (store mode) or directory bundle
(`crates/format/src/lib.rs:5-12`): `manifest.json` (versioned document tree),
`tiles/<blake3>.zst` (content-addressed 256×256 tiles), `blobs/<blake3>.zst`
(ICC, EXIF, PSD blocks, smart objects), optional `thumb.png` and
`composite/preview.png`, `save.lock` for directory bundles. The manifest model
(`DocM`, `manifest.rs:36-105`) is "a *separate* serde model from
`photocraft-doc` on purpose: the on-disk schema is versioned and migrated
independently of in-memory refactors" (`manifest.rs:1-4`); `FORMAT_VERSION = 1`
with a migration-steps list (currently empty, `migrate.rs:9-13`).

**Saving is incremental**: `PcraftWriter` remembers what it already
compressed/wrote, so only new tiles are encoded; directory saves re-verify
existing objects once per writer/folder, replace damaged ones and
garbage-collect unreferenced ones under an OS file lock (`lib.rs:14-21`;
`store.rs:430-440`; `SaveStats { tiles_written, tiles_reused, … }`,
`store.rs:336-350`). The engine caches the writer per document across saves
(`DocState.pcraft_cache`, `doc_state.rs:24`).

### 1.6 Engine embedding facts relevant to an adapter

- `photocraft-engine` has **no UI/GPU dependency** and is headless-CPU by
  construction; details and citations in §2.5 below.
- Everything is a command: ~700 commands with stable string ids and
  serde-serializable params (`docs/architecture.md` §4.2); the UI, CLI, control
  channel and MCP all dispatch the same registry (line 18).
- Session snapshots are published after every committed op (`docs/architecture.md`
  §6 line 318), so in-process readers can snapshot consistently.

## 2. Control surfaces

Four externally drivable surfaces exist today, all dispatching the same
command registry. This is what decides that a Carry-On adapter can drive
PhotoCraft **without forking**.

### 2.1 `photocraft-cli` (headless)

`apps/photocraft-cli` — "Headless PhotoCraft: convert, inspect, run commands,
batch, MCP server" (`apps/photocraft-cli/Cargo.toml` description). Eight
subcommands (`apps/photocraft-cli/src/args.rs:30-40`, dispatched
`src/lib.rs:52-61`):

- `convert <IN> <OUT>` — format conversion incl. `.pcraft`/PSD (`args.rs:83-96`)
- `info <FILE>` — document JSON: size, mode, depth, layer tree (`args.rs:99-105`)
- `run` — `--cmd <ID>` (repeatable, in order) with `--params <JSON>` per
  command, `--out <FILE>`, `--new <JSON>` or a positional input file
  (`args.rs:112-138`); prints each command's JSON result
- `batch` / `droplet` — apply recorded action steps (`[id, params]` pairs,
  JSON action files, or Photoshop `.atn` imports) to every image in a directory
  (`args.rs:154-188`)
- `commands` — the registry with params docs (`args.rs:191-199`)
- `mcp`, `serve` — see below

All are headless: every handler goes through
`photocraft_automation::Headless`, which wraps a plain
`photocraft_engine::Session::new()` (`crates/automation/src/headless.rs:15-19,
50-52`); `apps/photocraft-cli/Cargo.toml` has no egui/eframe/wgpu dependency.
`run` uses `Headless::trusted_local()` (host paths, CLI caller is trusted);
`mcp`/`serve` use `Headless::with_workspace(AuthorizedWorkspace)`
(`apps/photocraft-cli/src/automation.rs:37, 78`).

### 2.2 `photocraft-cli serve` (headless JSON-lines server)

`crates/automation/src/rpc.rs` implements a stdio or loopback-TCP JSON-lines
server over one headless engine session (`docs/control-protocol.md` lines
397–426). TCP **refuses non-loopback** (`rpc.rs:349-351`); stdio needs no
token. Method table (`rpc.rs:42-62`): `engine.execute`, `jobs.list`,
`jobs.cancel`, `engine.commands`, `session.list`, `doc.open`, `doc.place`,
`doc.replaceSmartObject`, `doc.new`, `doc.save`, `doc.exportLayers`,
`doc.exportDataSets`, `doc.importDataSets`, `doc.inspect`, `doc.render`,
`doc.select`, `doc.close`, `batch`, `methods`. Doc methods take an optional
`index` targeting a document without activating it. `batch` runs up to 256
steps (`MAX_BATCH_STEPS`, per-request reply budget, nested `batch` refused,
`rpc.rs:217`). File access goes only through launch-time
`--automation-read-root` / `--automation-write-root` capabilities and **fails
closed** without them (`docs/control-protocol.md` lines 375–388). Transport
limits: 1 MiB requests, 8 MiB replies, 30 s socket timeout (lines 445–453).

The dirty flag is machine-checkable: `session.list` replies carry per-document
`"dirty": d.is_dirty(), "revision": d.revision` (implemented in
`crates/engine/src/inspect.rs:10-27`) and `doc.inspect` returns the full layer
tree, `history` entries, snapshots, `canUndo/canRedo`, `revision`,
`activeLayer`, `selectedLayers`, `hasSelection`, `selectionBounds`
(`crates/engine/src/inspect.rs:29-68`). A successful layered save records the
current revision as saved (`docs/control-protocol.md` line 421).

Per-request panics are caught and the session keeps serving (`rpc.rs:254-259`).

### 2.3 Desktop JSON control channel

The desktop app listens on **loopback only** with a 256-bit bearer token
(`docs/control-protocol.md` lines 11–29): `photocraft --control <port>
--control-token-file <path> --automation-read-root … --automation-write-root …`.
Transport: `apps/photocraft/src/control_server.rs` — `127.0.0.1` bind (line 21),
constant-time token compare (`crates/automation/src/security.rs:104-109`),
16 max connections, 1 MiB max request, 30 s socket timeout, 60 s reply deadline
(`crates/ui-egui/src/control.rs:60`). Requests are JSON lines
`{"id","method","params"}` forwarded to the UI thread via channel +
`ctx.request_repaint()`.

Method domains (`crates/ui-egui/src/control.rs:6-35`,
`control/dispatch.rs:12`): reads are `engine.commands`, `ui.inspect`,
`ui.menu.list`, `jobs.list` — and `document.inspect` (layer tree / history /
selection / revision) via `engine.execute {"command":"document.inspect"}`
(`dispatch.rs:30-49`; the MCP bridge uses exactly this,
`crates/automation/src/server.rs:824`). Mutations: `engine.execute` /
`ui.menu.invoke` (any of ~700 commands), `ui.set`, `ui.pointer`, `ui.click`,
`ui.move`, `ui.key`, `ui.type` (synthetic input), `ui.dialog.*`,
`ui.window.*`, `ui.resize`, `ui.screenshot`, `app.open`/`app.save`/`app.quit`.

Commands that use ambient filesystem paths are **refused for automation** and
fail closed ("automation command `…` uses ambient filesystem paths and is
disabled; use capability-scoped document methods", `docs/control-protocol.md`
line 182). The policy is `crates/automation/src/workspace.rs`:
`AuthorizedWorkspace { read: Option<RootCapability>, write: Option<RootCapability> }`
(lines 32-36) — "Without a root, that direction of access is denied"; request
paths are untrusted forward-slash relative paths with `..`, `\`, `:`,
device-prefix and absolute-outside-root rejection *before* any I/O
(`relative_path`, lines 224-265); `authorize_engine_command` (lines 297-345)
deny-lists ambient-path commands; per-request wiring in `control.rs:172-180`
and `dispatch.rs:86-90`; desktop-side plumbing in
`apps/photocraft/src/services.rs:643-699`.

### 2.4 MCP server (`photocraft-automation`, rmcp)

Two modes (`docs/control-protocol.md` lines 326–338): **headless**
(`photocraft-cli mcp`, `Backend::Headless(Arc<Mutex<Headless>>)` — an in-process
engine session, no window) and **bridge** (`Backend::Bridge(Arc<BridgeClient>)`,
`crates/automation/src/server.rs:22-28`) which forwards every tool to a
*running desktop app* over the control channel, so an agent sees and drives the
live editor (open windows, unsaved documents). The bridge client accepts only
loopback addresses (`crates/automation/src/bridge.rs:51-53`) and reconnects
after idle with a 55 s call timeout below the app's 60 s deadline (lines 26, 31).

Tools (`crates/automation/src/server.rs:616-1022`): `session_list`, `doc_open`,
`doc_place`, `doc_replace_smart_object`, `doc_export_layers`,
`doc_export_data_sets`, `doc_import_data_sets`, `doc_new`, `doc_save`,
`doc_export`, `doc_inspect`, `doc_render_preview`, `doc_select`, `doc_close`,
`command_list`, `command_run`, `command_batch` ("not an atomic transaction: no
rollback"), `jobs_list`, `jobs_cancel`; bridge-only: `ui_inspect`,
`ui_screenshot`, `ui_pointer`, `ui_menu_invoke`, `ui_set`, and
**`control_call`** — a raw escape hatch exposing the entire control-channel
method list. `doc_select`/`doc_close` and some `doc_save` options are
headless-only (`bridge_only_headless`, line 1024). So MCP ≈ subset of
(headless RPC surface ∪ control channel), never a superset of either. Budgets
apply everywhere (`crates/automation/src/budgets.rs`:
`MAX_RESPONSE_BYTES = 8 MiB`, `MAX_PNG_BYTES = 5 MiB`,
`MAX_PREVIEW_SIDE = 2048`). Security model: bearer token = full control of the
non-filesystem surface; protocol unencrypted, loopback only, must not be
tunneled (`docs/control-protocol.md` lines 368–373).

### 2.5 Crate-level embedding (in-process)

`photocraft-engine` is directly embeddable: `Session` is a plain
`#[derive(Default)]` struct (`crates/engine/src/lib.rs:184-280`) with
`Session::new()` (`session/documents.rs:17-19`) and
`Session::execute(&mut self, id: &str, params: Value) -> Result<Value>` —
"Run a command by id with JSON params" (`session/execute.rs:16-22`) — plus
`start` (background jobs), `precheck`, document registry access
(`documents()`/`active()`/`set_active`/`close`), `command_specs()`, and
`poll_jobs()`. Every frontend dispatches through the same `Session::execute`
(`lib.rs:1-6`). Embedding safety: `#![forbid(unsafe_code)]` (lib.rs:7); platform
services are injected `Option`s, not globals (`preset_store: Option<PresetStore>`
is `None` headless, lib.rs:235-237; no network I/O in the engine, lines 222-223);
ambient-filesystem reads are command-gated and denied for untrusted sessions
(`crates/automation/src/workspace.rs:297-427`). There is even a purpose-built
authorization hook: `pub authorize: Option<fn(&str, &Value) -> Result<()>>`
(lib.rs:240-244), which `photocraft-automation` installs per session
(`headless.rs:15-19`).

**Engine has no GPU/UI dependency at all**: `crates/engine/Cargo.toml` lists
only doc/raster/compose/algo/paint/text/vector/io/psd/format/plugins/codecs +
rayon/blake3/flate2/craft-print — no wgpu, no egui; `crates/compose` is pure
CPU (rayon). wgpu appears only in `crates/gpu`, `crates/ui-egui`, and the
desktop/web apps, all above the engine layer. (Note:
`docs/architecture.md:185` still claims engine features `gpu`/`cpu-only`; at
this pin `crates/engine/Cargo.toml` defines only a `corpus` test feature — the
engine is CPU-only unconditionally; that doc line is stale/aspirational.)

### 2.6 What this means for a Carry-On adapter

- **Reading state**: document identity (`session.list` → path, dirty,
  revision), layer tree, history, selection bounds (`doc.inspect` /
  `document.inspect`), command enablement (`engine.commands`), and full
  UI/viewport state (`ui.inspect`) — enough to build the authoritative +
  ephemeral objects of an L3 session.
- **Mutating state**: any engine command by id (`engine.execute` /
  `Session::execute`); unsaved edits can be applied headlessly (e.g.
  `paint.stroke`, `layer.setProps`, filters) or driven through the live UI.
- **Bytes**: `doc.save`/`app.save` produce the document bytes under a
  capability-scoped root; `doc.render`/`ui.screenshot` produce PNG previews.
- **No fork needed**: every operation a Carry-On adapter needs (open, inspect,
  apply, save, quit, and even viewport/selection restore) has an external
  surface. Three drive modes, all without forking: (1) `photocraft-cli serve`
  stdio/TCP with capability roots; (2) `photocraft-cli mcp` headless or bridge;
  (3) crate dependency (`photocraft_automation::Headless` or raw
  `photocraft_engine::Session`) with the same fail-closed policy in-process.
  The desktop control channel is loopback-only per process — an external
  adapter process must live on the same machine as the app, or the adapter
  embeds the engine in-process instead.

## 3. Continuation semantics

To continue editing a PhotoCraft document on another machine:

| Role | Object | Why |
|---|---|---|
| **prerequisite** | saved document bytes — `.pcraft` preferred (lossless native, versioned manifest, content-addressed incremental bundle, §1.5); PSD as interchange | source of truth; without it there is nothing to edit |
| **prerequisite** | unsaved edits, one of: (a) a `.pcraft` snapshot of the dirty state (autosave-style; lossless, already implemented by `autosave.rs`), or (b) a command journal `[[id, params], …]` from the last saved revision (replayable via `Session::execute`, §1.3) | losing these is data loss |
| **prerequisite** | document meta: original path/name, `revision` + `saved_revision` (identity + provenance; decides whether a journal replays against the right base) | `session.list`/`document.inspect` expose both (`inspect.rs:10-68`) |
| **optional (immediate, latency-only)** | active document/tab index (`ui.menu.invoke {"id":"window.document.<index>"}`), target/selected layers (`layer.select` by name+occurrence), selection (restorable by replay or carried as document state — note selection *is* part of the document bytes, §1.2), viewport `zoom/center/rotation` (`ui.set`), panel/dock layout, tool | restores the *working position*; all readable/settable via `ui.inspect`/`ui.set` (`docs/control-protocol.md` lines 49–52) |
| **not carried** | preferences (`preferences.json`, `ui.ron`), brush presets/preset store, keyboard shortcuts | machine-local by design (`docs/control-protocol.md` lines 264–288); presets *can* be carried later as optional provenance objects if wanted |

Resolved answers (were TBD-CODE):

1. **Is history serializable?** No — `HistoryState` holds `Arc<Document>`
   snapshots with no serde derives (`crates/ops/src/lib.rs:43-77`); history is
   in-memory snapshots, not a journal, and cannot be carried.
2. **Is selection saved in `.pcraft`?** Yes — `DocM.selection`
   (`manifest.rs:52`); it is document state and survives save/open.
3. **Can the journal be captured retroactively?** In-process, yes:
   `Session.journal` accumulates since session start, so an embedded adapter
   can read it and slice from any point. Externally, only if a recording was
   running (`actions.record` + `actions.stop` copies the replayable tail,
   `actions_cmds.rs:332-356`) — there is no control-channel method to dump the
   raw journal. This asymmetry pushes the design toward in-process capture
   (§`ADAPTER-DESIGN.md`).
4. **Revision semantics**: `DocState.revision` increments on every change;
   `saved_revision` tracks the last save; `dirty == revision != saved_revision`
   (`doc_state.rs:20-24, 122-124`). Not in the file (§1.4) — so carried meta
   must preserve it.

## 4. Risks (early-alpha reality)

- **API churn**: 1,004 merged PRs to date; dozens of merges per day at pin
  (`git log` around `1f7fc0f`). Command ids are the stable seam (used by menus,
  shortcuts, actions, CLI, MCP — `docs/architecture.md` §4.2), so an adapter
  keyed on command ids + the documented control methods ages better than one
  keyed on Rust types.
- **Alpha status**: 953 open issues; PSD writes not yet reliably accepted by
  Photoshop (`ROADMAP.md` "Why alpha"); ~1,100–1,800 h to beta (same doc).
- **Blast-radius controls exist**: transport limits, fail-closed roots, strict
  param checking (`docs/control-protocol.md` lines 445–486) — good for adapter
  hygiene, but also *ceilings an adapter must respect* (8 MiB replies: document
  bytes travel by file, not by reply).
