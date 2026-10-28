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
    // One or more include roots, colon-separated (the generated zim_config.h
    // may live in the cross-build dir rather than the source tree).
    let incdirs = env::var("CARRYON_LIBZIM_INCDIR").unwrap_or_else(|_| {
        "/Volumes/ADATA ELITE SE880/compilation/prefix/host/include".to_string()
    });

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file(manifest.join("shim/czim.cpp"))
        .include(manifest.join("shim"));
    for dir in incdirs.split(':') {
        if !dir.is_empty() {
            build.include(dir);
        }
    }
    build.flag_if_supported("-std=c++17").warnings(true);
    // The cross cases drive the compiler through the standard
    // CC_aarch64_linux_android / CC_aarch64_apple_ios style env vars.
    build.compile("czim");

    println!("cargo:rustc-link-search=native={libdir}");
    if env::var_os("CARRYON_LIBZIM_STATIC").is_some() {
        // Static libzim carries its whole dep chain in the link line — order
        // matters to lld: libzim first, then lzma/zstd, and the C++ runtime
        // (NDK libc++_static) for the shim's exceptions/rtti.
        println!("cargo:rustc-link-lib=static=zim");
        println!("cargo:rustc-link-lib=static=lzma");
        println!("cargo:rustc-link-lib=static=zstd");
        println!("cargo:rustc-link-lib=c++_static");
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
