# Kiwix bridge — Phase 3 evidence

Date: 2026-10-28/29. Same rules: labels cite real command output; every number
below is copied from a captured log (all logs in
`evidence/20261028-kiwix-phase3/`). Machines: **mac source**
(MacBookAir10,1, Apple silicon, macOS 27.0, 8 cores — `device-specs.txt`)
and **destination = rooted Samsung Galaxy A04** (`SM-A045F`, Android 13,
arm64-v8a, MediaTek mt6765, 3.8 GB RAM — same file), connected over Wi-Fi adb
(`adb-R92TC05FELE-Z6JMQk._adb-tls-connect._tcp`), **two machines, two NICs,
real LAN TLS 1.3 with mutual certificate pinning** (PLAT-001 physical
cross-device evidence; still no APK/signing, no §30 platform claim).

The ZIM is the one the real Kiwix app downloaded on-device in Phase 2:
`wikipedia_ru_mathematics_mini_2026-10.zim` (27,010,448 B, uuid
`e648a210-47dd-5898-33e1-88f16a0ec2c8`). The mac holds a byte-identical copy
(sha256-verified in Phase 2, `9e1b3a3e…`).

## A. Physical LAN carry + L4 authority over the reading position — OBSERVED (physical)

`crates/carryon-adapter-kiwix/shell/carryon_kiwix.c` (the `carryon_xdev.c`
pattern, kiwix edition), driven through the real Carry-On C ABI
(`libcarryon_ffi.so` built `--features kiwix`, aarch64-linux-android release,
NDK r27c; the mac side links the same code as a dylib). Adapter:
`org.carryon.kiwix` registered from the new factory arm — both machines open
the SAME ZIM file locally through the libzim C backend (no fork, no app
changes).

Sequence and real wire counters (from `physical-src.log` / `physical-dst.log`):

1. **Progressive cut carry (strategy D)** — the destination received
   **45,522 bytes total wire** (`dest(after import): WIRE sent=1859
   recv=45522`) for the entire sealed cut: session + current article
   (30,723 B) + bookmarks/history/settings/meta. The 27 MB ZIM bulk was
   never sent: it is an L1 object (Cache kind is impossible above the
   engine's 16 MiB cut-object cap — see PHASE3 findings — its digest rides
   inside the session object).
2. **`reader.restore` on the device**: oracle agreed; article sha256
   `795cdfcd45285b3432ab3668d2d76a77a52a182eb600a257e707a79d6404bede` —
   identical to the Phase-2 on-device probe of the same article.
3. **L4 authority transfer (§21.2)** over the same TLS session: proposal →
   accept → relinquish receipts match on both machines
   (`77175902cdb8d6dd…` binds authority to the portable position state);
   `may_mutate` flipped **false on the mac source, true on the device**.
4. **`reader.navigate` ON THE DEVICE after authority transfer** — the L4
   story closed on real hardware: the destination moved the reading
   position to «Эйнштейн,_Альберт» (generation 2, scroll 321) and extracted
   the new article's bytes from its own local ZIM through the backend
   (`article_bytes_available: true`), all on the A04's CPU.
5. Bonus fail-closed proof: acting at the OLD cut after the move is refused
   by the engine (`OBJECT_Missing: object kiwix.session.v1 gen 2`) —
   recorded as `RESTORE-AT-OLD-CUT refused (expected — stale cut)`.

Durable device-side state pulled into this evidence dir: the destination
core's `metadata.sqlite3` (mirror session `80c22b72…`, epoch 1, adapter
`imported`) and its journal — last events include
`AuthorityTransferCommitted … {"new_epoch":1,"now_authoritative":true,
"proposal_id":"46f5d11a-8882-4150-a99c-239f2a287191","role":"destination"}`
and the `ActionExecuted` records. Plus `device-data-dir.txt` (store layout)
and a device screenshot.

A **LOCAL loopback rehearsal** of the exact same flow ran green first
(`carryon_kiwix` source/dest both on the mac, 127.0.0.1) — labeled LOCAL,
kept in the log trail.

## B. Progressive ZIM materialization — OBSERVED (physical) + OBSERVED (local full-transfer A)

`examples/kiwix_materialize.rs`: destination-side sparse-file writer fed by
`read_object`-equivalent bulk chunks over a second pinned TLS session,
speaking the engine's own digest-addressed wire messages
(`TransferRequest`/`ChunkData`, per-chunk sha256 verified, fail-closed range
checks). The chunk loop is adapter-level (the core has no partial-object
fetch API — recorded as an engine gap for Phase 4); the TRANSPORT is the real
mutual-pinned `carryon-net` TLS with real counters.

Measured layout (from the file's own header — `materialize-physical-dst.log`):
ZIM v6.3, 31,576 articles, 94 clusters; clusters start at 2048; **dirents live
at 88% into the file**; urlPtr/clusterPtr tables at the end; checksum at
27,010,432.

Physical run (mac → A04, `materialize-physical-{src,dst}.log`):

| stage | fetched | % of ZIM |
|---|---|---|
| header + mime list + dirents + urlPtr + clusterPtr tables | 3,148,592 B | 11.657% |
| article's cluster (92) | +44,651 B | |
| title-listing cluster (93, required by libzim's open — `fileimpl.cpp:264`) | +3,231,773 B | |
| **article readable** | **6,425,016 B** | **23.787%** |

- `dest: ARTICLE READABLE — materialized 6425016/27010448 bytes = 23.7872% of
  the ZIM (index 3148592 + cluster 3276424)`
- Wire (real counters): dest `sent=8341 recv=8577052`; source agrees
  (`served 6425016 bytes of 27010448 (23.787%)`). The sha256 of the article
  read from the partially-materialized ZIM on the device equals every earlier
  measurement: `795cdfcd…`.
- Sparse hole kept: 76% of the library was never transferred.
- A **full transfer** over the same wire (LOCAL `--full` run,
  `materialize-full-local-*.log`) measures the strategy-A wire cost:
  `recv=36048737` bytes for all 27,010,448 — i.e. progressive D is **23.8%**
  of full A on the wire for this book.
- Honest scope: 23.787% is dominated by the title-listing cluster (3.23 MB,
  mandatory at open). For first-paint WITHOUT libzim (the companion-shell
  path) the engine carry in §A already needs only ~45.5 KB — the materializer
  number is specifically "open the real ZIM natively with libzim on a
  library-less device".

## C. GB-scale byte math — OBSERVED (LOCAL)

`kiwix_measure` on `wikipedia_en_medicine_maxi_2026-10.zim`
(download.kiwix.org, **1,864,210,320 B**, "WikiMed Medical Encyclopedia",
476,201 entries; real object bytes moved through `read_object` chunking,
`measure-gb-medicine.txt`):

- Strategy A (everything up front): **1,864,223,792 B**
- Strategy D (prerequisites only): **13,434 B** (session 539 + article
  12,813 + meta 82)
- **D/A = 0.0007% ≈ 7.2×10⁻⁶ — an asymmetry factor of 138,769×**
- First paint from the article object alone (no ZIM): verified
  (sha256 `37dbc021…` == manifest digest).

The 80 GB Wikipedia case extrapolates to ~4×10⁻⁷; the 1.7 GB measured number
is the honest recorded value.

## D. What did NOT work / was not done this phase (honest negatives)

- **iOS simulator run: NOT DONE** (scope cut at wrap-up). What exists:
  kiwix-build built libzim for `ios_simulator_arm64` successfully
  (`configure OK 8.3s / compile OK 27.7s / test OK / install OK` —
  `/Volumes/kbwork/out/BUILD_ios_simulator_arm64/INSTALL`), and the crate
  compiled for `aarch64-apple-ios` (device staticlib) in Phase 2. The
  simulator *execution* of the backend remains UNSUPPORTED-by-evidence —
  compile-level only.
- **Engine A-mode for the bulk over the wire is impossible**: the core caps
  cut objects at 16 MiB (`CORE_MAX_OBJECT_BYTES`), so a 27 MB+ ZIM can never
  ride a cut manifest. The A numbers in §B/§C are real byte movements through
  the adapter/transport, not engine-cut transfers. Lifting or tiering this
  cap is an orchestrator decision, not mine.
- **First materializer design was wrong twice** (recorded in the trail): the
  naive `[0, clusterPtrPos)` "index" fetch moved 100% of the file (the
  cluster-pointer table lives at the END, after the clusters), and sparse
  open failed until (a) the title-listing cluster was fetched and (b) a
  no-xapian-preload open variant (`czim_open_noxapian`) was added. Both fixed
  with real layouts read from the actual file.
- `reader.navigate` to a nonexistent entry refuses the whole action (fail
  closed, tested in Phase 2); the stale-cut refusal in §A.5 is engine
  discipline working as designed.

## E. Reproduction pointers

- Physical pair: `shells/carryon_kiwix.c` + `libcarryon_ffi.so (--features
  kiwix)`, NDK r27c toolchain, push set includes the NDK's own
  `libc++_shared.so` (the system copy lacks NDK symbols — the §E gotcha from
  Phase 2, handled in this phase's scripts by shipping it alongside).
- Materializer: `cargo build -p carryon-adapter-kiwix --example
  kiwix_materialize [--target aarch64-linux-android --release]`.
- ZIMs: `wikipedia_ru_mathematics_mini_2026-10.zim` (app-downloaded,
  on-device), `wikipedia_en_medicine_maxi_2026-10.zim` (1.7 GB, SSD).
