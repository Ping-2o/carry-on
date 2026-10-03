# Carry-On preparation-strategy benchmark

`cargo run -p xtask -- bench [out_dir]` compares four strategies for continuing a
working editor session on another device. All figures are **measured**, in-process,
over a real loopback TLS 1.3 link — wire bytes from the transport counters, wall-clock
from `Instant`, peak RSS + CPU from `getrusage`. LOCAL evidence (§2/§30): not a
physical cross-device measurement. For the physical L3 handoff see
`crates/carryon-ffi/shells/android-cli/l3_handoff.sh`.

## Strategies

| id | strategy | what it moves before the first useful action |
|----|----------|----------------------------------------------|
| A  | full selected-state transfer | prerequisites **and** optional payload up front |
| B  | ordinary save / reopen | serialize the whole store to disk, reopen (no selective move; local, no network) |
| C  | pure demand loading | nothing up front; the action pulls prerequisites on first use; optional only if demanded |
| D  | **Carry-On progressive / action-conditioned** | prerequisites up front (reach `ACTION_READY`); **defer** optional until/unless demanded |

Objects: `editor.document.v1` + `editor.unsaved_edits.v1` + `editor.meta.v1` are
authoritative **prerequisites** (sealed into the cut, needed for correctness);
`editor.navigation.v1` is **optional** (ephemeral, latency-only, not sealed).

## Metrics (all measured)

- `time_to_action_ready_ms` — to the `ACTION_READY` state (§16.2 exact label).
- `source_independence_ms` — to the point the destination needs no source.
- `bytes_before_first_action` / `total_bytes` — real wire bytes (transport counters).
- `prepared_but_unused_bytes` — moved but never used (waste).
- `cpu_ms`, `peak_rss_kb` — `getrusage(RUSAGE_SELF)`.
- `normal_use_overhead_ms` — per-session action-conditioned prep cost even with **no**
  handoff (the bookkeeping a plain editor skips).
- `endpoint_changed_symbols` — **separate** symbol-distance metric (spec §5.6). It is
  NOT bytes and NOT runtime; it is reported in its own column and never conflated.

## Scenario matrix — Carry-On loses, ties, and wins (honestly)

| scenario | result for D | why |
|----------|--------------|-----|
| `no-handoff` | **lose** | No transfer ever happens; D's progressive preparation is pure overhead A/B/raw never pay. |
| `tiny-no-optional` | **tie** | Nothing to defer; deferral is neutral. |
| `small-optional-demanded` | **tie** | The optional is cheap and needed anyway; bundling vs deferring is a wash. |
| `big-optional-unused` | **win** | Large optional the action never needs: D moves ~5 KB before first action vs A ~800 KB (and A wastes ~800 KB). |
| `big-optional-demanded` | **win** | Large optional needed later: D reaches `ACTION_READY` ~5× sooner than A; total bytes converge. |

Verdict (`v`) compares D against **A** (both real selective transfers; B is a
different, network-free modality shown as a baseline). Representative run
(numbers vary by host/run):

```
scenario                 strategy                 TTA_ms   srcIndep    bytes_1st  total_bytes   wasted_B  ovhd_ms   v
no-handoff               D carryon-progressive      98.5       98.5         5148         5148          0    0.588  lose
big-optional-unused      A full-selected           457.5      457.5       805636       805636     800488    0.000
big-optional-unused      D carryon-progressive      99.8       99.8         5148         5148          0    0.537   win
big-optional-demanded    A full-selected           448.5      448.5       805636       805636          0    0.000
big-optional-demanded    D carryon-progressive      91.0      448.5         5148       805636          0    0.829   win
```

Full results (every strategy × scenario, plus `cpu_ms`/`peak_rss_kb`) are written to
`<out_dir>/bench-results.json`.
