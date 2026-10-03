# Carry-On benchmark

The preparation-strategy benchmark is now a repeated, swept, statistical study.

- **Run it:** `cargo run -p xtask -- bench docs/benchmark --reps 30`
- **Full results:** [`benchmark/RESULTS.md`](benchmark/RESULTS.md) — categorized report
  (methodology, machine, headline, per-metric, per-strategy, per-axis sweeps, verdict
  matrix, skipped combos).
- **Raw data:** `benchmark/raw-results.json` (every repetition of every strategy of
  every config) + `benchmark/summary.json` (per-config aggregate stats + verdict).

Compares four strategies — full selected-state transfer (A), ordinary save/reopen (B),
pure demand loading (C), Carry-On progressive / action-conditioned preparation (D) —
across a document-size × optional-size × demand × chunk-size grid. All metrics are
MEASURED (real wire bytes, `Instant` wall-clock, `getrusage` CPU/RSS). LOCAL evidence
(loopback, in-process; spec §2/§30), not physical cross-device. `endpoint_changed_symbols`
is a separate symbol-distance metric (spec §5.6), never bytes or runtime.
