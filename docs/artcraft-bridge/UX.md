# PhotoCraft desktop flagship UX (stage 10)

What the Carry-On macOS app does with a **running** PhotoCraft: sees the live
session, cuts it through `carryon-adapter-photocraft`, streams the cut in visible
per-tile budget windows, and re-opens it in a second live PhotoCraft. No fork, no
patched PhotoCraft, no injected code.

The pane is **PhotoCraft** (sidebar `⌘3`): *Send* and *Receive* roles.

## Channel: the JSON control channel, not MCP

PhotoCraft exposes two front doors to the same engine commands: its desktop
**JSON-lines control channel** (loopback, bearer-token file) and an **MCP
server** (same commands, agent-oriented). We use the control channel because:

- it is the surface this adapter already pins and tests end-to-end (Phase 2/3
  evidence: control driver, live L3 restore, physical L4 carry);
- its security model is the one we can state exactly — loopback bind, 0600 token
  file, constant-time compare, fixed method whitelist in the driver
  (`ALLOWED_METHODS`), no ambient paths (the app's own automation roots);
- MCP would add a second protocol + a second security surface for the same
  verbs. If an MCP client is ever shipped, it must reuse the same driver seam
  (`PhotocraftDriver`) and the same whitelist discipline.

The bearer token is read **inside Rust** from the token file and never returned,
logged, or placed in an evidence bundle (EVD-006). The UI only knows the file
path.

## Source side — "Cut & send session"

1. **Inspect** (`carryon_photocraft_inspect`): one `session.inspect` + one
   `ui.inspect` → the live card (document, revision, dirty, tool, zoom,
   rotation, selected layers, document count). Writes nothing.
2. **Capture** (`carryon_photocraft_register`):
   - the saved base document bytes (the app's own `.pcraft` ZIP on this machine);
   - the dirty-state snapshot, only when dirty, produced **non-destructively**
     (`image.duplicate` → `app.save` scratch → read → `file.close`; the original
     is never saved or cleaned);
   - the ephemeral working position and the meta;
   - `split` encoding: the saved ZIP is split into PhotoCraft's own
     content-addressed tiles/blobs. PhotoCraft's ZIP entries are **stored**
     (method 0) and carry the same `manifest.json` + `tiles/<blake3>.zst` +
     `blobs/<blake3>.zst` the directory bundle does, so the split is a pure
     re-framing of bytes the app wrote — previews are dropped (the destination
     re-renders them). See `crates/carryon-adapter-photocraft/src/zipbundle.rs`.
3. The cut is sealed by the engine (`create_cut`); the app serves it over TLS
   1.3 with pin-based mutual auth. Every budget window is a fresh connection
   (the engine's progressive-import discipline); the receiver's final *done*
   hop stops the loop.

## Destination side — "Receive & restore"

1. The source's Welcome features carry a data-only descriptor (session id, cut,
   object count, name, dirty flag, the ephemeral workspace JSON) — the same
   `Hello`-feature pattern the file handoff uses (§3.8: data, never behavior).
2. **Import in windows**: each `import_cut` runs under a budget of
   `window_bytes`. A partial admission publishes a verified prefix (dedup makes
   the next window skip it); `progress` in the UI is
   `published / total` objects + wire bytes. An object larger than the window is
   refused **fail-closed** by the engine (§6.8's rule) — the app widens the
   window and retries, showing the widening. That is how the ~4.16 MB dirty
   snapshot lands after the 48 tiles.
3. **Restore** (`carryon_photocraft_restore`): re-materialize the carried state
   from the destination's own store by the sealed cut's digests, write it under
   the app's automation read root (a directory bundle when the document is
   clean, the `.pcraft` snapshot when dirty), `app.open` it, and reapply the
   working position (tool, zoom/center/rotation, selected layers) to the
   now-active document. It then runs the adapter's `session.restore` so the
   engine's **oracle** re-derives the authoritative-state hash from the imported
   state — a restore that merely opened a file does not pass.
4. A final *done* hop stops the sender.

First paint: the document becomes usable when its closure completes (the engine
never seals a partial cut). The UI shows the tile/object stream before that, and
the reopened document after the restore — the honest version of "progressive
first paint" for this engine.

## What the run proves, and what it does not

Evidence: `docs/evidence/20261108-photocraft-ux-local/` (two live PhotoCraft
apps + two Carry-On cores on **one** machine, loopback).

**OBSERVED (LOCAL)**:
- 51 sealed objects (48 tiles + manifest + meta + dirty snapshot);
- 17 budget windows at 256 KiB, widening 4× for the snapshot; oracle AGREED;
  tool/zoom/selected-layers match after the live restore;
- app A's window screenshotted by PhotoCraft's own `ui.screenshot` after restore.

**NOT claimed**:
- **PHYSICAL** — no second Mac, no second NIC. The two-instance-on-one-Mac
  equivalence is stated in the evidence, not hidden.
- §30 / platform "supported" — untouched.
- L4 authority transfer in this UX run: the adapter's physical L4 proof exists
  from Phase 4 (`docs/artcraft-bridge/evidence/phys-l4`); the stage-10 pane
  carries + restores, it does not move the single-writer epoch yet.
- MCP: not exercised.
- The ephemeral workspace (viewport/tool/selection) travels in the descriptor,
  not as a sealed cut object (the engine's wire manifest is authoritative-only;
  the workspace object is Ephemeral/optional by design). It is integrity-checked
  data but not part of the authority hash.

## Running it locally

```sh
# two live apps + the engine-level run + the app-kit run (headless)
docs/artcraft-bridge/scripts/photocraft_ux_local.sh
#   CLEAN=1 …  keeps the source document saved (tests the reconstructed bundle
#              directory path instead of the dirty-snapshot path)

# or in the app: PhotoCraft pane (⌘3), fill the control address/token/roots
# from the app's --control/--automation-* flags, Inspect, then Send/Receive.
```

The script pins PhotoCraft `1f7fc0f5…` (`PHOTOCRAFT-PIN.md`) and writes the
evidence JSON + screenshots into `docs/evidence/20261108-photocraft-ux-local/`.
