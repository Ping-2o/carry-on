# ``CarryOnKit``

The Swift SDK over the **real** Carry-On engine C ABI
(`carryon.h`, ABI 1.2, append-only within major 1): typed handles for the
engine lifecycle, TLS 1.3 transport with mutual certificate pinning, the L3
editor and L2 graph adapters, the L0/L1 file handoff, pairing, engine-side
discovery, evidence bundles, and the 8-stage demo pipeline.

## Getting started

```sh
# 1. Build and vendor the engine (fetched, never committed):
sdk/swift/Scripts/build_engine_static.sh

# 2. Add the package (path dependency, or a git dependency on sdk/swift):
#    .package(path: "path/to/carry-on/sdk/swift")
```

```swift
import CarryOnKit

// Always verify the engine ABI before using the SDK:
try CarryOnSDK.requireCompatibleEngine()

let engine = try Engine.open(dataDir: myDataDir)
try engine.registerAdapter(id: "org.carryon.file",
                           paramsJSON: #"{"path":"/abs/path/to/file"}"#)
let session = try engine.createSession(adapterID: "org.carryon.file",
                                       title: "carry a file",
                                       authorityMode: "read_only_replica")
let cut = try engine.createCut(sessionID: session)
```

## Versioning

The SDK version follows the engine ABI `major.minor` (ABI 1.2 → SDK 1.2.x):
major tracks the ABI major (breaking), minor tracks the ABI minor
(append-only additions), patch is SDK-only fixes. See
<doc:ABIPolicy>.

## Labels are law

Nothing in this SDK upgrades a platform claim. A loopback run is
`OBSERVED (LOCAL)`; a run across two real machines is `OBSERVED (PHYSICAL)`;
no platform is "supported" until §30 acceptance passes on real hardware.
Private key material never leaves the engine (`EVD-006`): only the SHA-256
pin fingerprint is exposed.
