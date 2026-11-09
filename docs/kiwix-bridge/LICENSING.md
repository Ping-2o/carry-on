# Kiwix builds and the libzim license

**Status: Proposed — needs owner sign-off** (issue #29). This is an engineering
record, not legal advice.

## Summary

The Carry-On engine is `MIT OR Apache-2.0`. The Kiwix backend
(`carryon-adapter-kiwix`, feature `zim-backend`) links
[libzim](https://github.com/openzim/libzim), which is GPL-2.0. The upstream
repository declares GPL-2.0; distributions such as Homebrew and Alpine package it
as GPL-2.0-or-later. Confirm the exact terms against `COPYING` and the source
headers at the pinned commit (`ad888b4cdc5c76d788e9e70b630842dc1b31871f`, 9.8.2,
see [`BUILD-EVIDENCE.md`](BUILD-EVIDENCE.md)) before any release.

Any binary that contains or links libzim is a combined work and is distributed
under the GPL's terms. Everything else stays permissive.

## Which artifacts include libzim

Rule of thumb: `carryon-ffi` built with `--features kiwix`, or
`carryon-adapter-kiwix` built with its default `zim-backend` feature, links
libzim.

| Artifact | Built by | libzim? | How it is linked |
|---|---|---|---|
| Default engine (`libcarryon_ffi` dylib / `.so` / staticlib) | `cargo build -p carryon-ffi` (`kiwix` off) | No | — |
| CI artifacts: debug APK, `carryon-kit` AAR, macOS app bundle, Swift SDK | `.github/workflows/ci.yml` | No | runners have no libzim; `carryon-adapter-kiwix` is checked with `--no-default-features` only |
| Android engine for the Kiwix companion reader | `CARRYON_KIWIX=1 apps/android/Scripts/fetch_engine_lib.sh` (`carryon-ffi --features kiwix`) | **Yes** | static `libzim.a` + liblzma + zstd inside `libcarryon_ffi.so`; any APK that bundles this `.so` includes libzim |
| macOS Kiwix companion driver `carryon_kiwix` | `apps/android/Scripts/build_kiwix_shell.sh` (`carryon-ffi --features kiwix`) | **Yes** | dynamic `libzim.dylib` from `$CARRYON_LIBZIM_LIBDIR` |
| `carryon-adapter-kiwix` tests and examples (default features) | `cargo test -p carryon-adapter-kiwix` | **Yes** | dynamic on the macOS host; static with `CARRYON_LIBZIM_STATIC=1` (`build.rs`) |

Dynamic linking does not take a binary out of scope: the FSF treats a program
that links a GPL library as a combined work either way.

## Other licenses in the libzim build

- **liblzma** (xz 5.2.11, public domain) and **zstd** (1.5.4, BSD-3-Clause OR
  GPL-2.0): built from the meson fallbacks and linked statically into the Android
  Kiwix engine (`BUILD-EVIDENCE.md` §B).
- **xapian** (GPL-2.0-or-later) and **ICU** (Unicode-3.0): only in the macOS host
  libzim, which was built with `-Dwith_xapian=true` (`BUILD-EVIDENCE.md` §A). The
  Android build has xapian off.
- **libc++_shared.so** from the NDK (Apache-2.0 WITH LLVM-exception): shipped next
  to the Kiwix `.so` by `fetch_engine_lib.sh`.

## What keeps the default artifacts permissive

- The `kiwix` feature of `carryon-ffi` is off by default, so the plain engine does
  not even compile `carryon-adapter-kiwix`.
- CI never builds with libzim and excludes `carryon-adapter-kiwix` from the
  workspace jobs.
- `deny.toml` keeps GPL / LGPL / AGPL out of the Rust dependency graph.
  **cargo-deny does not see libzim**: it is a C++ library linked from `build.rs`,
  outside Cargo. This file is the guard for that gap.

## Compatibility notes for a GPL Kiwix artifact

- Carry-On is dual-licensed, and recipients can take it under MIT, which is
  compatible with GPL-2.0. Apache-2.0 on its own is compatible with GPL-3.0 but not
  with GPL-2.0-only.
- `deny.toml` allows Apache-2.0-only and MPL-2.0 crates. If libzim is
  GPL-2.0-only, every Apache-2.0-only crate in the `--features kiwix` graph is an
  open compatibility question. If it is GPL-2.0-or-later, the combined work can be
  distributed under GPL-3.0. Check the real release graph with
  `cargo deny --features carryon-ffi/kiwix list` before shipping.
- A GPL release must ship with the license texts and the corresponding source:
  Carry-On at the release tag (including the shim in
  `crates/carryon-adapter-kiwix/shim/`), libzim and its dependencies at the pinned
  commits, and the build scripts.

## Decision

| Option | What it means | Status |
|---|---|---|
| A. Opt-in, separate GPL artifacts | Default releases never enable `kiwix`. Kiwix builds, if released, are separate artifacts with a `-kiwix` suffix, labeled GPL, with license texts and a source offer | **Recommended** |
| B. Opt-in, development only | No Kiwix binary is distributed; the companion reader stays a local / evidence build | Applies until sign-off |
| C. Kiwix in the default apps | Every app release becomes a GPL combined work | Rejected |

Proposed policy:

1. The release jobs (#21) and the SDK publishing (#22) never pass
   `--features kiwix` or `CARRYON_KIWIX=1`. Their artifacts stay
   `MIT OR Apache-2.0`.
2. A Kiwix release, if one is ever made, is a separate artifact (for example
   `carry-on-android-kiwix-<version>.apk`) with its own notes, and only after the
   license terms and the compatibility check above are confirmed.
3. Until the owner signs off, option B applies: no Kiwix binaries are distributed.

Owner sign-off: **pending**.
