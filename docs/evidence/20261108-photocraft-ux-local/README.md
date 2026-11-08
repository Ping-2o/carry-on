# Stage-10 PhotoCraft desktop flagship UX — LOCAL evidence

**Label: OBSERVED (LOCAL).** Two live PhotoCraft desktop apps and two Carry-On
cores on **one Mac**, loopback only. There is no second Mac in this run, so
nothing here is PHYSICAL and nothing upgrades a platform to "supported" (§30
unrun). The two-instance-on-one-Mac equivalence of the "second Mac" story is the
honest substitute, stated wherever the result is cited.

- PhotoCraft pin: `1f7fc0f53696e54da79b95015abb37d7c22cdd1a` (0.16.0 dev build)
- Channel: PhotoCraft's loopback JSON control channel (not MCP)
- Integration: fork-free; the adapter is compiled in; the app is driven only
  through its own automation roots
- Script: `docs/artcraft-bridge/scripts/photocraft_ux_local.sh`
- UX/architecture: `docs/artcraft-bridge/UX.md`

## What was run

1. Two PhotoCraft instances started with their own control ports, token files
   and automation read/write roots.
2. App A opened a generated 2048×1536 photo, painted three strokes, saved it as
   a native `photo.pcraft`, then stayed **dirty** (one more stroke) to exercise
   the snapshot object.
3. **Engine path** (`crates/carryon-adapter-photocraft/examples/photocraft_ux_live.rs`):
   capture → split cut → 256 KiB budget windows → restore into live app B →
   engine oracle + independent position check. → `run.json`
4. **App path** (`apps/macos` `carryon-photocraft-smoke`, the same
   `CarryOnKit` send/receive code the PhotoCraft pane runs) → `run-app.json`.

## Results

| | engine path (`run.json`) | app path (`run-app.json`) |
|---|---|---|
| encoding | split (PhotoCraft tiles) | split |
| sealed objects | 51 | 51 |
| parts (tiles/blobs) | 48 | 48 |
| budget window | 262144 B | 262144 B |
| windows (incl. 4 refused+widened) | 22 connections / 17 progress windows | 17 windows, 4 widenings |
| wire bytes | 11,587,342 sent / 11,482,859 received | 11,482,859 received |
| transfer | 6.58 s (engine run) | 2.58 s (app core run) |
| restore | 1.23 s | into live app B |
| oracle | checked, **AGREED** ("auth_hash matches the sealed state") | checked, **AGREED** |
| position after restore | tool ✓ zoom ✓ selected layers ✓ | `Brush`, zoom 1.35, 1 layer ✓ |
| screenshots | `ux-after-restore.png` (app B's own window via `ui.screenshot`) | — |

Progressive note: the 48 tiles stream ~3 per 256 KiB window; the ~4.16 MB dirty
snapshot cannot fit a 256 KiB window, so the engine refuses fail-closed and the
app widens the window 4× (256 KiB → 4 MiB) for that object. Both the refusals
and the widening are visible in `windows[]` / `run-app.json`.

The restored document's internal name is `photo.pcraft_copy.pcraft`: PhotoCraft's
`image.duplicate` names the duplicate "…copy", and the carried dirty snapshot is
exactly that non-destructive duplicate (the original keeps its dirty flag and
path). The displayed carry name is `photo.pcraft`.

## Clean-document variant (`clean-document/`)

The same script with `CLEAN=1` (the document stays **saved** — 0 B unsaved):

| | engine path | app path |
|---|---|---|
| sealed objects / parts | 51 / 48 (unsaved object is present and empty) | 51 |
| wire bytes received | 5,892,888 | 5,892,888 |
| windows | 17 connections | 16 |
| opened in app B | `carryon-ingress/<tag>/photo.pcraft/` — a **reconstructed tile bundle directory** | same |
| display name | `photo.pcraft` (no `_copy` suffix) | `photo.pcraft` |
| oracle | AGREED | AGREED |

This is the path the dirty run cannot exercise: a clean carry opens the
rebuilt directory bundle (`manifest.json` + `tiles/` + `blobs/`), which
PhotoCraft reads natively; the dirty carry opens the exact-state `.pcraft`
snapshot instead.

## Files

- `run.json` — engine-level run (windows, bytes, capture, oracle, position match,
  PhotoCraft pin, label).
- `run-app.json` — the CarryOnKit send/receive run (the app's own code path).
- `ux-after-restore.png` — app B after the restore, captured by PhotoCraft's own
  `ui.screenshot` (the app's real window; no OS-level screenshot tooling),
  downscaled to 1400 px for the repository.
- `app-a.log`, `app-b.log` — the two apps' logs. They contain document names and
  paths only; the bearer tokens are never written to them (EVD-006).

## Limitations (explicit)

- **LOCAL only** — one machine; a second Mac remains untested for this UX.
- L4 authority transfer is not part of this UX run (the adapter's PHYSICAL L4
  proof is Phase 4, `docs/artcraft-bridge/evidence/phys-l4`).
- The ephemeral workspace (viewport/tool/selection) travels in the Hello
  descriptor (data-only), not as a sealed cut object — the engine's wire manifest
  is authoritative-only and the workspace object is Ephemeral/optional by design.
- MCP was not exercised.
- The macOS pane is built and unit-tested (`swift test`), and the app-path run
  above executes the same `CarryOnKit` send/receive code headlessly; the pane's
  interactive visual run is not automated.
- No platform is "supported"; wording stays **prototype / platform pilot**.
