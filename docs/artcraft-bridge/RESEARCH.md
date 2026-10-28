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
the model: a `Document` with size, colour mode, bit depth, ICC profile, a layer
tree (`LayerGroup` root), saved alpha channels, paths, guides, slices, metadata,
and — notably — the selection (`docs/architecture.md` §5 sketch, lines 277–301).
Layer content is an enum from day one: `Raster(TiledSurface)` | `Group` |
`Adjustment` | `Fill` | `Text` | `Shape` | `Smart` (§5, lines 292–300) —
non-destructive by construction. Pixels live in 256×256 copy-on-write,
`Arc`-shared tiles (`raster::TiledSurface`, §5 line 303); every large payload in
the document model is `Arc`-shared so snapshots are cheap (§1 principle 5).

TBD-CODE: exact `Document`/`Layer` struct fields, selection location,
snapshot/revision types (see §1.4 below — filled from `crates/doc` source).

### 1.2 Authoritative vs transient

| State | Class | Evidence |
|---|---|---|
| Layer tree (content, masks, vector masks, effects, blend, opacity, locks), channels, paths, guides, metadata, colour mode/depth/profile | **authoritative** | persisted by `.pcraft` save (§9) and PSD round-trip; `docs/architecture.md` §5 |
| History (undo states) | authoritative-ish, in-memory | each mutation is an `Op` transaction; history stores transactions; snapshots are `Arc`-shared (§6). TBD-CODE: serializability of history states |
| Selection (raster mask + optional vector outline) | **document state** (travels with the doc, Photoshop-style), but ephemeral in carry terms — TBD-CODE whether `.pcraft` saves it | `docs/architecture.md` §5 line 284 (`selection: Option<Mask>` on Document); control `document.inspect` reports selection bounds |
| Tool state (active tool, brush, colours, options bar) | **transient session/UI** | `ui.inspect`/`ui.set` surface, not a document object; preferences persist some of it |
| Viewport (zoom, pan, rotation), panels, dialogs, windows | **transient UI** | `ui.inspect` / `ui.set {zoom, center, rotation…}` (`docs/control-protocol.md` methods, lines 49–50) |
| Active/target layer, expanded groups | view state; some is saved with the document (PSD open/closed folders — `layer.setExpanded`, control-protocol.md line 130) | mixed |

The transient/authoritative split for the adapter is therefore:

- **authoritative**: the `.pcraft` document (or PSD) + unsaved edits + document
  identity (revision, path, format version).
- **ephemeral (carry for UX, never for correctness)**: active document + tab
  index, active/target layer selection, selection mask, viewport
  (zoom/center/rotation), panel state, tool state. All of it is *readable and
  settable over the control channel* (`ui.inspect`, `ui.set`), which is exactly
  what a Carry-On optional object needs.

### 1.3 Are unsaved edits serializable?

Three candidate serializations exist, TBD-CODE for details:

1. **Command journal / recorded actions**: actions record replayable
   `[commandId, params]` steps (`actions.list`/`actions.get`/`actions.record`,
   `docs/control-protocol.md` lines 146–154; the shape `file.automate.batch`
   and droplets take). Recorded steps omit queries and action-editing commands
   (line 149), and newly recorded steps store layer *names* (+ occurrence), not
   document-specific ids, explicitly so a recording replays on other documents
   (lines 627–635). The batch method (`docs/control-protocol.md` line 425)
   replays `[{command, params}]` lists headlessly.
2. **Autosave / crash recovery**: the engine autosaves to a `Recovery` folder
   (`docs/control-protocol.md` lines 264–270; `docs/architecture.md` §9 line
   417) reusing the `.pcraft` writer from a background snapshot.
3. **`.pcraft` save** (`app.save` / `doc.save`): writes the full document
   (layers + pixels) to the automation write root.

So the honest answer shaping the adapter: **yes, with granularity choices** —
the unsaved state can be carried either as (a) an autosaved/saved `.pcraft`
snapshot (coarse but lossless and already implemented), or (b) a replayable
command journal from a known revision (fine-grained, progressive-transfer
friendly, but requires pinning the journal semantics). TBD-CODE: which is
exposed externally, and whether history states themselves serialize.

### 1.4 Engine embedding facts relevant to an adapter

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
| **prerequisite** | saved document bytes (`.pcraft` preferred — lossless native; PSD interchange, `docs/architecture.md` §8–9) | source of truth; without it there is nothing to edit |
| **prerequisite** | unsaved edits — either (a) an autosave/snapshot `.pcraft` of the dirty state, or (b) a command journal from the last saved revision (TBD-CODE which the adapter can actually obtain externally) | losing these is data loss |
| **prerequisite** | document meta: path/name, format, revision, dirty flag | identity + provenance; decides whether journal replay is valid |
| **optional (immediate, latency-only)** | active document/tab, target layer(s), selection, viewport (zoom/center/rotation), panel layout, tool | restores the *working position*; readable/settable via `ui.inspect`/`ui.set`/`ui.menu.invoke` (`window.document.<index>`, line 52) |
| **not carried** | preferences (`preferences.json`, `ui.ron`) | machine-local by design; portable mode exists but preferences are not session state |

TBD-CODE rows to firm up after source review: (1) is history serializable;
(2) is selection saved in `.pcraft`; (3) can the journal be captured
retroactively (from history) or only via explicit `actions.record`;
(4) revision counter semantics.

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
