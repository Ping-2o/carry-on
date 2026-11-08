# Stage 11 — the block-sum theorem in the engine (RESEARCH) — 2026-11-08

**Labels are law (§2).** The theorem is `PROVED` in `main.tex`. Everything in
this bundle is a `RESEARCH` implementation + `OBSERVED (LOCAL)` measurement over
real data on one machine. The metric is **endpoint Hamming cost** — the number
of persistent representation symbols whose values change after one source
update. It is **not** wire bytes, latency, probes, or a delivery win. No platform
is "supported" (§30 unrun).

## 1 · Encoding site: the sealed cut (and why not the others)

| Candidate | Verdict |
|---|---|
| **Sealed cut** (§6.2) | **Chosen.** An immutable statement of the handoff boundary: an ordered vector of authoritative object versions. Coordinate `i` = the served size (`logical_size`) of authoritative object `i`, in cut ordinal order. One tile recompressed by one byte is one source-coordinate update. Checkpoint prefixes decode exact byte sums for coarse regions. |
| Hash-chained journal (§23.1) | Rejected. Append-only: an event append never rewrites earlier symbols, so endpoint Hamming distance is the wrong cost model. |
| Content-addressed store layout (§19.1) | Rejected. Paths are digests and payloads are opaque; the first `b_t` bytes of a stored object are not an exact block-sum view. A byte prefix is not a checkpoint under the definition. |

The index is **derived, never persisted**, and the production transfer path does
not consume it. It sits behind an opt-in `carryon-core` feature,
`blocksum-checkpoint`; `carryon-ffi` (and therefore both apps) never enables it
(verified with `cargo tree -p carryon-ffi -e features`).

## 2 · What was implemented

- `crates/carryon-core/src/checkpoint.rs` (feature `blocksum-checkpoint`):
  - `CheckpointPlan::tile_pyramid(cols, rows)` — a strict hierarchy over a tile
    grid: whole → two row bands → rows → two column halves per row → cells.
    Candidates that would not strictly refine the previous level are skipped, so
    any grid (including a single cell) is valid.
  - `encode` — the proved all-but-one-child construction from
    `carryon-math-core` (no reimplementation), plus independent
    `reference_views` block sums.
  - `single_update` / `dilation` — the theorem's unit and the aggregate over all
    coordinates, failing closed if `q+1` (or the per-prefix `t+1`) is exceeded.
  - Engine hook: `Core::cut_checkpoint_coordinates`, `Core::checkpoint_index`,
    `Core::checkpoint_dilation`, `Core::checkpoint_endpoint_distance` — the
    coordinates come from `cut_objects ⨝ objects` rows of a real sealed cut.
- `xtask bench-blocksum` — reads a real tile inventory and writes
  `benchmark.json` (this directory).
- Tests: unit tests over the real grid + degenerate grids, proptest properties
  (any grid / any update within the bounds; equal block sums ⇒ equal prefixes),
  an integration test that seals a real cut and derives the coordinates from it.
- Kani (`prototype/math-core/src/verification.rs`): 4 loop-free harnesses of the
  four-coordinate layout — update ≤ q+1, prefix purity at checkpoints 0 and 1,
  encode/update correctness. **4/4 verified, 0 failures** (0.68 s). Model-level
  (the `prototype/authority-model` pattern), not proofs of the Rust encoder.

## 3 · The real hierarchy and data

The measurement uses the real PhotoCraft demo photo saved by the pinned engine as
a tile-split bundle (the existing `tile_progressive` harness; log in
`photocraft-tile-run.log`):

- photo: 2400×1500, `sha256 03fad779…617fd2`, PhotoCraft pin `1f7fc0f5…`
- Background layer: 60 tiles of 256×256 (10×6 grid), 5,914,711 compressed bytes
- inventory: `tile-inventory.json` (per-cell `tx`, `ty`, BLAKE3 name, exact
  `.zst` byte size = the Carry-On object's `logical_size`)

The full split bundle also has 6 Retouch tiles at overlapping positions; they
are a separate layer and are not part of this full-canvas coordinate set.

Hierarchy: whole (1) → row bands (2) → rows (6) → column halves per row (12) →
cells (60). **q = 4**, bound **q+1 = 5**, checkpoint lengths `b = (1,2,6,12,60)`.

## 4 · Measurement

| Fact | Value |
|---|---|
| single-tile update (the theorem's unit) | max **5** = q+1 (attained), min 1, mean **3.466** |
| histogram (changed symbols → count) | 1:1, 2:8, 3:21, 4:22, 5:8 |
| bound holds for all 60 updates | **yes** (fails closed otherwise) |
| checkpoint views vs independent block sums | **equal at every level** |
| §5.11 applicability gate | pass (`prefix_semantics = view-pure`, `vp-1`) |
| six-tile update (simulated) | endpoint distance **12** — *not* the theorem's unit; no q+1 claim |
| raw identity layout, for contrast | endpoint distance 1, **not view-pure**; theorem does not apply |
| checkpoint delivery sizes (`b_t × 8 B`) | 8, 16, 48, 96, 480 bytes — metadata, not document bytes |

**Verdict:** the view-pure construction attains the proved bound exactly on a
real 60-tile hierarchy. This is an **update-cost** result: it says nothing about
delivery bytes or latency, and no transfer path consumes the index. There is no
delivery win to claim, and none is claimed.

Two further limits, stated plainly: the index is a **size summary**, not a
content digest — a tile edit that preserves the served byte length does not move
its coordinate; and the prefix checkpoint sizes (8–480 bytes) are metadata, not
a reduction of the document payload.

## 5 · Reproduce

```sh
# measurement over the committed real inventory (no PhotoCraft build needed)
cargo run -q -p xtask -- bench-blocksum
#   -> docs/evidence/20261108-blocksum-checkpoint/benchmark.json

# unit + property + engine-hook tests
cargo test -p carryon-core --features blocksum-checkpoint

# model-level Kani proofs (4 harnesses)
cargo kani -p carryon-math-core
```

Re-capturing the inventory (OBSERVED, LOCAL; needs the pinned PhotoCraft deps):

```sh
cargo run -q -p carryon-adapter-photocraft --features inprocess --example \
  tile_progressive -- <photocraft-demo.jpg> /tmp/tile-progressive.json
# then read $TMPDIR/pcraft-tile-progressive/base-bundle/manifest.json + tiles/
```

## 6 · Gates (this branch)

- Rust: `cargo fmt --all -- --check` clean; clippy `-D warnings` (workspace,
  kiwix excluded as in CI) clean; full `cargo test --workspace --features
  carryon-ffi/test-panic` green (the feature is enabled for the workspace by the
  `xtask` dev dependency, so the new tests run; the apps' `carryon-ffi` build
  does not enable it).
- Kani: `cargo kani -p carryon-math-core` — 4/4 verified.
- No ABI, app, schema, or platform changes; no device phase (the A04 was busy
  with the Kiwix session and was not touched).
