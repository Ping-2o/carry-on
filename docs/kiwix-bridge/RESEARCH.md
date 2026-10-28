# Kiwix bridge — Phase 1 research (RESEARCH label unless cited evidence)

Research date: 2026-10-28. All citations are to real files at the pinned commits
below (clones under `/Volumes/ADATA ELITE SE880/compilation/`). Claim labels per
`AGENTS.md` §2. Nothing here upgrades a label without a log.

## 0. Pinned sources

| Repo | URL | Pinned commit | Date | Notes |
|---|---|---|---|---|
| libzim | github.com/openzim/libzim | `ad888b4cdc5c76d788e9e70b630842dc1b31871f` | 2026-10-19 | Merge PR #1137; version 9.8.2 (meson) |
| libkiwix | github.com/kiwix/libkiwix | `c8a4b8aae68c1116ec09a49bcd487a342bb16529` | 2026-10-27 | Merge PR #1387; version 14.3.0 (meson) |
| kiwix-android | github.com/kiwix/kiwix-android | `8fd5b931c97ccd25043398dc654d938dc32c1977` | 2026-10-27 | Merge PR #5234 ("Modern" architecture) |
| kiwix-apple | github.com/kiwix/kiwix-apple | `f410135964333775373ce851a43eeb05c966dace` | 2026-10-23 | Merge PR #1799; **verified** the exact iOS repo ("Kiwix for iOS, iPadOS & macOS", Swift, GPL-3.0) |
| kiwix-build (tool) | github.com/kiwix/kiwix-build | `e7343a1640de5379c212eac2aec6ba32c9fd2d31` | 2026-10-08 | only used for feasibility probing |

Repo-layout warning (verified): at the pinned commit, kiwix-apple's `Kiwix/`
subdirectory contains only a storyboard; the Swift code lives in top-level
`App/`, `Model/`, `ViewModel/`, `Views/`, `SwiftUI/Model/`, `Support/`.

## 1. Session model — where the Kiwix apps keep continuation state

### 1.1 Android (kiwix-android @ 8fd5b93)

| State | Where | Mechanism | Citation |
|---|---|---|---|
| Current ZIM file | DataStore `kiwix_datastore_preferences`, key `"currentzimfile"` | Jetpack Preferences DataStore | `core/src/main/java/org/kiwix/kiwixmobile/core/utils/datastore/KiwixDataStore.kt` (companion, ~line 704 ff.); tag constants in `core/src/main/java/org/kiwix/kiwixmobile/core/utils/Constants.kt:31` (`TAG_CURRENT_FILE = "currentzimfile"`) |
| ZIM library on disk | libkiwix `Library` → `ZIMFiles/library.xml` | XML via JNI | `core/src/main/java/org/kiwix/kiwixmobile/core/dao/LibkiwixBookOnDisk.kt` |
| Current tab index | DataStore key `"currenttab"` | DataStore | `Constants.kt:32` (`TAG_CURRENT_TAB`), KiwixDataStore |
| Tab/reader session incl. **scroll** | Room DB `KiwixRoom.db`, `WebViewHistoryEntity` | `webViewBackForwardList` = `WebView.saveState(bundle)` BLOB + `webViewPosition = webView.scrollY` Int + `webViewIndex` | `core/.../main/reader/helper/ReaderSessionManager.kt:198-215` (save), `:139-148` (restore: `webView.scrollY = webViewHistoryItem.webViewCurrentPosition`); entity `core/.../dao/entities/WebViewHistoryEntity.kt`; DAO `core/.../dao/WebViewHistoryRoomDao.kt` |
| Tabs (in memory only) | `TabsManager` StateFlow — **not persisted** | | `core/.../main/reader/helper/TabsManager.kt` |
| History | Room, `HistoryRoomEntity` (`historyUrl`, `historyTitle`, `timeStamp`, `zimId`, `zimReaderSource`) | Room | `core/.../dao/entities/HistoryRoomEntity.kt`, `core/.../dao/HistoryRoomDao.kt`; written by `ReaderHistoryManager.saveHistory` (`core/.../reader/helper/ReaderHistoryManager.kt:35-54`) |
| Recent searches / notes | Room | Room | `RecentSearchRoomEntity` / `NotesRoomEntity` + DAOs |
| Bookmarks | **libkiwix JNI**, not Room: `Library`+`Manager` → `Bookmarks/bookmark.xml` + `library.xml` | XML | `core/.../dao/LibkiwixBookmarks.kt:94-128` (paths + init), `:166-196` (save → `library.addBookmark(Bookmark(){bookId,title,url,...})`) |
| Reading settings (text zoom, theme, back-to-top, TTS) | DataStore keys `"true_text_zoom"` (default 100), `"pref_dark_mode"`, `"pref_backtotop"`, `"pref_tts_speed"`, … | DataStore | `KiwixDataStore.kt:64-72, 704-760`; legacy SharedPreferences `"kiwix-mobile"` migrated by `SharedPreferenceToDatastoreMigrator.kt:29-71` |

Content pipeline (Android): the reader WebView uses **no HTTP server and no
ContentProvider** — `CoreWebViewClient.shouldInterceptRequest`
(`core/.../main/CoreWebViewClient.kt:121-137`) intercepts virtual
`https://kiwix.app/<entryPath>` URLs and answers with a `WebResourceResponse`
built from libzim JNI bytes. JNI wrapper: `ZimFileReader`
(`core/.../reader/ZimFileReader.kt:71-117`) holds `org.kiwix.libzim.Archive`
(from Maven artifact `org.kiwix:libkiwix:2.7.0`, `buildSrc/.../Versions.kt:77`);
`load()` (`ZimFileReader.kt:264-298`) = `getEntryByPath(...).getItem(true).data`
→ ByteArrayInputStream, with `directAccessInformation` streaming for
>1 MB uncompressed media. Hotspot "Zim Host" (`libkiwix Server`,
`app/.../webserver/KiwixServer.kt`) is unrelated to the reader.

### 1.2 iOS/macOS (kiwix-apple @ f410135)

| State | Where | Mechanism | Citation |
|---|---|---|---|
| ZIM file location + identity | CoreData `ZimFile` entity: `fileID: UUID` (unique), `fileURLBookmark: Binary` (security-scoped bookmark) | CoreData, store `DataModel` | `SwiftUI/Model/DataModel.xcdatamodeld/DataModel.xcdatamodel/contents` (ZimFile entity); `SwiftUI/Model/Database.swift` (`NSPersistentContainer(name: "DataModel")`); `Model/Entities/Entities.swift` (`fileURLBookmark`, `isDownloaded` predicate) |
| Current article + **scroll** + back/forward | CoreData `Tab` entity, attribute `interactionState: Binary` = serialized `WKWebView.interactionState` blob; `title`, `lastOpened: Date`, rel `zimFile` | CoreData | `DataModel.xcdatamodel/contents` (Tab entity, verified: `<attribute name="interactionState" optional="YES" attributeType="Binary"/>`); `BrowserViewModel.persistState()` (`ViewModel/BrowserViewModel.swift:313`) saves `webView.interactionState` → `tab.interactionState`; restore in `restoreBy(tabID:)` (`:381`); trigger on `.inactive`/`.onDisappear` (`Views/BrowserTab.swift:54-92`); most-recent-tab restore `Tab.fetchRequestLastOpened()` (`Model/Entities/Entities.swift`; `ViewModel/NavigationViewModel.swift` `navigateToMostRecentTabAsync`) |
| Scroll capture detail | scroll lives **inside** the `interactionState` blob; a JS `scrollHandler` message (`BrowserViewModel.swift:619-627`) is UI-only, not persisted | | same files |
| macOS window/tab layout | `Documents/session.json` (`WindowState`: `frame`, `selectedTabId`, `otherTabIds`) | JSON file | `Model/SessionRestore/SessionRestore.swift`, `App/App_macOS.swift:30-32,109` |
| Bookmarks | CoreData `Bookmark` entity (`articleURL: URI` unique, `title`, `created`, rel `zimFile`) | CoreData | `DataModel.xcdatamodel/contents` (Bookmark entity); `BrowserViewModel.createBookmark/deleteBookmark` (`:784-804`); UI `Views/Bookmarks.swift` |
| Reading settings | UserDefaults via `Defaults` SPM pkg: `webViewPageZoom` (`Key<Double>("webViewPageZoom")`), `webViewTextSizeAdjustFactor` (`"webViewZoomScale"`), `externalLinkLoadingPolicy`, `recentSearchTexts`; theme `@AppStorage("userColorScheme")` | UserDefaults | `SwiftUI/Model/DefaultKeys.swift:55-67`; `SwiftUI/Model/UserColorScheme.swift`; zoom applied `Views/BuildingBlocks/WebView.swift:81,210` |

Content pipeline (iOS): `Model/ZimFileService/ZimService.h/.mm` (Obj-C++)
wraps **libzim directly** (`#import "zim/archive.h"`, `zim/item.h`; verified
`item.getData()`/`item.getData(start, ...)` at `ZimService.mm:243-247`) plus
libkiwix only for `kiwix::Book` metadata/spellings. Delivery to `WKWebView` is
a custom **`zim://` URL scheme via `WKURLSchemeHandler`**
(`Model/Utilities/WebKitHandler.swift` — streams in 2 MB chunk ranges via
`rangesForDataStreaming`/`DataStream`), not an HTTP server; the only HTTP server
is the opt-in Hotspot (`Model/Hotspot/KiwixHotspot.mm` wrapping
`kiwix::Server`). Chunked + direct access: `ZimContentProvider.swift` has
`ZimContentProvider` (libzim `getURLContent(url:start:end:)`) and
`ZimDirectContentProvider` (bypasses libzim: `FileHandle.seek(toOffset:
directAccess.offset + start)`); media uses the direct path
(`WebKitHandler.swift:141`), everything else the libzim path.

**Cross-cutting reading of §1:** both apps already isolate exactly the
continuation set Carry-On needs — {zim file id/path, current article (URL/path
+ back/forward), scroll, bookmarks, history, settings} — in inspectable,
file-backed stores (Room/DataStore + libkiwix XML on Android; CoreData +
UserDefaults on iOS). OBSERVED (code-level, not yet executed on device).

## 2. libzim / libkiwix API — open, read, chunk

### 2.1 Opening and reading a specific article (libzim, all citation to pin `ad888b4`)

```cpp
zim::Archive archive("/path/to/wikipedia_en_all_maxi.zim");   // include/zim/archive.h:121
zim::Entry  entry = archive.getEntryByPath("A/Python_(programming_language)"); // archive.h:441
zim::Item   item  = entry.getItem(true /*follow redirects*/);  // include/zim/entry.h:57
zim::Blob   blob  = item.getData(offset, size);               // include/zim/item.h:63
uint64_t    n     = item.getSize();                           // item.h:69
std::string mime  = item.getMimetype();                       // item.h:44
```

Split ZIM files (`foo.zimaa/.zimab/...`) open transparently via the same ctor
(archive.h:107-120); fd-based ctors (archive.h:137-256, incl. `FdInput`
fd/offset/size segments for embedded archives) exist for Android SAF
(`AssetFileDescriptor`) and iOS security-scoped file coordination.

### 2.2 Offset / random access — the critical question

**Yes, at two distinct levels.** This is the hinge of the flagship: Carry-On can
transfer "just enough ZIM" progressively.

1. **Item-level chunked reads (decompressed):** `Item::getData(offset_type,
   size_type)` (include/zim/item.h:63, `getData(offset=0)` at :53) returns an
   arbitrary byte window of the item's *uncompressed* data — libzim decompresses
   the cluster on demand and caches it (`zim::setClusterCacheMaxSize`,
   include/zim/archive.h:45-62; kiwix-apple sets 16 MiB,
   `ZimService.mm:50`). **OBSERVED in production use**: kiwix-apple's
   `ZimService.mm:243-247` does exactly `item.getData(start, ...)` for HTTP-range
   style reads, and kiwix-android's `ZimFileReader.load()` supports Range
   headers the same way. Article sizes are KBs-to-MBs, so the adapter can serve
   `read_object(offset, length)` chunks straight out of this API.
2. **File-level raw access (uncompressed items):**
   `Item::getDirectAccessInformation()` (item.h:71-84) returns
   `{filename, offset}` into the ZIM file for items stored raw — documented for
   "reopening the file and reading the content bypassing the libzim". Both
   apps use it: kiwix-android `ZimFileReader.kt:283-295` (media streaming via
   `ParcelFileDescriptor`), kiwix-apple `ZimDirectContentProvider`
   (`FileHandle.seek(directAccess.offset + start)`). So for uncompressed
   entries, even *file byte ranges* are directly addressable.
3. **Archive-level identity/integrity:** `Archive::getUuid()`,
   `hasChecksum()`/`getChecksum()` (archive.h:638-652), `zim::validate(path,
   checks)` (archive.h:904-917), `getFilename()`, `getFilesize()` — everything
   needed to bind "same ZIM" identity in a cut without moving the 80 GB.

What libzim does **not** give: an offset hook *into a remote/foreign byte
store* — the `Archive` ctors want a local path or fd. Progressive ZIM-bulk
transfer therefore means a destination-side "sparse/loopback file materializer"
that fetches ZIM byte ranges over the Carry-On channel and feeds them to a
local file (libzim then reads it normally). The ZIM format makes this efficient
(see §3: header + tables first, clusters on demand — same trick the iOS
`zim://` handler uses inside one machine, extended across the wire).

### 2.3 libkiwix at this pin — no Reader class

At `c8a4b8a` libkiwix contains **no article-reading API**: reading is done in
libzim by both apps (§1). libkiwix supplies the *library* side:
`kiwix::Library`/`Manager` (library.xml, downloads), `kiwix::Book`
(`Book::update(const zim::Archive&)`, book.h:110), `kiwix::Bookmark`
(bookId/url/title/language/date — bookmark.h:59-75, the object the Android
app persists), `kiwix::Server` (HTTP hotspot/kiwix-serve), i18n, spelling
correction, OPDS. **Design consequence:** a Carry-On adapter that wants
"reading" needs **libzim** bindings; it needs **libkiwix** only to be
bookmark-compatible with the Android app's XML storage.

## 3. Continuation semantics with byte math

The asymmetry that makes Kiwix *the* progressive flagship: authoritative state
is {tiny session pointer}; the content library is an 80+ GB immutable,
checksummed, chunk-addressable object the destination may already own.

Objects for `carryon-adapter-kiwix` (v1 sketch; authoritative in
`ADAPTER-DESIGN.md`):

| object | role | size (order of magnitude) |
|---|---|---|
| `kiwix.session.v1` — {zim uuid + filename + size, article path/URL, scroll position, tab index} | **prerequisite, authoritative** | 10²–10³ bytes |
| `kiwix.article.v1` — current article bytes (HTML) | **immediate**, prerequisite for first paint without the ZIM | 10⁴–10⁶ bytes (typical wikipedia HTML 50–500 KB; cite your own ZIM when measured — sizes are ZIM-dependent, label any concrete number OBSERVED only after measuring) |
| `kiwix.bookmarks.v1` / `kiwix.history.v1` | optional | 10³–10⁵ bytes |
| `kiwix.settings.v1` (zoom/theme subset) | optional | 10¹–10² bytes |
| ZIM file (L1 content object) | **optional bulk, on demand** | up to ~10¹¹ bytes |

Byte math of progressive ZIM transfer (ZIM format constants from libzim source;
the *formulas* are PROVED by the format, concrete totals depend on the file):

- Header: exactly **80 bytes** (`src/fileheader.cpp:39`,
  `Fileheader::size = 80`; "This is also mimeListPos").
- Path pointer table: `articleCount × 8` bytes; cluster pointer table:
  `clusterCount × 8` bytes; title index: `articleCount × 4` (all offsets in
  header, `src/fileheader.h:52-76`). E.g. wikipedia EN (~6.5 M entries, ~10⁵
  clusters) → ~50 MB path+title tables + ~1 MB cluster table. Dirents add
  ~O(100 B) each → another few hundred MB for the full index. All ≪ 80 GB.
- One cluster (the unit of on-demand fetch; typical uncompressed cluster in
  wikipikis is 10⁵–10⁶ bytes, set at ZIM build time) is enough to serve the
  current article + its inline resources.

So the prerequisite tier of a handoff is `kiwix.session.v1` (KB) and — if the
destination lacks the ZIM — the immediate tier is `kiwix.article.v1` +
ZIM header + the two pointer tables + the cluster(s) touching the current
article (MB-scale, not GB-scale). The remaining ~10¹¹ bytes are **optional**
and fetched lazily as the user navigates (cluster-granularity), which is the
whole point: `bytes_before_first_action` is KBs–MBs vs 80+ GB for "send the
library".

Precedent inside Kiwix that chunked serving works at these granularities:
kiwix-apple's `zim://` scheme handler streams 2 MB chunk ranges
(`WebKitHandler.swift`), kiwix-android honors HTTP Range on virtual URLs
(`ZimFileReader.load`). OBSERVED at code level; carrying it across devices is
the adapter's Phase 2 work.

## 4. Feasibility verdicts (see BUILD-EVIDENCE.md for commands + log tails)

- libzim + libkiwix native build on this macOS host: **IMPLEMENTED** (logs).
- libzim cross-build `aarch64-linux-android` (NDK r27c, meson wraps, xapian
  off): **IMPLEMENTED** (elf64-littleaarch64 static lib in log).
- libkiwix for Android from this macOS host: **UNSUPPORTED so far** — meson
  fails at `icu-i18n` (Android ICU must itself be cross-compiled; kiwix-build's
  Android tooling downloads `android-ndk-r23c-linux.zip` —
  `kiwixbuild/dependencies/tc_android_ndk.py:17-20` — i.e. Linux-host only).
  Route for Phase 2: Linux CI container (as kiwix does) or porting the
  dependency set.
- libzim cross-build `aarch64-apple-ios` staticlib: **IMPLEMENTED**
  (mach-o arm64 static lib in log; no Xcode app needed this phase).
- libkiwix for iOS via kiwix-build on this Mac: blocked by kiwix-build's
  autoconf deps rejecting build paths containing spaces (our SSD volume is
  "ADATA ELITE SE880"); retrying via a space-free symlink — result in
  BUILD-EVIDENCE.md. Direct meson cross of libkiwix for iOS fails at
  `icu-i18n` (same class as Android).

## 5. Risks / open questions

- ZIM-identity binding: uuid + size + per-ZIM checksum via `Archive::getChecksum`
  is the natural "same library" test — verify checksum cost for 80 GB files is
  acceptable or use uuid+size+first/last-block digests. RESEARCH.
- The Android `WebView.saveState` Bundle blob and the iOS
  `WKWebView.interactionState` blob are **opaque** (platform versioned). The
  adapter must treat them as opaque portable-ish payloads (works across same
  app versions) but ALSO carry the explicit {article path, scrollY} fields for
  cross-version/cross-platform restore. Design handles both.
- GPL-3.0 (kiwix apps) / GPL-3.0 libzim — linking is fine for an adapter
  distributed alongside (needs legal review before any App Store distribution);
  Carry-On core stays clean via the adapter boundary. RESEARCH.
