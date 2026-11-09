# Large sessions beyond the 16 MiB core object cap

**Status: Proposed — needs owner decision** (issue #27). Label: `TARGET`. No code
changes come with this record; nothing here upgrades a label (spec §2).

## Context

- The core caps any single object at **16 MiB**
  (`CORE_MAX_OBJECT_BYTES`, `crates/carryon-core/src/host/registry.rs`). An adapter
  manifest may declare a smaller `max_object_bytes`, never a larger one.
- **Owner decision 2026-10-28 → option B (per-tile splitting)**
  (`docs/FRONTIER.md` log). PhotoCraft documents travel as one manifest object plus
  BLAKE3-named, parent-linked, content-addressed tile and blob objects
  ([`artcraft-bridge/TILE-SPLITTING.md`](artcraft-bridge/TILE-SPLITTING.md)). The same work made
  destination budget admission **cumulative and progressive**
  (`ImportOutcome::PartialBudget`): the longest prefix that fits is pulled through
  the unchanged CORE-004 path, the rest is deferred, and the mirror cut stays
  unsealed until the closure is complete. Measured `OBSERVED (LOCAL)`: 64 KiB /
  256 KiB windows bank verified tiles; a re-carry after two strokes moved 222,413 B
  for a 6.87 MB document.
- **Kiwix** never puts the ZIM in a cut: the bulk is an unsealed Cache object and
  only the reading position is authoritative. Engine A-mode for a 27 MB+ ZIM is
  impossible under the cap ([`kiwix-bridge/PHASE3-EVIDENCE.md`](kiwix-bridge/PHASE3-EVIDENCE.md)).
- The spec already expects payloads to move as bounded, independently verifiable
  chunks (§3 principle 9, §18 default chunk ≈ 1 MiB, §25.2, NET-004/NET-006) and
  lists "fixed vs content-defined chunking for large mutable files" as open design
  decision 4 (§32).

## Question

Do we need first-class large objects for sessions that do **not** tile naturally
(a single large blob with no internal content-addressed seams), and if so which
shape?

## Options

| | Option | How it works | CORE-004 | Budgets (§6.8) | Cost |
|---|---|---|---|---|---|
| B | **Adapter-level splitting (status quo)** | each adapter splits at its format's own seams (tiles, blobs, entries) | unchanged: every part is staged, digest-verified and published on its own | progressive admission already banks parts | per adapter; impossible for formats with no seams |
| C | **Chunked objects in the core** | a logical object = an ordered chunk list (each chunk ≤ cap, own digest) + a whole-object digest | chunks stage and verify individually; the **object** becomes visible only after every chunk is present and the whole-object digest matches | admission can bank verified chunks across windows, the same way it banks tiles today | new chunk-list record (likely a schema migration, CORE-007), additive ABI fields, a new fault suite |
| S | **Streamed objects** | one object > cap, streamed to staging with incremental hashing, published at the end | unchanged shape (stage → verify → publish) | no progress granularity: a window smaller than the object banks nothing, same as a monolith | streaming paths in store + transport, disk-space admission |
| A | **Raised cap + budget admission** | raise `CORE_MAX_OBJECT_BYTES` (the spec's example manifest uses 256 MiB) and refuse anything that does not fit the budget and free space before transfer | unchanged | the monolith problem returns: below-size windows make zero durable progress | smallest change; memory and disk peaks need checking in `store/` and `transfer.rs` |

### Effect on CORE-004 and fault injection

All four options keep CORE-004 ("incomplete objects MUST NOT be visible as valid")
only if publish stays two-phase **at the granularity a reader can observe**. For C
that means a chunked object is never readable until the whole-object digest
matches, even when every chunk is already published. Whatever is picked, the
existing `xtask fault-injection` suite (corrupted chunk, authority loss, stale
generation, restart recovery, insufficient budget, malicious manifest) must keep
passing, and the option needs these new cases:

- interrupt after *k* of *n* chunks/parts, restart, resume without re-sending
  verified chunks, and never publish the partial object;
- one corrupted chunk inside a large object fails the object and banks nothing
  invalid;
- conflicting bytes for the same chunk index fail the transfer (spec §18.6);
- malicious chunk list: declared size ≠ sum of chunks, a chunk over the cap,
  duplicate or out-of-range indexes;
- budget windows below the object size: C banks chunks, S/A refuse before any
  bytes;
- staging runs out of disk mid-object: fail closed, nothing published.

## Recommendation (proposed)

1. **Keep option B** as the default and do not raise the cap now. Every frontier
   flagship already fits: PhotoCraft splits into tiles, and Kiwix keeps the bulk
   out of the cut.
2. **Defer C** until a real session that does not tile and exceeds 16 MiB shows up
   (for example a single large media file carried as authoritative state). If one
   does, prefer **C over S and A**. It is the only option that generalizes B's
   window-by-window progress to formats with no seams, and it matches the spec's
   chunk model.
3. **Reject A** as the general answer: a raised cap brings back the monolith
   failure (no durable progress below the object size) and higher memory/disk
   peaks on mobile.
4. If C goes ahead: design note first, then the chunk-list record and its
   migration (CORE-007), additive ABI fields only, and the fault cases above before
   any label above `TARGET`.

Owner decision: **pending**. Open questions for the owner:

- Is there a concrete non-tiling > 16 MiB session on the roadmap? Without one, the
  answer is "B only" and this issue can close as decided.
- If C: fixed-size chunks (simple, matches the 1 MiB default) or content-defined
  chunking (better dedup for mutable files, spec §32 item 4)?
