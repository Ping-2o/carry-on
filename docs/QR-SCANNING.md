# QR scanning for pairing

Issue #26. **Label: `TARGET`.** The code is written and its pure half is unit-tested
on the JVM, but there is no device run on record yet.

## Android: implemented (source side)

The source's **Scan QR (camera)** button on the pairing screen:

1. **Asks for the permission only when needed.** `CAMERA` is declared in the manifest,
   but it is only requested when the user taps the button. If it is denied, the
   typed/pasted path is still there. `android.hardware.camera.any` is
   `required="false"`, so devices without a camera can still install the app.
2. **Reads frames with CameraX and decodes them with zxing core.** zxing is the same
   pure-Java library that already renders the destination's QR, so there is no
   Play-services dependency. Frames are analysed off the main thread
   (`QrScannerView.kt`). The first decoded text is delivered once, then the
   camera is unbound.
3. **Validates, failing closed.** The scanned text is only a *candidate*:
   - `QrScanGate` (`QrScan.kt`) rejects obvious junk before the engine sees it:
     empty text, longer than 1024 chars, control characters, or not `carryon://pair?`.
     It can only reject; it never accepts anything by itself.
   - The **engine** (`pairingPayloadParse`) parses the candidate. If it refuses,
     nothing is filled in.
   - If the engine accepts, address, code and payload are filled in for the user to
     review. **Connect & pair** then takes exactly the pasted-text path: a pinned
     connect to the key from the payload, plus the code check and the mutual
     verification-string confirm (spec §18.4.2-5).

Tests: `apps/android/app/src/test/java/com/carryon/app/pairing/QrScanTest.kt`
(render → decode round-trip, padded row stride, noise, bad geometry, gate cases).

### Still open (device evidence)

- [ ] Scan a destination's QR on a real phone (e.g. the A04), then pair. Record it as
      PHYSICAL evidence.
- [ ] Permission denied, then the manual fallback still works (same device).
- [ ] A malformed or foreign QR is refused and nothing is filled (same device).

## macOS: decision, not now

Not implemented in this change. The paste/type path stays the macOS route.

Why:

1. In the pilot the Mac is mostly the side that *shows* the QR. When the Mac joins
   as the source, the payload text can be copied from the other screen, or the
   address and six-digit code can be typed. Both go through the same engine
   validation already.
2. Scanning on macOS isn't free:
   - it needs an AVFoundation capture session plus Vision (`VNDetectBarcodesRequest`);
     `AVCaptureMetadataOutput` does not exist on macOS;
   - `NSCameraUsageDescription` in the Info.plist that `make_app.sh` writes;
   - because the release build is signed with the hardened runtime (#21), the
     `com.apple.security.device.camera` entitlement.

   That widens what the signed app may do, for a rarely used path.
3. Continuity Camera (an iPhone as the Mac's camera) goes through that same
   AVFoundation path (macOS 14 lists it as a `.continuityCamera` device type). One
   implementation would therefore cover both the built-in camera and an iPhone.

Revisit when Mac-as-source pairing becomes a common flow, or when a user asks for
it. The sketch then: a capture session on the default video device, frames into
Vision, then the result into the existing payload-paste handler (so the engine still
parses it), plus the usage string and the entitlement.
