# Kiwix bridge — Phase 2 evidence

Date: 2026-10-28. Same rules as Phase 1: labels cite the exact commands and
the outputs below were captured from real runs. Everything on this page ran
on this machine or on the physically connected rooted Galaxy A04
(`adb-R92TC05FELE`, magisk root, Wi-Fi adb).

## A. libzim C-ABI backend — IMPLEMENTED (macOS host), OBSERVED (physical, Android)

Stack: `crates/carryon-adapter-kiwix/shim/czim.{h,cpp}` (C++ shim over libzim
at the pinned `ad888b4`, fail-closed C ABI) → `src/zim_backend.rs` (safe Rust
FFI, feature `zim-backend`, default on) → `KiwixAdapter::from_zim`.

### A.1 Host tests against REAL ZIMs — IMPLEMENTED

`cargo test -p carryon-adapter-kiwix` — 12/12, zero skips. The 4 backend
tests (`tests/zim_backend.rs`) run against the real
`wikipedia_en_climate_change_mini_2024-06.zim` (kiwix's own testing-suite,
8,978,471 B) and assert: identity read; chunked 1000-byte-window article
reads reassemble to the full item AND agree with a whole read; unknown entry
fails closed; `from_zim` builds a session whose bulk object serves at raw
file offsets (ZIM magic `ZIM\x04` at 0), whose chunked reassembly matches the
declared streaming sha256, whose `reader.navigate` extracts the NEW article's
bytes from the ZIM, and whose nonexistent-entry navigation is refused leaving
the generation untouched.

```
test result: ok. 8 passed; 0 failed  (tests/kiwix_adapter.rs — pure Rust)
test result: ok. 4 passed; 0 failed  (tests/zim_backend.rs — REAL ZIM, 0 skips)
```

### A.2 Cross-builds — IMPLEMENTED

Android arm64 (NDK r27c, static libzim + lzma + zstd from the Phase-1
`--wrap-mode=forcefallback` build, `c++_static`):

```
$ file .../aarch64-linux-android/debug/examples/zim_probe
ELF 64-bit LSB pie executable, ARM aarch64, ... interpreter /system/bin/linker64
```

iOS arm64 (kiwix-build static libzim at `/Volumes/kbwork`):

```
cargo build -p carryon-adapter-kiwix --target aarch64-apple-ios --lib   # exit 0 (FFI feature on)
```

### A.3 On-device probe — OBSERVED (physical)

Pushed the arm64 `zim_probe` (plus the NDK's own `libc++_shared.so`, standard
NDK deployment — the device's system/ART copy lacks the NDK-version symbols)
to `/data/local/tmp/carryon/` and ran it on the Galaxy A04's CPU against the
ZIM the Kiwix app itself had downloaded:

```
$ adb shell "LD_LIBRARY_PATH=/data/local/tmp/carryon /data/local/tmp/carryon/zim_probe \
    /storage/emulated/0/Android/media/org.kiwix.kiwixmobile.standalone/Kiwix/wikipedia_ru_mathematics_mini_2026-10.zim"
uuid=e648a210-47dd-5898-33e1-88f16a0ec2c8
title=Математика по Википедии
filesize=27010448
entry_count=31559
main_path=index
header_magic=ZIM\x04
article_path=index
article_title=Main Page
article_size=30723
article_sha256=795cdfcd45285b3432ab3668d2d76a77a52a182eb600a257e707a79d6404bede
PROBE_OK
```

The host probe on the pulled ZIM prints **byte-identical** values (same uuid,
size, entry count, article sha256 `795cdfcd…`) — the backend agrees across
host and device.

## B. Real session capture from the real app — OBSERVED (physical)

App: official `org.kiwix.kiwixmobile.standalone` **v3.14.1**
(`v3.14.1-arm64-v8a-standalone.apk` from GitHub releases, `adb install`).
The user then browsed the app's own online catalog, downloaded
`wikipedia_ru_mathematics_mini_2026-10.zim` (27,010,448 B) into the app's
media dir, and opened articles (Room history shows
`Эйнштейн,_Альберт` then `index`).

Empirical storage layout of v3.14.1 (recorded honestly — it differs from
repo HEAD, which had moved to Jetpack DataStore by 2026-09-21): the same
keys `currentzimfile`/`currenttab` live in the classic SharedPreferences
`<filesDir>/shared_prefs/kiwix-mobile.xml` (the exact file the repo HEAD
migrator reads):

```xml
<string name="currentzimfile">/storage/.../wikipedia_ru_mathematics_mini_2026-10.zim</string>
<int name="currenttab" value="0" />
```

Room `KiwixRoom.db` (pulled with `-wal`/`-shm`; the device has no sqlite3
binary, so queries ran on the host copy):

```
WebViewHistoryEntity: zimId=e648a210-47dd-5898-33e1-88f16a0ec2c8,
                      webViewIndex=0, webViewCurrentPosition=0,
                      bundle=3988 B (Chromium back/forward list)
HistoryRoomEntity (latest): https://kiwix.app/index | Википедия
```

`kiwix_capture` (host, `examples/kiwix_capture.rs`) reads either container
(XML detected), cross-checks the ZIM uuid against the real file, and extracts
the current article bytes from the real ZIM:

```
prefs container: legacy SharedPreferences XML
CAPTURED zim identity from real file: uuid=e648a210-... size=27010448
currentzimfile = /storage/.../wikipedia_ru_mathematics_mini_2026-10.zim
currenttab     = 0
scrollY        = 0        (honest: no scroll captured on the current page)
article        = index / Википедия, 30,723 B extracted from the real ZIM
DROPPED: webViewBackForwardListBundle: 3988 B carried as opaque payload,
         NOT interpreted (platform-versioned Chromium serialization)
DROPPED: bookmarks (app stores them in libkiwix XML; out of Phase 2 scope)
CAPTURE_OK
```

The session JSON it emits is exactly `carryon_adapter_kiwix::Session`
(zim identity + position + opaque payload + settings).

## C. The flagship measurement — OBSERVED (LOCAL byte accounting over REAL captured state)

`examples/kiwix_measure.rs`, run against the captured ZIM + position. Both
strategies move **real object bytes through the adapter's `read_object`**
chunk path (64 KiB chunks) — no arithmetic shortcut:

```
strategy A (everything up front):
   kiwix.session.v1       400
   kiwix.article.v1    30,723
   kiwix.zimfile.v1 27,010,448
   kiwix.bookmarks.v1      2
   kiwix.history.v1        2
   kiwix.settings.v1      34
   kiwix.meta.v1          82
   bytes_before_first_action = 27,041,691

strategy D (prerequisites only):
   kiwix.session.v1      400
   kiwix.article.v1  30,723
   kiwix.meta.v1          82
   bytes_before_first_action = 31,205

D/A ratio            = 0.001154 (0.1154%)
D < 50% of A         = true
first paint from article object alone (no ZIM) = true   (sha256 == manifest digest)
A/D asymmetry factor = 866.6x
MEASURE_OK
```

Honest scope of this number:
- It is a **LOCAL, single-machine** byte accounting over the REAL captured
  session (real ZIM, real article bytes, real session state). No network
  transfer has happened yet — the physical mac↔device LAN carry is Phase 3
  (the `bench_xdev.sh` pattern).
- The captured ZIM is 27 MB, not 80 GB: the ratio **scales worse for D's
  advantage as the library grows** — with a full English Wikipedia ZIM
  (~80 GB), the same session+article prerequisites (~31 KB) put the ratio near
  4×10⁻⁷. The 27 MB book is the conservative case and it already shows 866.6×.
- "First paint from the article object alone" is a destination-side
  reconstruction check (sha256 of the transferred article bytes == the
  manifest digest), not a rendered UI. The companion shell is Phase 3.

## D. What did NOT work (honest negatives)

- **v3.14.1 storage mismatch with repo HEAD research**: the DataStore
  protobuf path does not exist on the installed release; the capture tool
  was adapted (SharedPreferences XML). Anyone targeting HEAD sources must
  re-verify the container empirically per release.
- **`file://` VIEW intent** opens the app but not the reader — the library
  flow had to be driven manually (the user did the downloads/browse step).
- **zim_probe first run on device**: `CANNOT LINK EXECUTABLE ... _ZTTNSt6__ndk1…`
  — the executable's `libc++_shared.so` needed is the NDK one; the
  system-resolved copy lacks NDK symbols. Fixed by shipping the NDK's
  `libc++_shared.so` next to the binary (`LD_LIBRARY_PATH`), the standard
  NDK deployment shape.
- **libkiwix → Android** stays UNSUPPORTED (per Phase 1 verdict; not chased
  this phase by design).

## E. Reproduction environment

- Device: rooted Galaxy A04, magisk `su`, Wi-Fi adb
  (`adb-R92TC05FELE-Z6JMQk._adb-tls-connect._tcp`).
- ZIM provenance (labeled honestly): `wikipedia_ru_mathematics_mini_2026-10.zim`
  was downloaded BY THE APP from its own catalog (2026-10 edition), not from
  download.kiwix.org; `wikipedia_en_climate_change_mini_2024-06.zim` is from
  kiwix's own `zim-testing-suite-0.10.0` (via kiwix-build's source tree).
- Artifacts: capture set at
  `/Volumes/ADATA ELITE SE880/compilation/…` (SSD) and in the opencode
  temp dir for this session; the device keeps its copies under the app's own
  dirs and `/data/local/tmp/carryon/`.
