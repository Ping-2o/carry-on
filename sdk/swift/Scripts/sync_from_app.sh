#!/usr/bin/env bash
# sync_from_app.sh — sync the distributable SDK's C shim, header, and binding
# sources from the app package (apps/macos), with drift checks.
#
# The SDK package carries COPIES of the binding sources: apps/macos stays the
# root package that owns the app shell, and sdk/swift is the distribution
# surface built from the exact same sources. The copy is deliberate (the app
# package uses unsafeFlags for its dynamic dylib — legal for a root package,
# forbidden for a dependency — while the SDK links the static engine archive);
# this script + the ABI compatibility suite keep the copy honest.
#
#   sdk/swift/Scripts/sync_from_app.sh [--check]
#
# --check only verifies drift (CI mode): exit 1 if any file would change.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SDK="$(cd "$HERE/.." && pwd)"
REPO="${REPO:-$(cd "$SDK/../.." && pwd)}"
APP_KIT="$REPO/apps/macos/Sources/CarryOnKit"
CC_APP="$REPO/apps/macos/Sources/CCarryon"
CC_SDK="$SDK/Sources/CCarryon"
KIT_SDK="$SDK/Sources/CarryOnKit"

# The SDK binding = the app binding, minus the app-internal Console/Models that
# the app's own views import (they are app-facing; the SDK keeps the pipeline
# types they expose). Keep in sync with the drift check below.
SYNC_SOURCES=(
  AnyJSON.swift CarryonCore.swift CarryonError.swift DemoConsole.swift
  DemoPipeline.swift Discovery.swift EngineDiscovery.swift EvidenceModels.swift
  Handoff.swift Net.swift Pairing.swift SecureStore.swift
  PhotoCraft.swift
)

CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

fail=0
sync_file() {
  local src="$1" dst="$2"
  if [ ! -f "$src" ]; then echo "FAIL: missing source $src"; fail=1; return; fi
  if [ "${3:-}" = "noclobber" ] && [ ! -f "$dst" ]; then
    echo ">> new file (copy): $(basename "$dst")"
  fi
  if [ -f "$dst" ] && cmp -s "$src" "$dst"; then
    [ "$CHECK" = 1 ] || echo ">> up-to-date: $(basename "$dst")"
  else
    if [ "$CHECK" = 1 ]; then
      echo "DRIFT: $dst differs from $src"
      fail=1
    else
      mkdir -p "$(dirname "$dst")"
      cp -f "$src" "$dst"
      echo ">> synced: $(basename "$dst")"
    fi
  fi
}

for f in "${SYNC_SOURCES[@]}"; do
  sync_file "$APP_KIT/$f" "$KIT_SDK/$f"
done

# The C shim + module map are byte-stable copies; the HEADER is synced by
# build_engine_static.sh (with its own drift check) — here we only check it.
sync_file "$CC_APP/shim.c" "$CC_SDK/shim.c"
sync_file "$CC_APP/include/module.modulemap" "$CC_SDK/include/module.modulemap"
sync_file "$REPO/crates/carryon-ffi/include/carryon.h" "$CC_SDK/include/carryon.h"

# Nothing in the SDK may reference the app target or app-only paths.
if grep -rn "CarryOnApp\|import SwiftUI" "$KIT_SDK" 2>/dev/null; then
  echo "FAIL: SDK binding must not depend on the app shell / SwiftUI"
  fail=1
fi

if [ "$fail" = 1 ]; then
  echo "FAIL: SDK sources drifted — run sdk/swift/Scripts/sync_from_app.sh"
  exit 1
fi
echo ">> OK: SDK sources match the app package"
