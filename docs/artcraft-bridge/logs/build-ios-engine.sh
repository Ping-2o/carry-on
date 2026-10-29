#!/bin/bash
# iOS cross-build attempt: photocraft-engine (lib) for aarch64-apple-ios
source "$HOME/.cargo/env"
# separate target dir so this can run concurrently with the android build
export CARGO_TARGET_DIR="/Volumes/ADATA ELITE SE880/compilation/target-ios"
cd "/Volumes/ADATA ELITE SE880/compilation/photocraft"
echo "=== rustc: $(rustc --version)"
echo "=== command: cargo build -p photocraft-engine --target aarch64-apple-ios"
cargo build -p photocraft-engine --target aarch64-apple-ios 2>&1
echo "=== EXIT: $?"
