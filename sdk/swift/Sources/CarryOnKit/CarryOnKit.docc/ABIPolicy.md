# ABI policy and the compatibility gate

## The contract

`Sources/CCarryon/include/carryon.h` is the ABI contract. It is a byte-identical
copy of `crates/carryon-ffi/include/carryon.h`, synced by
`Scripts/build_engine_static.sh` with a drift check. The ABI is **append-only
within major 1**: new entries appear in a new minor; no existing entry changes
signature or meaning.

## Versioning

| SDK version | Engine ABI | Meaning |
|---|---|---|
| `1.3.1` | `1.3` | patch = SDK-only fix (docs, validation, no ABI change) |
| `1.3.0` | `1.3` | new ABI minor — new entries, append-only |
| `2.0.0` | `2.0` | ABI major — breaking; new pairing required |

An SDK built against ABI `1.2` runs fine on an engine `1.3` (append-only), but
an engine `1.1` is refused: it lacks entries the SDK may call.

## The gate

`CarryOnSDK.requireCompatibleEngine()` compares
`carryon_abi_version()` against the constants this package was built with and
throws a `CarryonError` with an actionable message on a mismatch — call it at
startup, before any other engine call.

## The suite

`Tests/CarryOnKitTests/AbiCompatibilityTests.swift` checks the three-way
contract on every `swift test`:

1. every `carryon_*` symbol in the header is **exported** by the vendored
   engine archive (`nm` on the `.xcframework`);
2. every header symbol is **bound** by the Swift sources (or explicitly
   allowlisted with a reason);
3. the vendored header is byte-identical to the engine crate's, and the SDK
   version's major/minor equal the header's ABI major/minor.

The Kotlin SDK runs the mirror suite (`AbiCompatibilityTest.kt`) against the
JNI shim, the externals, and the AAR's published manifest metadata.
