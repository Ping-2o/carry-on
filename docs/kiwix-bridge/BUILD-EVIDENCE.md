# Kiwix bridge — Phase 1 build feasibility evidence

Environment: macOS host (arm64, Apple clang, SDK iPhoneOS 27.0/Xcode-beta),
meson 1.11.2, ninja 1.13.2, Android NDK r27c at
`/Volumes/ADATA ELITE SE880/compilation/android-ndk-r27c`. Heavy artifacts on
the external SSD (`/Volumes/ADATA ELITE SE880/compilation/`) per project rules.
Labels: each verdict below cites the exact command and its observed output.

## A. libzim + libkiwix native build on the macOS host — IMPLEMENTED

```sh
export PKG_CONFIG_PATH="$(brew --prefix icu4c@79)/lib/pkgconfig:$PKG_CONFIG_PATH"
brew install xapian pugixml libmicrohttpd     # icu4c@79 already present

cd "/Volumes/ADATA ELITE SE880/compilation/libzim"
meson setup build -Dwith_xapian=true           # xapian 2.1.0, icu-i18n 79.1 via pkg-config
ninja -C build
# tail:
#   [201/201] Linking target test/search_iterator
meson configure build -Dprefix="/Volumes/ADATA ELITE SE880/compilation/prefix/host" && meson install -C build

# libkiwix needs the kainjow mustache single header (meson errors without it):
curl -sL -o ".../prefix/host/include/mustache.hpp" \
  https://raw.githubusercontent.com/kainjow/mustache/master/mustache.hpp

cd "/Volumes/ADATA ELITE SE880/compilation/libkiwix"
export PKG_CONFIG_PATH="/Volumes/ADATA ELITE SE880/compilation/prefix/host/lib/pkgconfig:$PKG_CONFIG_PATH"
meson setup build                              # libzim 9.8.2 (from our prefix), libkiwix 14.3.0
ninja -C build
# tail:
#   [124/124] Linking target test/server_search
```

## B. libzim → aarch64-linux-android (NDK r27c) — IMPLEMENTED

Cross-file (full file at `crates/carryon-adapter-kiwix/cross/android-arm64-cross.txt`
in this branch): NDK r27c clang wrappers, API 24, sysroot from the NDK, static.

```sh
cd "/Volumes/ADATA ELITE SE880/compilation/libzim"
PKG_CONFIG_PATH= meson setup build-android \
  --cross-file android-arm64-cross.txt \
  -Dwith_xapian=false -Ddefault_library=static -Dbuildtype=release \
  --wrap-mode=forcefallback
```

`--wrap-mode=forcefallback` is essential: without it meson finds the *host*
homebrew liblzma/zstd via pkg-config and links them into the Android artifact.
With it, liblzma (xz 5.2.11) and zstd (1.5.4) build from source as meson
subprojects with the NDK compiler.

First `ninja -C build-android` ended with:

```
ld.lld: error: undefined symbol: LZ4F_freeCompressionContext
>>> referenced by fileio.c (zstd CLI program, not the library)
clang: error: linker command failed with exit code 1
```

The zstd *subproject* also builds the zstd CLI, which needs lz4 (absent for
Android). The library itself was already built. Fix and result:

```sh
meson configure build-android -Dzstd:bin_programs=false -Dzstd:bin_tests=false
ninja -C build-android   # -> ninja: no work to do. (all targets complete)
file build-android/src/libzim.a
#   build-android/src/libzim.a: current ar archive
llvm-objdump -a build-android/src/libzim.a | head -3
#   file format elf64-littleaarch64   (x N objects)
```

Verdict: **libzim.a for Android arm64 built from this macOS host with NDK
r27c** (xapian off; full-text search would need xapian+icu cross-compiled —
see D). ~1.7 MB static archive.

## C. libzim → aarch64-apple-ios (staticlib, no Xcode app) — IMPLEMENTED

Cross-file (kept at `crates/carryon-adapter-kiwix/cross/ios-arm64-cross.txt`):
`clang -arch arm64 -isysroot $(xcrun --sdk iphoneos --show-sdk-path)
-mios-version-min=15.0`, static.

```sh
cd "/Volumes/ADATA ELITE SE880/compilation/libzim"
PKG_CONFIG_PATH= meson setup build-ios \
  --cross-file ios-arm64-cross.txt \
  -Dwith_xapian=false -Ddefault_library=static -Dbuildtype=release \
  --wrap-mode=forcefallback -Dzstd:bin_programs=false
ninja -C build-ios
# tail:
#   [164/166] Compiling C++ object src/libzim.a.p/writer_creator.cpp.o
#   [165/166] Linking static target src/libzim.a
#   [166/166] Linking target examples/createZimExample
file build-ios/src/libzim.a
#   build-ios/src/libzim.a: current ar archive
xcrun llvm-objdump -a build-ios/src/libzim.a | grep "file format" | head -3
#   file format mach-o arm64   (x N objects)
```

## D. libkiwix cross-builds — honest negatives from this host

**Android (direct meson):**

```
Run-time dependency icu-i18n found: NO  (tried pkg-config)
meson.build:52:13: ERROR: Dependency "icu-i18n" not found (tried pkg-config)
```

libkiwix hard-requires icu-i18n, pugixml, libcurl, libmicrohttpd, xapian-core
(`meson.build:52-63`); none exist as Android binaries on a macOS host — each
must itself be cross-compiled first. kiwix's own tool for this
(kiwix-build @ e7343a1) **downloads `android-ndk-r23c-linux.zip`**
(`kiwixbuild/dependencies/tc_android_ndk.py:17-20`), i.e. its Android builds
run on Linux hosts/CI, not macOS. Phase 2 route: Linux container (as kiwix CI
does) or hand-port the dependency set. The adapter design deliberately needs
only **libzim** for reading; libkiwix is optional (bookmark-XML compatibility
only), so this negative does not block the flagship.

**iOS (direct meson):** same class of failure — no iOS-built icu/xapian/curl/
microhttpd present locally; `meson setup` fails at `icu-i18n` before compiling
anything. kiwix-build is the canonical iOS path and does run on macOS; its
first two runs here died on **autoconf's space-in-path check** (our SSD volume
is `ADATA ELITE SE880`):

```
configure: error: You can't build in a directory whose path contains a space
```

(direct path and symlinked path both fail — autoconf resolves the physical
directory). Workaround attempt for the canonical path: an APFS sparse-bundle
image stored on the SSD and mounted at the space-free mount point
`/Volumes/kbwork`, then `python -m kiwixbuild --config ios_arm64
--working-dir /Volumes/kbwork/out --hide-progress libzim` — result appended
below in §F when it finishes.

## E. Verified toolchain pins

- libzim `ad888b4cdc5c76d788e9e70b630842dc1b31871f` (9.8.2)
- libkiwix `c8a4b8aae68c1116ec09a49bcd487a342bb16529` (14.3.0)
- kiwix-android `8fd5b931c97ccd25043398dc654d938dc32c1977`
- kiwix-apple `f410135964333775373ce851a43eeb05c966dace`
- kiwix-build (tool only) `e7343a1640de5379c212eac2aec6ba32c9fd2d31`

## F. kiwix-build iOS result — IMPLEMENTED (canonical path, after a path fix)

`python -m kiwixbuild --config ios_arm64 --working-dir ... --hide-progress libzim`
(kiwix-build @ `e7343a1`, venv with `distro` installed; PEP-668 blocks the
system python) — first runs failed on autoconf's space-in-path check (§D).
Fixed by keeping the working dir inside an APFS sparse-bundle image stored on
the SSD and mounted at the space-free `/Volumes/kbwork`. Retry succeeded:

```
configure icu4c : OK (14.0s)
compile icu4c : ....OK (130.1s)
install icu4c : OK (1.4s)
build zim-testing-suite (ios_arm64):
build libzim (ios_arm64):
  configure libzim : OK (11.0s)
  compile libzim : OK (10.6s)
  test libzim : OK (0.3s)
  install libzim : OK (0.3s)
```

Cross-built dependency set at `/Volumes/kbwork/out/SOURCE/`: icu4c-73.2,
xapian-core-1.4.26, lzma-5.2.6, zstd-1.5.7, zlib-1.3.1,
zim-testing-suite-0.10.0 — i.e. the **full-text-search (xapian + icu)**
build that the manual §C build (xapian off) skips. Source pin verified:
kiwix-build cloned libzim at `ad888b4cdc5c76d788e9e70b630842dc1b31871f` —
the exact commit we pinned in §E.

```
/Volumes/kbwork/out/BUILD_ios_arm64/INSTALL/lib/libzim.a:
  file format mach-o arm64   (x N objects)
```

Verdict: the canonical kiwix-build iOS toolchain runs on this macOS host,
needs only a space-free working dir (sparse image on the SSD), and produces
an `ios_arm64` static libzim with xapian. This is the same machinery that
produces the `CoreKiwix.xcframework` kiwix-apple consumes; Phase 2 can
extend the same command to `libkiwix` (kiwix-build supports it for iOS
configs directly).

