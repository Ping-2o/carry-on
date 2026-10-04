# Carry-On engineering targets

Explicit, checkable engineering targets for the continuation engine, each with a claim
label (per [`AGENTS.md`](../AGENTS.md) §2: `PROVED` / `IMPLEMENTED` / `OBSERVED` / `TARGET`
/ `OPTIONAL` / `RESEARCH` / `UNSUPPORTED`) and a pointer to the evidence that supports it.
Labels are law: none is upgraded without the evidence it requires. A loopback run on
`127.0.0.1` is LOCAL evidence, not physical cross-device evidence; no platform is
"supported" until §30 passes on real hardware.

| # | Target | Label | Evidence |
|---|--------|-------|----------|
| 1 | **Zero authoritative-data loss.** No corrupt, partial, or tampered byte is ever published as a valid object; every object is staged, whole-object digest-verified, then atomically published (CORE-004). | **PROVED** | Two-phase publish invariant (`crates/carryon-core/src/store/publish.rs`); fault-injection `corrupted_chunk` (tampered bytes → OBJECT_DigestMismatch; lying chunk digest → TRANSFER_ConflictingChunk, NET-007), nothing published — `crates/carryon-core/tests/fault_injection.rs`, `xtask fault-injection` → `fault-report.json`. |
| 2 | **Source-off execution.** After authority transfers, the destination continues editing with the source process gone; the source cannot resurrect authority. | **OBSERVED** (physical) | `unsaved_continue.sh`: edit on mac → carry to Galaxy A04 → move authority → `kill -9` mac source → device edits independently → restarted mac source reports `may_mutate == false`. Summary in `source-off-proof.json`; device bundle `unsaved-continue-evidence.json`. Loopback analogue: `authority_handoff.rs` `full_l4_authority_transfer_moves_ownership`. |
| 3 | **Single-writer safety.** Authority is held by exactly one device; an interrupted handoff never yields two writers — it leaves an ambiguous epoch that blocks writes until explicit recovery (AUTH-001..005). | **PROVED** (invariant) / **OBSERVED** (physical) | `guard_mutation` / ambiguous epoch (`crates/carryon-core/src/authority_xfer.rs`); fault-injection `connection_loss_authority_commit` (source keeps authority, dest ambiguous+blocked, recovers); `authority_handoff.rs` `interrupted_commit_leaves_destination_ambiguous_then_recovers`. Physical: `xdev_handoff.sh`. |
| 4 | **Pre-action transfer < 50% of a full selected-state transfer.** Carry-On's progressive strategy (D) moves fewer bytes before the first useful action than moving all prerequisites + optional up front (A). | **OBSERVED** | Independent A/C/D executions. Loopback: `xtask bench` (`docs/benchmark/`). Real Mac→Android: `bench_xdev.sh` → `xtask bench-aggregate` reports the measured D/A `bytes_before_first_action` ratio in `RESULTS-xdevice.md` / `summary-xdevice.json` (field `pre_action_bytes_D_over_A.meets_under_50pct_target`). State the measured ratio and its conditions; do not overclaim beyond the tested grid. |
| 5 | **Bounded preparation overhead.** The engine's action-conditioned planning and budget admission bound the work done before (and the bytes admitted for) a handoff; an over-budget import is refused before any bytes move (§6.8). | **IMPLEMENTED** / **OBSERVED** | Budget admission (`admit_import_budget`, `crates/carryon-core/src/transfer.rs`); fault-injection `insufficient_budget` (BUDGET refusal before bytes); `normal_use_overhead` measurement (`xtask bench`, the no-handoff sweep). |

## How to re-verify

```sh
# 1. Invariants + fault-injection (LOCAL):
cargo test --workspace --features carryon-ffi/test-panic
cargo run -p xtask -- fault-injection docs/evidence/tmp     # -> fault-report.json, 6/6

# 2. Loopback study with INDEPENDENT A/C/D:
cargo run -p xtask -- bench docs/benchmark --reps 30

# 3. Real Mac->Android campaign (needs NDK, MAC_IP, one adb device on the same LAN):
NDK=<ndk> MAC_IP=<this host lan ip> crates/carryon-ffi/shells/android-cli/bench_xdev.sh
#   -> docs/evidence/<stamp>/RESULTS-xdevice.md + summary-xdevice.json

# 4. Source-off unsaved-state continuation:
NDK=<ndk> MAC_IP=<ip> crates/carryon-ffi/shells/android-cli/unsaved_continue.sh
#   -> source-off-proof.json (all_passed:true)

# 5. Assemble the evidence archive:
crates/carryon-ffi/shells/android-cli/collect_evidence.sh docs/evidence/<stamp>
```

## What is NOT claimed

- No signed APK / code signing; no §30 platform acceptance — **no platform is "supported"**
  (PLAT-006 out of scope).
- Target #4's ratio holds **on the tested grid** (document/optional sizes, chunk sizes,
  latency points). It is an `OBSERVED` measurement, not a universal guarantee.
- Carry-On is typed continuation, **never** process/memory migration (see
  [`COMPARISON.md`](COMPARISON.md)).
