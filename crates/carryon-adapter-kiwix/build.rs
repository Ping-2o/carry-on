// build.rs — compiles the C++ shim over libzim and links libzim.
//
// Feature `zim-backend` (default): real libzim backend via FFI.
//   - macOS host: links the shared libzim installed at
//     $CARRYON_LIBZIM_LIBDIR (default: the Phase-1 prefix on the SSD).
//   - Cross targets: link a static libzim; set CARRYON_LIBZIM_STATIC=1,
//     CARRYON_LIBZIM_LIBDIR and CARRYON_LIBZIM_DEP_LIBDIRS (colon-separated).
//     Evidence per target in docs/kiwix-bridge/BUILD-EVIDENCE.md.
// Without the feature the crate is pure Rust and every ZIM-dependent path
// fails closed (Incompatible) — no fabricated bytes, ever.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=shim/czim.cpp");
    println!("cargo:rerun-if-changed=shim/czim.h");
    println!("cargo:rerun-if-env-changed=CARRYON_LIBZIM_LIBDIR");
    println!("cargo:rerun-if-env-changed=CARRYON_LIBZIM_STATIC");
    println!("cargo:rerun-if-env-changed=CARRYON_LIBZIM_DEP_LIBDIRS");
    if env::var_os("CARGO_FEATURE_ZIM_BACKEND").is_none() {
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let libdir = env::var("CARRYON_LIBZIM_LIBDIR")
        .unwrap_or_else(|_| "/Volumes/ADATA ELITE SE880/compilation/prefix/host/lib".to_string());
    let incdir = env::var("CARRYON_LIBZIM_INCDIR").unwrap_or_else(|_| {
        "/Volumes/ADATA ELITE SE880/compilation/prefix/host/include".to_string()
    });

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file(manifest.join("shim/czim.cpp"))
        .include(manifest.join("shim"))
        .include(&incdir)
        .flag_if_supported("-std=c++17")
        .warnings(true);
    // The cross cases drive the compiler through the standard
    // CC_aarch64_linux_android / CC_aarch64_apple_ios style env vars.
    build.compile("czim");

    println!("cargo:rustc-link-search=native={libdir}");
    if env::var_os("CARRYON_LIBZIM_STATIC").is_some() {
        println!("cargo:rustc-link-lib=static=zim");
        // Static libzim needs its compression deps; extra search paths let
        // the caller point at cross-built lzma/zstd/icu/xapian.
        if let Ok(deps) = env::var("CARRYON_LIBZIM_DEP_LIBDIRS") {
            for dir in deps.split(':') {
                if !dir.is_empty() {
                    println!("cargo:rustc-link-search=native={dir}");
                }
            }
        }
    } else {
        println!("cargo:rustc-link-lib=dylib=zim");
    }
}
