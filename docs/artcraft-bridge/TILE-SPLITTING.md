# Tile-splitting — PhotoCraft documents as manifest + content-addressed tiles

> Phase 4, task 1. Design doc, written **before** implementation (owner-approved
> option B, ADAPTER-DESIGN §7.4/§7.6). Status labels follow
> [`../TARGETS.md`](../TARGETS.md): this design is **TARGET**; each section names
> the label its implementation must earn. PhotoCraft facts cite
> [`RESEARCH.md`](RESEARCH.md) (pin `1f7fc0f5…`).
>
> **Spec check done against [`CROSS-PLATFORM-ENGINE-SPEC.md`](../../CROSS-PLATFORM-ENGINE-SPEC.md).**
> §6.3 Objects / §6.4 kinds / §6.6 dependency closure / §6.8 budgets /
> §18.6 transfer / CORE-004 two-phase publish. One core **behavior change** is
> called out in §5 (budget admission becomes cumulative/progressive) — it is a
> strengthening of §6.8 enforcement, not a spec rewrite; no spec text is
> contradicted. No schema migration is needed (CORE-007 untouched).

## 1. Problem

The core caps one object at 16 MiB (`CORE_MAX_OBJECT_BYTES`,
`crates/carryon-core/src/host/registry.rs`; `validate_entry` rejects larger —
ADAPTER-DESIGN §7.4). A monolithic `.pcraft` zip fits today's real photo
(6.9–7.4 MiB) but a 45 MP camera layer extrapolates to 40–90 MB: **over the cap,
uncarriable at all**. And even under the cap, a monolith interacts badly with
§6.8 budgets: an import attempt whose budget is below the object size moves
**zero durable bytes**, no matter how many windows it gets.

PhotoCraft's native format is already content-addressed at the tile level
(RESEARCH §1.5): a bundle is `manifest.json` + `tiles/<blake3>.zst`
(256×256 tiles) + `blobs/<blake3>.zst` (ICC/EXIF/PSD blocks), as a zip **or a
directory** with identical layout, and saving is incremental (only new tiles are
encoded). Splitting the carry at exactly these seams is therefore not an
invention — it reuses PhotoCraft's own object boundary.

## 2. Object model (adapter level; the core's store/object model is untouched)

`photocraft.document.v1` (one zip object) becomes, in **split mode**:

| object id | kind | parents | content |
|---|---|---|---|
| `photocraft.document.manifest.v1` | Authoritative | — (root) | the bundle's `manifest.json` bytes — the versioned document tree (RESEARCH §1.5) |
| `photocraft.tile.<blake3hex>.v1` | Authoritative | `document.manifest@gen` | one `tiles/<blake3>.zst` entry's exact bytes |
| `photocraft.blob.<blake3hex>.v1` | Authoritative | `document.manifest@gen` | one `blobs/<blake3>.zst` entry's exact bytes |

Design points, each checked against the spec:

- **Object id = PhotoCraft's BLAKE3 content name** (the tile's *own* content
  address, stable across generations and machines), embedded in the object id —
  a stable logical identifier within the adapter namespace (§6.3). The core's
  `content_hash` stays SHA-256 of the exact served bytes (§6.3: digest of exact
  uncompressed logical bytes — the `.zst` entry bytes ARE the logical object
  bytes we serve). Two digests, two roles: BLAKE3 names (PhotoCraft's, inside
  the id and the manifest's references), SHA-256 verifies (Carry-On's, in the
  store). No path is ever named (digests, never executable paths — golden rule).
  **Discovered while implementing** (and pinned by the round-trip test): the
  manifest references parts by **bare 64-hex BLAKE3 hash**, never by path —
  the `tiles/…`/`blobs/…` path is derived by PhotoCraft's reader.
- **Parent linkage** (§6.3 "exact object-version inputs required for
  reconstruction or validity"): each tile/blob's `parents` = the manifest object
  version it belongs to. The document's reconstruction closure (§6.6) is
  manifest + every part the manifest references; `resolve_action` declares all
  of them as **execution prerequisites** for `document.edit` / `session.restore`.
- **Kinds** (§6.4): tiles are **Authoritative** — user pixels, not regenerable
  from the manifest (so not Derived), too large to be Preview, not disposable
  Cache. Previews (`thumb.png`, `composite/preview.png`) are caller-rendered
  (RESEARCH §1.5, format crate "does not render") and are **not carried** — the
  destination re-renders them on open. Honest loss, documented: no thumbnail
  arrives with the cut; document content is unaffected.
- **The 16 MiB cap is unchanged** and now non-binding: a 256×256 RGBA tile
  compresses to KBs, orders below the cap; the manifest.json of a 45 MP layered
  document is still KBs–MBs of JSON. The monolithic `photocraft.document.v1`
  object remains supported (the control-channel driver's mode; back-compat for
  every Phase 1–3 test and harness) — the adapter carries an explicit
  `document_encoding` in the meta object: `monolithic` (default) | `split`.
- **Unsaved edits**: journal mode stays ONE small object
  (`photocraft.unsaved_edits.v1`, parents = `document.manifest@gen`). Snapshot
  mode, when the dirty-state bundle exceeds the cap, splits the same way with
  its own manifest object `photocraft.unsaved.manifest.v1`. Tile object ids are
  shared between base document and dirty snapshot when content is shared
  (PhotoCraft tiles are copy-on-write, RESEARCH §1): the adapter deduplicates
  ids when declaring the session, so the carry moves each distinct tile **once**.
- **Authority binding (§21.2) is unchanged in formula**: `auth_hash` =
  SHA256(len(document-logical-bytes) ‖ document-logical-bytes ‖ len(unsaved) ‖
  unsaved ‖ meta). In split mode the document's logical bytes are the
  **canonical bundle serialization**: len-prefixed `manifest.json` bytes, then
  every part in sorted-path order (len-prefixed path ‖ len-prefixed bytes) —
  deterministic, byte-stable across machines (no zip metadata). All parts, not
  a manifest-referenced subset: PhotoCraft's loader (not this adapter) is the
  authority on which hashes it will fetch, so the hash binds everything the
  bundle carries. The destination re-derives it from the state it imported;
  authority stays bound to state, never to a token (§3.8).

## 3. Reconstruction and the oracle

Destination reconstruction is **fail closed**:

1. write `manifest.json` + every carried part into a scratch directory bundle
   under the granted work root (`write_bundle_under_root`);
2. open it (`photocraft_automation::files::open` dispatches directory bundles —
   RESEARCH §1.5; verified against the pin);
3. **the closure check is PhotoCraft's own loader**: the manifest names parts
   by bare BLAKE3 hash and the loader fetches each hash it needs, refusing on
   the first missing one (`corrupt .pcraft bundle: missing tiles/<hash>.zst` —
   the exact error the round-trip test pins with a holed bundle). An
   incomplete carry therefore **cannot open as a document**; an extra
   unreferenced part is ignored, exactly as PhotoCraft ignores unreferenced
   bundle entries on open (the bundle GC only runs on save).

The adapter deliberately does not re-implement the reference walk: the manifest
model is versioned and migrates independently of the adapter (RESEARCH §1.5),
and any adapter-side subset would be a second, weaker copy of the loader's own
fail-closed check.

The oracle is unchanged and stronger than byte equality: the reconstructed
document's **flattened composite pixels** must hash equal to the source's
(the Phase-2/3 oracle — `.pcraft` bytes are not comparable across sessions
because layer-id allocation differs; RESEARCH §1.3.2).

## 4. Invariants (law)

1. **Two-phase publish (CORE-004) for every tile**: the core already publishes
   each declared object through stage → whole-object digest verify → atomic
   publish (`crates/carryon-core/src/store/publish.rs`); a cut seals only after
   all its authoritative objects are published+verified. Tile-splitting
   multiplies the number of objects, not the discipline. No change.
2. **Fail closed**: reconstruction requires every referenced part; an incomplete
   bundle never activates, never seals a mirror cut, never runs an action.
3. **Digests, never paths**: object ids embed BLAKE3 names; no store path
   crosses the adapter boundary.
4. **Schema**: no new tables (published objects are already content-addressed,
   resumable, and deduplicated by digest). **No migration** (CORE-007).
5. **FFI**: append-only within ABI major 1. The import result JSON gains an
   additive `partial_budget` shape (§5); no signature changes;
   `CARRYON_ABI_MINOR` bumps 0 → 1.

## 5. Budget admission — cumulative and progressive (the one core change)

§6.8 gives preparation *explicit* budgets; "budget rejection is a normal
outcome". Today's destination admission
(`admit_import_budget`, `crates/carryon-core/src/transfer.rs`) checks **each
object individually against the whole `total_net_bytes`**: a closure whose
objects *sum* above the budget admits entirely once objects multiply — a hole
the tile split would fall straight into (60 tiles × 120 KB each vs a 1 MiB
budget would move 7 MB "within budget").

**Change (called out per the phase rules):** admission becomes
**cumulative and progressive** — not-yet-present objects are counted in
manifest order against a running spend; the **longest prefix that fits** the
budget is admitted; the rest is deferred:

- prefix objects pull through the unchanged CORE-004 path — every pulled tile is
  verified and durably published; progress **persists at object granularity**;
- the mirror cut is **not** sealed and the session stays `Importing` — no
  action can run until a later attempt completes the closure (fail closed);
- deferral is journaled (`BUDGET_Rejected` with the deferred list and bytes,
  §6.8 visibility) and reported as a new `ImportOutcome::PartialBudget { … }`;
  on the wire the destination closes with `Abort { reason: "BUDGET_PARTIAL …" }`
  (no new wire message; additive shapes only);
- a re-invocation of `import_cut` (budget replenished — the mobile-background
  story) re-admits against the *same* store: already-present objects are
  skipped (dedup fast path) and only the remaining prefix moves. This is where
  the split pays: with a monolith, a budget below the object size makes zero
  durable progress in every window; with tiles, every window banks verified
  tiles and the next window continues.

This strengthens §6.8 enforcement (the budget now bounds the whole attempt) and
uses §6.6's sanctioned "deferral until demand". Source-side sealing is
untouched: a cut still requires its complete authoritative closure. Existing
behavior for "the first object alone exceeds the budget" is unchanged (typed
`BUDGET` error before any bytes).

## 6. Measurements (raw numbers, honest labels)

**OBSERVED, LOCAL** (one machine, loopback TLS, in-process PhotoCraft engine
at pin `1f7fc0f5…`), harness `examples/tile_progressive.rs`, evidence
[`evidence/tile-progressive.json`](evidence/tile-progressive.json), real photo
(2400×1500, photo + adjustment + retouch layers, 1 dirty stroke, journal mode):

1. **bfa, split vs monolithic**: split **5,930,649 B** = manifest 15,376 +
   60 tiles + 1 blob (5,914,792) + journal 103 + meta 378; monolithic
   **6,868,017 B** = zip 6,867,536 + journal + meta. The split total is
   *lower* because the monolithic zip embeds PhotoCraft's `thumb.png` +
   `composite/preview.png` previews, which the split bundle deliberately does
   not carry (re-rendered on open) — the delta is previews, not pixels.
2. **Progressive windows**: 64 KiB window → 7 objects pulled + verified +
   durably published (meta, journal, manifest, 1 blob, 3 tiles; +65,108 B
   logical / 120,784 B wire), 57 objects deferred (5,865,541 B); 256 KiB
   window → 3 more tiles (+260,747 B), tail shrinks 57→54; replenished window
   → completes. **Monolithic counterfactual** at the same windows: 64 KiB
   banks only the 2 tiny objects and defers the *entire* 6.87 MB document; a
   256 KiB retry (only the document left) is refused before any bytes
   (`BUDGET_Network`). No sequence of sub-6.87-MB windows can ever complete
   the monolithic carry; the split converges in windows.
3. **Tile dedup on re-carry (OBSERVED wire bytes)**: after two strokes and an
   incremental re-save, 6 of 67 content objects changed (122,041 B); a
   destination that already held cut 1 imported cut 2 moving **222,413 B wire
   in 0.28 s** — manifest + 6 changed tiles + journal + meta, the untouched
   61 tiles deduplicated by the content-addressed store.
4. **Oracle**: the destination reconstructed the bundle from **its own store by
   digest**, opened it with the real engine, replayed the journal — composite
   pixels sha256 identical source↔destination
   (`b5bdb593d26676f01cac37288efae39c29e0028fe310c0b20583a73b6a2b05ec` both
   sides).

All LOCAL until a physical run promotes them; the split path is what the
Phase-4 physical L4 harness (task 2) uses.

## 7. What is explicitly NOT here

- No core cap change (option A stays rejected/queued with the orchestrator).
- No bulk-object-kind or partial-fetch core extension — if kiwix-bridge needs
  those, that is an orchestrator decision (coordination rule).
- No live-GUI restore of split documents via the control channel in this phase
  (the desktop app *can* save into a directory bundle, but the control driver
  keeps monolithic mode; noted as a follow-up).
