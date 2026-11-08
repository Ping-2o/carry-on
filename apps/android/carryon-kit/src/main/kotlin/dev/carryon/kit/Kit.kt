/*
 * Kit.kt — the SDK identity + the ABI compatibility gate (the Kotlin mirror of
 * CarryOnKit's KitVersion.swift).
 *
 * Version policy (plan.md stage 9): the SDK version FOLLOWS the engine ABI
 * major.minor — ABI 1.2 → SDK 1.2.x. Major tracks the ABI major (breaking);
 * minor tracks the ABI minor (append-only additions); patch is SDK-only fixes.
 *
 * The gate: call [CarryOnKit.requireCompatibleEngine] once at startup (the
 * contract tests do). An engine older than the SDK's ABI minor fails with a
 * clear, actionable message instead of a random E_PROTO at the first call.
 *
 * SECRETS (EVD-006/§22.3): nothing in this file ever touches key material.
 */
package dev.carryon.kit

/** The Kotlin SDK identity + the runtime ABI gate. */
object CarryOnKit {
    /** SDK version (semver, follows the engine ABI major.minor). */
    const val SDK_VERSION = "1.2.0"

    /** The engine ABI this AAR was built against (vendored carryon.h). */
    const val ABI_MAJOR = 1
    const val ABI_MINOR = 2

    /** Human-readable requirement string for error messages. */
    val abiRequirement: String
        get() = "engine ABI $ABI_MAJOR.$ABI_MINOR (append-only within major $ABI_MAJOR)"

    /**
     * Verify the loaded engine against this SDK. Throws [CoreException] with a
     * `-4` (ERR_BAD_JSON-class) local code and an actionable message when the
     * engine's ABI does not satisfy this SDK.
     *
     * A NEWER minor is fine: the ABI is append-only within major 1, so an
     * older SDK sees every symbol it was built against.
     */
    fun requireCompatibleEngine() {
        val engine = try {
            abiVersion()
        } catch (e: CoreException) {
            throw CoreException(
                e.code,
                "CarryOnKit $SDK_VERSION could not load the engine ($abiRequirement). " +
                    "Build and package libcarryon_ffi.so with the SDK's fetch script " +
                    "(apps/android/Scripts/fetch_engine_lib.sh).",
            )
        }
        if (engine.major != ABI_MAJOR) {
            throw CoreException(
                -4,
                "CarryOnKit $SDK_VERSION requires $abiRequirement; the loaded engine reports ABI " +
                    "${engine.major}.${engine.minor}. A different major is an unsupported pairing — " +
                    "rebuild the engine from the same source.",
            )
        }
        if (engine.minor < ABI_MINOR) {
            throw CoreException(
                -4,
                "CarryOnKit $SDK_VERSION requires $abiRequirement; the loaded engine reports ABI " +
                    "${engine.major}.${engine.minor}. The engine is older than the SDK — rebuild it " +
                    "(apps/android/Scripts/fetch_engine_lib.sh).",
            )
        }
    }
}
