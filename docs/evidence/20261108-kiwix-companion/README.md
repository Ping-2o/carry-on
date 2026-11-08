# Stage 10 — Kiwix companion reader, mobile half: mac ⇄ Galaxy A04 (2026-11-08)

The Kiwix flagship's **mobile half**: a companion reader app on the Galaxy A04
(Android 13, arm64) that opens a real ZIM through libzim, renders the article,
and carries the **reading position** to/from the mac over LAN TLS 1.3 — one
small reading session (KBs), never the library (27 MB here; 1.7 GB proven in
the Phase-3/4 bridge evidence).

**Honest labels (§2).** The two carry runs are `OBSERVED (PHYSICAL)`: two
machines (mac + A04), two NICs, real mutual-pinned TLS over 192.168.3.0/24.
Opening/reading the ZIM on one device is `OBSERVED (LOCAL)`. No platform is
"supported" (§30 acceptance unrun); the APK is a debug build (PLAT-006 out of
scope).

## What is measured

| Run | Direction | Wire bytes | bytes_before_first_action | D/A of the ZIM | tta | Result |
|---|---|---|---|---|---|---|
| `mac-source-to-a04.log` + `a04-receive-evidence.json` | **mac → A04** (app receives) | 45,522 B recv (app) / 45,522 B sent (mac) | **45,522 B** | **0.1685 % (1,685 ppm)** of 27,010,448 B | **2,392 ms** (connect→import→restore, app-side) | restored `index` @ scroll **42**; authority epoch 1; `may_mutate` false→true; L4 navigate on-device → generation 2; evidence verified ok |
| `mac-dst-from-a04.log` + `a04-serve-evidence.json` | **A04 → mac** (app serves) | 46,292 B sent (app) / 45,526 B recv (mac) | 45,526 B (mac) | 0.1685 % | 1,509 ms (whole mac dest invocation wall clock) | restored `index` @ scroll 0; authority epoch 1; mac navigates to «Эйнштейн, Альберт» @ 321 → generation 2; stale-cut action refused (OBJECT_Missing, rc 300) |

The ZIM is `wikipedia_ru_mathematics_mini_2026-10.zim` (27,010,448 B, uuid
`e648a210-47dd-5898-33e1-88f16a0ec2c8`), the same archive the Phase-2/3 bridge
evidence used. Both sides hold the SAME file locally (companion model); the
carry moves the session object + the current article (30,723 B, sha256
`795cdfcd…`), which the core verifies against the cut's declared digests
(CORE-004). The authority transfer gate additionally compares the
destination's reconstructed session against the imported object's hash — a
mismatch fails closed (observed as `ADAPTER_Incompatible … does not match
imported state` while the app still passed the wrong position; the final runs
pass with the carried position).

## The reader UI on the A04

- `a04-kiwix-article-open.png` — ZIM opened through libzim: Main Page of the
  math-mini ZIM rendered in a WebView (text-first: assets are separate ZIM
  entries and are not part of the carried article, so images 404 honestly).
- `a04-kiwix-navigate-mathematics.png` — reading moved to `Mathematics` @
  scroll 321 via `reader.navigate` + a re-sealed cut (generation 2).
- `a04-receive-restored-position.png` — the receive console of the final
  mac→A04 run (import → restore @42 → authority → on-device navigate →
  verified evidence).
- `a04-serve-console.png` — the serve side of the A04→mac run.

The screen shows the flagship number directly: `bytes_before_first_action`
and `D/A` against the ZIM size, plus the measured wire counters and the engine
console.

## Reproduction

```sh
# engine + APK with the kiwix feature (real libzim, static for arm64):
CARGO_TARGET_DIR=<checkout>/target-kiwix CARRYON_KIWIX=1 \
  apps/android/Scripts/fetch_engine_lib.sh
cd apps/android && ./gradlew :app:assembleDebug
adb push wikipedia_ru_mathematics_mini_2026-10.zim \
  /sdcard/Android/data/com.carryon.app/files/
adb install -r app/build/outputs/apk/debug/app-debug.apk

# mac-side driver (source or dest role), same libzim backend:
apps/android/Scripts/build_kiwix_shell.sh      # /tmp/carryon-kiwix/carryon_kiwix
$K pin <id_dir>                                 # pin exchange, both sides
$K source <id_dir> <data_dir> 0.0.0.0:39819 <app_pin> <zim> index 42 D
# app: Kiwix tab (or am start extras: kiwix_open=1 kiwix_auto=receive ...)
$K dest <id_dir> <data_dir> <a04_ip:port> <app_pin> <session> <zim> index 0 <nav_path>
```

Engine-level regression on a real device: `./gradlew connectedDebugAndroidTest`
— `KiwixSessionInstrumentedTest` opens the real ZIM, restores, verifies the
article sha256 against the store, navigates to a real entry, and checks the
fail-closed paths.

## Limits (honest)

- Text-first rendering: the carried object is the article; its image/asset
  entries live in the ZIM and are not served by the app (the Phase-4 bridge
  showed a full first paint with assets via its own local server).
- The destination must know the source's position for the adapter's local
  state reconstruction (same discipline as the physical shell); the engine's
  digest gate proves the reconstruction equals the carried session. A future
  engine API that lets an adapter read imported object bytes would remove that
  step — recorded as an engine gap, not claimed here.
- libzim links the C++ runtime dynamically in the final cdylib
  (`libc++_shared.so` is packaged into the APK).
- No signing, no APK distribution, no §30 acceptance.
