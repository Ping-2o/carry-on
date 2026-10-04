# Carry-On benchmark

The preparation-strategy benchmark is a repeated, swept, statistical study. Strategies A,
C, and D are each run as an **independent end-to-end execution** (own cores, own TLS
session, own transfer) — they are *not* derived arithmetically from one shared
measurement. There are two campaigns:

1. **Loopback study** (`xtask bench`): fast, CI-able, in-process over 127.0.0.1. LOCAL
   evidence (spec §2/§30).
2. **Real Mac→Android campaign** (`bench_xdev.sh` → `xtask bench-aggregate`): each trial is
   one real cross-device TLS transfer over the LAN. PHYSICAL cross-device evidence
   (PLAT-001); still no APK/§30 acceptance. Output `RESULTS-xdevice.md` /
   `summary-xdevice.json` in an archive under [`evidence/`](evidence/).

See also [`COMPARISON.md`](COMPARISON.md) (Carry-On vs cloud save/reopen, rsync, CRIU, VM
migration, demand loading) and [`TARGETS.md`](TARGETS.md) (engineering targets + claim
labels + evidence pointers, incl. the measured D-vs-A pre-action byte ratio).

## Loopback study

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

## Real Mac→Android campaign (physical cross-device)

Each trial runs one strategy end-to-end over a real LAN TLS 1.3 link: the mac is the
source, the Galaxy A04 is the destination (via adb). The campaign is randomized across
state sizes, injected latency, optional-state ratio, demand, and a no-handoff case
(≥30 trials; seed recorded). Latency is injected with `CARRYON_NET_DELAY_MS`, a bench-only
per-frame send delay that inflates real wall-clock only (the byte counters stay real).

```sh
# needs NDK, MAC_IP (this host's LAN ip), one adb device on the same LAN:
NDK=<ndk-dir> MAC_IP=<lan-ip> crates/carryon-ffi/shells/android-cli/bench_xdev.sh
# -> docs/evidence/<stamp>/{trials.jsonl, RESULTS-xdevice.md, summary-xdevice.json}
crates/carryon-ffi/shells/android-cli/collect_evidence.sh docs/evidence/<stamp>
```

`bench-aggregate` reports median, p95, mean, stddev, 95% confidence interval, and failure
counts per strategy, a latency sweep, and the D-vs-A pre-action byte ratio (the < 50%
target). Every trial line is one independent execution — nothing is composed.

## Fault injection

`cargo run -p xtask -- fault-injection <dir>` drives six fault classes (corrupted chunk,
connection loss in the authority commit window, stale generation, restart recovery,
insufficient budget/storage, malicious manifest) through the real engine and writes
`fault-report.json` — each must **fail closed**. The same classes are asserted as tests in
`crates/carryon-core/tests/fault_injection.rs`.
