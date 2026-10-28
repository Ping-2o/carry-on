#!/bin/bash
# iOS cross-build attempt 2: photocraft-cli (bin) for aarch64-apple-ios
source "$HOME/.cargo/env"
export CARGO_TARGET_DIR="/Volumes/ADATA ELITE SE880/compilation/target-ios"
cd "/Volumes/ADATA ELITE SE880/compilation/photocraft"
echo "=== rustc: $(rustc --version)"
echo "=== command: cargo build -p photocraft-cli --target aarch64-apple-ios"
cargo build -p photocraft-cli --target aarch64-apple-ios 2>&1
echo "=== EXIT: $?"
