#!/usr/bin/env bash
# build_desktop_shim.sh — build the JNI shim for the HOST JVM (macOS/Linux) so
# the SDK's contract tests can exercise the REAL engine without an Android
# device. The shim is the same carryon_jni.c the AAR ships; only the engine
# binary differs (host cdylib instead of the cross-compiled .so).
#
#   apps/android/Scripts/build_desktop_shim.sh
#
# Output (default: apps/android/carryon-kit/build/desktop-lib):
#   libcarryon_ffi.{dylib,so}   host engine (cargo build -p carryon-ffi --release)
#   libcarryon_jni.{dylib,so}   JNI shim (same source as the AAR)
#
# Contract tests then run with:
#   ./gradlew :carryon-kit:testDebugUnitTest \
#       -PcarryonDesktopLibDir=carryon-kit/build/desktop-lib
#
# Labels: this is TEST HARNESS scaffolding — running the engine on the host
# JVM proves the Kotlin binding contract, not an Android platform claim (§2/§30).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ANDROID="$(cd "$HERE/.." && pwd)"
REPO="${REPO:-$(cd "$APP_ANDROID/../.." && pwd)}"
KIT="$APP_ANDROID/carryon-kit"
OUT="${OUT:-$KIT/build/desktop-lib}"

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi
: "${CARGO_TARGET_DIR:=/Volumes/ADATA ELITE SE880/compilation/target}"
export CARGO_TARGET_DIR

if [ -z "${JAVA_HOME:-}" ]; then
    if command -v /usr/libexec/java_home >/dev/null 2>&1; then
        JAVA_HOME="$(/usr/libexec/java_home 2>/dev/null || true)"
    fi
fi
[ -n "${JAVA_HOME:-}" ] && [ -d "$JAVA_HOME/include" ] || {
    echo "FAIL: JAVA_HOME must point at a JDK with JNI headers (set JAVA_HOME)" >&2
    exit 1
}

echo ">> building host engine (cargo build -p carryon-ffi --release)"
(CARGO_TARGET_DIR="$CARGO_TARGET_DIR" cargo build --manifest-path "$REPO/Cargo.toml" -p carryon-ffi --release)

mkdir -p "$OUT"
case "$(uname -s)" in
    Darwin)  EXT=dylib; JNI_OS=darwin;  SHARED="-dynamiclib" ;;
    Linux)   EXT=so;    JNI_OS=linux;   SHARED="-shared -fPIC" ;;
    *) echo "FAIL: unsupported host $(uname -s)" >&2; exit 1 ;;
esac

ENGINE="$CARGO_TARGET_DIR/release/libcarryon_ffi.$EXT"
[ -f "$ENGINE" ] || { echo "FAIL: engine not built at $ENGINE" >&2; exit 1; }
cp -f "$ENGINE" "$OUT/"

echo ">> building desktop JNI shim"
${CC:-cc} $SHARED -O2 -Wall -Wextra \
    -I "$REPO/crates/carryon-ffi/include" \
    -I "$JAVA_HOME/include" -I "$JAVA_HOME/include/$JNI_OS" \
    "$KIT/src/main/cpp/carryon_jni.c" \
    -L "$OUT" -lcarryon_ffi \
    -o "$OUT/libcarryon_jni.$EXT"

echo ">> OK: $OUT"
ls -l "$OUT"
