# PhotoCraft mobile feasibility — honest verdict with evidence

> Labels: build results below are **OBSERVED** (LOCAL cross-compile — no device
> ran anything); GUI/mobile verdicts are reasoned from cited code and are
> **RESEARCH** unless marked otherwise. Nothing here claims a platform is
> *supported* (no §30 acceptance, no on-device run). PhotoCraft pin
> `1f7fc0f5…` ([`PHOTOCRAFT-PIN.md`](PHOTOCRAFT-PIN.md)).
> Raw logs: [`logs/`](logs/).

Host: macOS arm64 (Apple Silicon), rustc `1.99.0 (b940084d7 2026-09-28)`,
rustup targets `aarch64-linux-android` + `aarch64-apple-ios`, Android NDK
**r27c** (`~/ndk-r27c` → `/Volumes/ADATA ELITE SE880/compilation/android-ndk-r27c`,
API level 24), Xcode iOS SDK `iPhoneOS27.0`. All build artifacts on the
external SSD (`CARGO_TARGET_DIR`).

## 1. Android (`aarch64-linux-android`) — engine: **OBSERVED: BUILDS**

Command (script `build-android-engine.sh`, log
[`logs/android-engine.log`](logs/android-engine.log)):

```sh
export CARGO_TARGET_DIR="/Volumes/ADATA ELITE SE880/compilation/target"
NDK_TOOLCHAIN="…/android-ndk-r27c/toolchains/llvm/prebuilt/darwin-x86_64"
export CC_aarch64_linux_android="$NDK_TOOLCHAIN/bin/aarch64-linux-android24-clang"
export CXX_aarch64_linux_android="$NDK_TOOLCHAIN/bin/aarch64-linux-android24-clang++"
export AR_aarch64_linux_android="$NDK_TOOLCHAIN/bin/llvm-ar"
export RANLIB_aarch64_linux_android="$NDK_TOOLCHAIN/bin/llvm-ranlib"
export RUSTFLAGS="-C linker=$NDK_TOOLCHAIN/bin/aarch64-linux-android24-clang"
cargo build -p photocraft-engine --target aarch64-linux-android
```

(The NDK is reached through the space-free symlink `~/ndk-r27c` because
`RUSTFLAGS` is whitespace-split; the first attempt with the raw
"`ADATA ELITE SE880`" path died instantly with
`error: multiple input filenames provided (first two filenames are '-' and 'ELITE')`.
Recorded here so nobody repeats it.)

Log tail (`logs/android-engine.log`):

```text
   Compiling photocraft-engine v0.16.0 (/Volumes/ADATA ELITE SE880/compilation/photocraft/crates/engine)
    Finished `dev` profile [optimized + debuginfo] target(s) in 45m 15s
=== EXIT: 0
```

213 crates compiled, zero errors. Artifact:
`target/aarch64-linux-android/debug/libphotocraft_engine.rlib` (178 MiB);
`llvm-ar` + `file` on an extracted member:
`ELF 64-bit LSB relocatable, ARM aarch64` — a genuine Android-arch object,
not a host artifact.

## 2. iOS (`aarch64-apple-ios`) — engine: **OBSERVED: BUILDS**

Command (script `build-ios-engine.sh`, log
[`logs/ios-engine.log`](logs/ios-engine.log)): plain
`cargo build -p photocraft-engine --target aarch64-apple-ios` (no linker
needed for an rlib; the task asked for a staticlib-level proof, i.e. "the
code compiles and archives for the target", which a lib build gives; a real
`.a` for an app would need a tiny staticlib shim crate in Phase 2).

Log tail (`logs/ios-engine.log`):

```text
   Compiling photocraft-engine v0.16.0 (/Volumes/ADATA ELITE SE880/compilation/photocraft/crates/engine)
    Finished `dev` profile [optimized + debuginfo] target(s) in 46m 03s
=== EXIT: 0
```

Artifact member check: `Mach-O 64-bit object arm64`.

## 3. CLI (`photocraft-cli` binary) for both targets — **OBSERVED: BUILDS**

The full headless CLI (engine + automation/MCP + tokio + clap + all codec
features) links as a complete binary for both mobile targets.

**3.1 Android** (script `build-android-cli.sh`, same NDK env as §1; log
[`logs/android-cli.log`](logs/android-cli.log)):

```text
    Finished `dev` profile [optimized + debuginfo] target(s) in 57m 09s
=== EXIT: 0
```

Artifact: `target/aarch64-linux-android/debug/photocraft-cli` (78 MiB) —
`file`:

```text
ELF 64-bit LSB pie executable, ARM aarch64, dynamically linked,
interpreter /system/bin/linker64, with debug_info, not stripped
```

That is a real Android executable (PIE, bionic loader) — untested on a
device, but built and linked for it. A `--target aarch64-linux-android`
build of `serve`/`mcp` is the whole Carry-On-relevant surface, no GUI needed.

**3.2 iOS** (script `build-ios-cli.sh`; log
[`logs/ios-cli.log`](logs/ios-cli.log)):

```text
    Finished `dev` profile [optimized + debuginfo] target(s) in 56m 54s
=== EXIT: 0
```

Artifact: `target-ios/aarch64-apple-ios/debug/photocraft-cli` (267 MiB) —
`Mach-O 64-bit executable arm64`. This is the full CLI binary, one step past
the "staticlib is enough" bar of this phase (a lib rlib was already proven in
§2). Note: an iOS executable still cannot ship without an Xcode app shell
(embedded frameworks, signing, Info.plist) — that shell is exactly the missing
piece §4 describes.

Both CLI builds ran concurrently on the host (sharing cores), 57m each, ~260
crates compiled per target including tokio/rmcp/clap and the full codec set.
No target-specific patches were needed: **stock PhotoCraft at pin `1f7fc0f`
cross-compiles for Android (NDK r27c, API 24) and iOS with nothing but the
documented CC/AR/RUSTFLAGS environment** — and no C/C++ dependency required
porting (the pure-Rust rule pays off: zstd/zlib via `zlib-rs`, blake3 `pure`,
flate2 `runtime_detection` per the workspace `Cargo.toml`).

## 4. The GUI's mobile story — verdict

**Verdict: native PhotoCraft GUI on iOS/Android is NOT realistic in months;
the realistic mobile path for the editor UX is the WASM web build in a
browser tab. The realistic mobile path for a *Carry-On adapter* is the
headless engine, which the logs above show cross-compiles today.**

Evidence, layer by layer:

- **Project platform matrix**: macOS, Windows, Linux, FreeBSD, web
  (`ROADMAP.md` Platforms row; `docs/scorecard.md` Distribution table
  DIST-1..6). No iOS/Android row, roadmap item, or issue at pin. The README
  badge says `macOS · Windows · Linux · FreeBSD · Web`.
- **App shell**: `apps/photocraft` (desktop) is winit+eframe+wgpu with heavy
  desktop-only wiring — AppKit (`objc2-app-kit`, `muda`, `core-graphics2`),
  X11 (`x11rb`), Wayland (`wl-clipboard-rs`), `rfd`, `arboard`
  (`apps/photocraft/Cargo.toml`, all `[target.'cfg(...)']`-gated). There is
  **no Android app** (no Gradle project, no `android-game-activity` entry
  point, no `AndroidManifest.xml`) and **no iOS app** (no Xcode project, no
  `Info.plist`, no UIKit shell) anywhere in the tree. The other shells are
  `apps/photocraft-web` (wasm32) and `apps/photocraft-cli` (headless).
- **Toolkit layer — the capability exists but is experimental**: the pinned
  egui fork (rev `6d518fc`) ships `eframe` with `android-game-activity` /
  `android-native-activity` features (`crates/eframe/Cargo.toml` of the fork;
  `android-activity v0.6.1` in `Cargo.lock`), and carries iOS accommodations
  in `eframe/src/native/run.rs`, `egui-winit`, and `egui-wgpu` (e.g. iOS
  frame-latency comment "The default is good on iOS, while `Some(1)` cuts
  FPS in half"). So the windowing/UI layer *could* run on Android (and
  partially on iOS) — but nothing above it exists for mobile.
- **Platform services**: fonts, file dialogs, clipboard, pen/tablet are
  function hooks injected by each app shell
  (`docs/architecture.md` §2 "What exists today": "`platform` services are
  function hooks injected by each app"). `photocraft-tablet` is macOS/X11
  only; system-font access and persistence would need new per-OS
  implementations (the web build shows the shape of that work: bundled
  fonts, `localStorage` — `docs/control-protocol.md` lines 264–289).
- **Performance reality**: 22 of 25 performance budgets are already missed on
  an M4 Pro with a desktop GPU (`ROADMAP.md` "Why alpha", `scorecard.md`);
  the GPU compositor targets parity ≤1/255 of the CPU oracle. Phone
  GPUs/thermals/memory are far below the machines those budgets assume. A
  native mobile GUI would fight both PhotoCraft's own desktop-first
  performance debt and mobile hardware limits — this is a multi-quarter
  effort *for the PhotoCraft project itself*, whose roadmap has no mobile
  item at all.
- **WASM web build is real and shipped**: DIST-5 "Web build (wasm32, trunk)
  done" (`docs/scorecard.md`); engine must build for `wasm32-unknown-unknown`
  per the enforced wasm gate (`docs/architecture.md` §3 rule 4, line 184);
  `apps/photocraft-web/index.html` carries a mobile viewport meta tag
  (`width=device-width, initial-scale=1, user-scalable=no`). **Not tested
  here**: whether the web build is actually usable on a phone browser (no
  mobile-device run in this phase — the honest statement is "shipped for
  browsers, mobile-browser usability UNSUPPORTED/UNTESTED").

What this means for Carry-On specifically (the reason this verdict matters):

- A Carry-On destination on Android/iOS can embed the **headless engine**
  (`photocraft_engine::Session`, RESEARCH §2.5) — or even run the full
  `photocraft-cli serve`/`mcp` binary, which the logs above show **links
  completely** for both mobile targets with stock toolchains. An L3/L4
  "carry the document + unsaved edits + continue headless or render previews"
  story on mobile needs no PhotoCraft GUI.
- The **GUI-on-mobile** story would additionally need: a new app shell
  (GameActivity/Xcode project), platform services (fonts, persistence,
  input), GPU/mobile performance work, and §30 acceptance on real devices —
  every one of those is undone, and only the toolkit layer (egui fork)
  shows signs of readiness.
- The **WASM-in-a-browser-tab** path exists today for the editor UX and is
  the only honest answer to "edit on a phone" — but per the frontier rules
  (FRONTIER.md §1) a browser tab is not a §30 platform, and a §30 story
  would need the native headless engine (which builds) rather than the web
  build.
