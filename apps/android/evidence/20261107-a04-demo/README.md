# Android Demo Mode — first device run (Galaxy A04, 2026-11-07)

Evidence bundles from the **first ever run of Carry-On for Android on a real
device**: Samsung Galaxy A04 (SM-A045F, Android 13, arm64), driven over adb
(`input tap` + `uiautomator dump`). The APK was assembled locally from this
branch head (engine cross-compiled by `Scripts/fetch_engine_lib.sh`, NDK r27c)
after the CI-built APK crashed at its first FFI call (see the DT_NEEDED bug in
the commit log); engine ABI 1.1.

> **Labels are law (spec §2).** Every bundle here is `OBSERVED (LOCAL)` —
> **device-LOCAL**: the Demo Mode pipeline runs loopback TLS 1.3 **inside the
> one device** (two in-process cores in the app's own cache dir). This is NOT
> physical cross-device transfer evidence (no second machine, no second NIC)
> and Android is NOT a "supported" platform (§30 acceptance has never been
> run). Each bundle carries the engine's own LOCAL disclosure.

## Runs (all 8 stages DONE, `verify_evidence` ok=true)

| Bundle | Scenario | Toggles | `time_to_action_ready_ms` | dest recv → ready | Notes |
|---|---|---|---|---|---|
| `l3-editor-baseline-evidence.json` | L3 editor | none | **692.1 ms** | 2556 B | §21.2 flip: dest owns, src read-only, epoch 1 |
| `l3-editor-all-toggles-evidence.json` | L3 editor | chunk 4 KiB + §6.8 budget + §11.2 suspend + recovery report | 1301.8 ms | 5212 B | 3 wire rounds: partial-budget prefix → suspend → resume (worst case; the extra rounds are the showcase itself) |
| `graph-baseline-evidence.json` | graph L2 | none | **586.2 ms** | 2215 B | authority honestly skipped (`supports_authority_transfer:false`); read-only mirror + `graph.shortest_path` |
| `graph-all-toggles-evidence.json` | graph L2 | all four | 1076.9 ms | 4117 B | 3 wire rounds: typed `E_BUDGET_NETWORK` refusal → suspend → resume |

`tta` on the A04 (baseline runs, **< 1 s**): ~0.59–0.69 s, consistent with the
mac loopback demo's order of magnitude. Foreground-first admission (§20.2) is
set on both cores before every import (console line `foreground: true on both
cores`); toggled runs add deliberate extra TLS round-trips (suspend/resume,
budget refusal/replenish), which is why their wall-clock exceeds 1 s — that is
the showcase doing its job, not an admission regression. `bytes_recv_to_action_ready`
counts destination-side wire bytes only (review P2 in DemoRunner.kt).

Every run's engine console ended with the pipeline-complete line and the
verbatim DISCLOSURE footer; `may_mutate` flips were checked per iteration
(editor: src→false, dst→true after epoch 1; graph: dst stays false — honest
read-only mirror).

## How to reproduce on a device

```sh
apps/android/Scripts/fetch_engine_lib.sh
cd apps/android && ./gradlew :app:assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
# Demo Mode tab → scenario + toggles → ▶ RUN DEMO
adb shell run-as com.carryon.app cat cache/demo/<stamp>/dst/evidence.json
# regression suite (device connected, ~2 min):
./gradlew :app:connectedDebugAndroidTest
```

Note: `run-as` is blocked on this Samsung shell build; the bundles were pulled
via `su` from `/data/data/com.carryon.app/cache/demo/<stamp>/dst/evidence.json`.
