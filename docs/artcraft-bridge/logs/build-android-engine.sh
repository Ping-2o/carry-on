#!/bin/bash
# Android cross-build attempt: photocraft-engine (lib) for aarch64-linux-android
# NDK reached through the space-free symlink ~/ndk-r27c (RUSTFLAGS is whitespace-split,
# so the "ADATA ELITE SE880" path cannot appear in it; first attempt failed on this).
source "$HOME/.cargo/env"
export CARGO_TARGET_DIR="/Volumes/ADATA ELITE SE880/compilation/target"
NDK_TOOLCHAIN="$HOME/ndk-r27c/toolchains/llvm/prebuilt/darwin-x86_64"
export CC_aarch64_linux_android="$NDK_TOOLCHAIN/bin/aarch64-linux-android24-clang"
export CXX_aarch64_linux_android="$NDK_TOOLCHAIN/bin/aarch64-linux-android24-clang++"
export AR_aarch64_linux_android="$NDK_TOOLCHAIN/bin/llvm-ar"
export RANLIB_aarch64_linux_android="$NDK_TOOLCHAIN/bin/llvm-ranlib"
export RUSTFLAGS="-C linker=$NDK_TOOLCHAIN/bin/aarch64-linux-android24-clang"
cd "/Volumes/ADATA ELITE SE880/compilation/photocraft"
echo "=== rustc: $(rustc --version)"
echo "=== command: cargo build -p photocraft-engine --target aarch64-linux-android"
cargo build -p photocraft-engine --target aarch64-linux-android 2>&1
echo "=== EXIT: $?"
