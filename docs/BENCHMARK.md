# Carry-On benchmark

The preparation-strategy benchmark is now a repeated, swept, statistical study.

- **Run it:** `cargo run -p xtask -- bench docs/benchmark --reps 30`
- **Full results (mac, loopback):** [`benchmark/RESULTS.md`](benchmark/RESULTS.md) —
  88 configs × 30 reps, categorized report (methodology, machine, headline, per-metric,
  per-strategy, per-axis sweeps, verdict matrix, skipped combos).
- **Physical results (Android device CPU):**
  [`benchmark/RESULTS-physical.md`](benchmark/RESULTS-physical.md) — Galaxy A04 (arm64,
  8 cpu), reduced grid (14 configs × 8 reps) run on real ARM hardware over the device's
  own loopback. `cargo run -p xtask -- bench <dir> --grid small --postfix -physical`,
  cross-compiled for `aarch64-linux-android` and run via adb. Still loopback-within-one-
  device, not mac↔device network.
- **Raw data:** `benchmark/raw-results.json` / `raw-results-physical.json` (every rep of
  every strategy of every config) + `summary.json` / `summary-physical.json` (aggregates).

Compares four strategies — full selected-state transfer (A), ordinary save/reopen (B),
pure demand loading (C), Carry-On progressive / action-conditioned preparation (D) —
across a document-size × optional-size × demand × chunk-size grid. All metrics are
MEASURED (real wire bytes, `Instant` wall-clock, `getrusage` CPU/RSS). LOCAL evidence
(loopback, in-process; spec §2/§30), not physical cross-device. `endpoint_changed_symbols`
is a separate symbol-distance metric (spec §5.6), never bytes or runtime.
