/*
 * KitVersion.swift — SDK identity + the ABI compatibility gate. THIS FILE IS
 * SDK-LOCAL (never overwritten by sync_from_app.sh): it carries the SDK
 * version and the engine ABI the package was built against.
 *
 * Version policy (plan.md stage 9): the SDK version FOLLOWS the engine ABI
 * major.minor — ABI 1.3 → SDK 1.3.x. Major tracks the ABI major (breaking);
 * minor tracks the ABI minor (append-only additions); patch is SDK-only
 * fixes. No SDK release ever spans two ABI minors.
 *
 * The compatibility gate: call `CarryOnSDK.requireCompatibleEngine()` at
 * startup (all examples and tests do) — an engine older than the SDK's ABI
 * minor fails with a clear, actionable error instead of a random E_PROTO or a
 * misparsed frame.
 */

import CCarryon
import Foundation

public enum CarryOnSDK {
    /// SDK version (semver, follows the engine ABI major.minor).
    public static let version = "1.3.0"

    /// The engine ABI this package was built against (vendored carryon.h).
    public static let abiMajor: UInt32 = CARRYON_ABI_MAJOR
    public static let abiMinor: UInt32 = CARRYON_ABI_MINOR

    /// Human-readable requirement string for error messages.
    public static let abiRequirement = "engine ABI \(abiMajor).\(abiMinor) (append-only within major \(abiMajor))"

    /// The gate. Throws a CarryonError with an actionable message when the
    /// loaded engine's ABI does not satisfy this SDK (older minor = missing
    /// entries; different major = unsupported pairing).
    public static func requireCompatibleEngine() throws {
        var major: UInt32 = 0, minor: UInt32 = 0
        carryon_abi_version(&major, &minor)
        if major != abiMajor {
            throw CarryonError(
                code: -4,
                message: "CarryOnKit \(version) requires \(abiRequirement); the loaded engine reports ABI \(major).\(minor). " +
                    "A different major is an unsupported pairing — rebuild the engine from the same source " +
                    "(sdk/swift/Scripts/build_engine_static.sh).")
        }
        if minor < abiMinor {
            throw CarryonError(
                code: -4,
                message: "CarryOnKit \(version) requires \(abiRequirement); the loaded engine reports ABI \(major).\(minor). " +
                    "The engine is older than the SDK — rebuild it (sdk/swift/Scripts/build_engine_static.sh).")
        }
        // A NEWER minor is fine: the ABI is append-only within major 1, so an
        // older SDK on a newer engine sees every symbol it was built against.
    }
}
