/*
 * AbiCompatibilityTest.kt — the Kotlin half of the ABI compatibility suite
 * (plan.md stage 9: "extend the existing cross-check" → now a test). Pure JVM,
 * no device, runs everywhere:
 *
 *   crates/carryon-ffi/include/carryon.h   (the contract, vendored copy)
 *   src/main/cpp/carryon_jni.c             (the shim)
 *   src/main/kotlin/dev/carryon/kit/CarryOnFfi.kt (the externals)
 *   src/main/AndroidManifest.xml           (the published ABI metadata)
 *
 * Every header symbol must be reachable through the shim (or allowlisted with
 * a reason), every `external fun` must have its exact JNI entry point, and the
 * published ABI/version metadata must agree with the header and the code.
 *
 * The Swift SDK runs the mirror suite (AbiCompatibilityTests.swift) against
 * its bindings + the engine archive.
 */
package dev.carryon.kit

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class AbiCompatibilityTest {

    /**
     * Header symbols the shim intentionally does not wrap.
     *
     * The PhotoCraft live bridge (ABI 1.3) drives a DESKTOP app over its loopback
     * control channel; there is no Android surface for it (the A04 never runs a
     * PhotoCraft desktop instance), so the Kotlin SDK deliberately does not bind
     * it. The Swift SDK does (its desktop package), and both ABI suites still
     * verify the engine exports the symbols.
     */
    private val unboundAllowlist = setOf(
        "carryon_photocraft_inspect",
        "carryon_photocraft_register",
        "carryon_photocraft_restore",
    )

    private fun module(): File = File(".").absoluteFile

    private fun header(): String =
        File(module(), "src/main/cpp/include/carryon.h").readText()

    private fun shimSource(): String =
        File(module(), "src/main/cpp/carryon_jni.c").readText()

    private fun ffiSource(): String =
        File(module(), "src/main/kotlin/dev/carryon/kit/CarryOnFfi.kt").readText()

    private fun manifest(): String =
        File(module(), "src/main/AndroidManifest.xml").readText()

    private fun headerSymbols(): List<String> {
        val regex = Regex("""\b(carryon_[a-z0-9_]+)\s*\(""")
        return regex.findAll(header()).map { it.groupValues[1] }.distinct().sorted().toList()
    }

    // ---- header ↔ shim -------------------------------------------------------

    @Test
    fun everyHeaderSymbolIsReachableThroughTheShim() {
        val symbols = headerSymbols()
        assertTrue("header symbol extraction broke (${symbols.size})", symbols.size > 50)
        val shim = shimSource()
        val missing = symbols.filter { name ->
            !shim.contains("$name(") && name !in unboundAllowlist
        }
        assertTrue(
            "header symbols the shim cannot reach: $missing",
            missing.isEmpty(),
        )
    }

    // ---- shim ↔ Kotlin externals (the JNI name contract) ---------------------

    @Test
    fun everyExternalHasItsExactJniEntryPoint() {
        val externals = Regex("""external fun ([A-Za-z0-9_]+)\(""")
            .findAll(ffiSource())
            .map { it.groupValues[1] }
            .toList()
        assertTrue("external extraction broke (${externals.size})", externals.size > 50)
        val shim = shimSource()
        val missing = externals.filter { name ->
            !shim.contains("Java_dev_carryon_kit_CarryOnFfi_$name")
        }
        assertTrue(
            "externals without a matching JNI entry point (would be UnsatisfiedLinkError): $missing",
            missing.isEmpty(),
        )
        // The old app package must not survive anywhere in the shim.
        assertTrue(
            "shim still exports Java_com_carryon_app_ffi_* names",
            !shim.contains("Java_com_carryon_app_ffi_"),
        )
    }

    @Test
    fun everyJniEntryPointBacksAnExternal() {
        val externals = Regex("""external fun ([A-Za-z0-9_]+)\(""")
            .findAll(ffiSource()).map { it.groupValues[1] }.toSet()
        val jniNames = Regex("""Java_dev_carryon_kit_CarryOnFfi_([A-Za-z0-9_]+)""")
            .findAll(shimSource()).map { it.groupValues[1] }.toSet()
        val orphans = jniNames - externals
        assertTrue("JNI entry points with no external fun: $orphans", orphans.isEmpty())
    }

    // ---- versions and published metadata -------------------------------------

    @Test
    fun sdkVersionFollowsAbiMajorMinor() {
        val parts = CarryOnKit.SDK_VERSION.split(".").map { it.toInt() }
        assertEquals("SDK version must be semver MAJOR.MINOR.PATCH", 3, parts.size)
        assertEquals(CarryOnKit.ABI_MAJOR, parts[0])
        assertEquals(CarryOnKit.ABI_MINOR, parts[1])
    }

    @Test
    fun headerAbiMatchesTheSdkConstants() {
        val header = header()
        val major = Regex("""#define CARRYON_ABI_MAJOR (\d+)u""").find(header)!!.groupValues[1].toInt()
        val minor = Regex("""#define CARRYON_ABI_MINOR (\d+)u""").find(header)!!.groupValues[1].toInt()
        assertEquals(CarryOnKit.ABI_MAJOR, major)
        assertEquals(CarryOnKit.ABI_MINOR, minor)
    }

    @Test
    fun manifestPublishesTheSameAbi() {
        val manifest = manifest()
        val major = Regex("""dev\.carryon\.kit\.ABI_MAJOR"\s+android:value="(\d+)"""")
            .find(manifest)!!.groupValues[1].toInt()
        val minor = Regex("""dev\.carryon\.kit\.ABI_MINOR"\s+android:value="(\d+)"""")
            .find(manifest)!!.groupValues[1].toInt()
        val version = Regex("""dev\.carryon\.kit\.SDK_VERSION"\s+android:value="([0-9.]+)"""")
            .find(manifest)!!.groupValues[1]
        assertEquals(CarryOnKit.ABI_MAJOR, major)
        assertEquals(CarryOnKit.ABI_MINOR, minor)
        assertEquals(CarryOnKit.SDK_VERSION, version)
    }

    @Test
    fun consumerProguardRulesKeepTheJniBridge() {
        val rules = File(module(), "consumer-rules.pro").readText()
        assertTrue(
            "consumer rules must keep dev.carryon.kit.CarryOnFfi",
            rules.contains("-keep class dev.carryon.kit.CarryOnFfi"),
        )
        assertTrue(
            "consumer rules must keep native method names",
            rules.contains("native <methods>"),
        )
    }
}
