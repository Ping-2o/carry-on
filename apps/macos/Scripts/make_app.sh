#!/bin/sh
# make_app.sh — assemble dist/CarryOn.app from the release build and verify it.
#
#   Scripts/make_app.sh
#
# Bundle layout:
#   CarryOn.app/Contents/MacOS/CarryOn              (release executable)
#   CarryOn.app/Contents/Frameworks/libcarryon_ffi.dylib  (real engine)
#   CarryOn.app/Contents/Info.plist                 (local-network + Bonjour keys)
#
# Signing: `codesign --sign -` is AD-HOC, LOCAL-ONLY signing for this machine.
# It is NOT PLAT-006 distribution signing (no Developer ID, no notarization, no
# signed APK-equivalent); no platform is "supported" (spec §30 unrun).
set -eu

app_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$app_dir"

bundle="dist/CarryOn.app"
bin=".build/release/CarryOnApp"

echo "make_app: swift build -c release"
swift build -c release

if [ ! -f "$bin" ]; then
    echo "error: release binary not found at $bin" >&2
    exit 1
fi
if [ ! -f "Vendor/libcarryon_ffi.dylib" ]; then
    echo "error: Vendor/libcarryon_ffi.dylib missing; run Scripts/build_engine.sh first" >&2
    exit 1
fi

rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Frameworks" "$bundle/Contents/Resources"

cp "$bin" "$bundle/Contents/MacOS/CarryOn"
cp "Vendor/libcarryon_ffi.dylib" "$bundle/Contents/Frameworks/libcarryon_ffi.dylib"

# The dev binary carries an absolute rpath to Vendor/ (see Package.swift); add the
# bundle-relative rpath so the bundle resolves the engine from its own Frameworks.
install_name_tool -add_rpath @executable_path/../Frameworks \
    "$bundle/Contents/MacOS/CarryOn"

# The link-time absolute Vendor rpath is STALE in the sealed bundle: dyld searches
# LC_RPATH entries in listed order and -add_rpath appends, so the mutable,
# gitignored Vendor/ copy would win over the signed Contents/Frameworks copy.
# Delete it from the bundled copy only (the dev binary in .build/ is untouched).
# -delete_rpath fails if the rpath string is absent — fail-closed under set -eu.
install_name_tool -delete_rpath "$app_dir/Vendor" \
    "$bundle/Contents/MacOS/CarryOn"

# Regression gate: the sealed executable must carry no rpath pointing at the
# mutable out-of-bundle Vendor dir (toolchain rpaths like /usr/lib/swift stay).
if otool -l "$bundle/Contents/MacOS/CarryOn" | grep -A2 LC_RPATH \
        | grep -F "$app_dir/Vendor" >/dev/null 2>&1; then
    echo "error: sealed bundle still carries the absolute Vendor rpath" >&2
    exit 1
fi

cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>Carry-On</string>
	<key>CFBundleExecutable</key>
	<string>CarryOn</string>
	<key>CFBundleIdentifier</key>
	<string>org.carryon.macos</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>CarryOn</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>0.1.0</string>
	<key>CFBundleVersion</key>
	<string>1</string>
	<key>LSMinimumSystemVersion</key>
	<string>14.0</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.utilities</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSLocalNetworkUsageDescription</key>
	<string>Carry-On advertises and browses local Carry-On devices (_carryon._tcp) — engine-side discovery over the local network. Advertisements carry only minimal metadata (protocol version, instance id, pairing flag, device class, connection hint) — never user, session, or key material.</string>
	<key>NSBonjourServices</key>
	<array>
		<string>_carryon._tcp</string>
	</array>
	<key>NSHumanReadableCopyright</key>
	<string>Copyright © 2026 Carry-On contributors. LOCAL demo build — ad-hoc signed, not distribution signing (PLAT-006).</string>
</dict>
</plist>
PLIST

# Sign the nested dylib first, then the bundle (ad-hoc, LOCAL-ONLY — not PLAT-006).
codesign --sign - --force "$bundle/Contents/Frameworks/libcarryon_ffi.dylib"
codesign --sign - --force "$bundle"

echo "make_app: verifying signature"
codesign -v --strict "$bundle"
codesign -dv "$bundle" 2>&1 | sed 's/^/  /' | head -6

echo "make_app: headless launch check (bundle binary, --smoke-exit)"
# Per-run private log (mktemp): a fixed /tmp path is pre-plantable by another
# local user on a shared machine (sticky /tmp, symlink-following truncate).
smoke_log="$(mktemp "${TMPDIR:-/tmp}/carryon-app-smoke.XXXXXX")"
trap 'rm -f "$smoke_log"' EXIT
if "$bundle/Contents/MacOS/CarryOn" --smoke-exit >"$smoke_log" 2>&1; then
    echo "make_app: bundle smoke-exit PASSED (log: $smoke_log)"
else
    rc=$?
    echo "error: bundle smoke-exit FAILED (rc=$rc); tail of log:" >&2
    tail -20 "$smoke_log" >&2
    exit 1
fi

echo "make_app: OK — $bundle (launch with: open $bundle)"
