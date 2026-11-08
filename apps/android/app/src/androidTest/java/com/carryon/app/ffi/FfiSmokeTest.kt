/*
 * FfiSmokeTest.kt — instrumented smoke test: the REAL engine .so loads on a
 * device (regression for the 2026-11-07 device-run crash).
 *
 * The bug: the CI-built APK's libcarryon_jni.so recorded the engine dependency
 * by its ABSOLUTE BUILD-MACHINE PATH in DT_NEEDED (the cargo cdylib has no
 * SONAME), so the very first FFI call died on-device with
 *
 *   java.lang.UnsatisfiedLinkError: dlopen failed: library
 *   "/home/runner/work/.../libcarryon_ffi.so" not found
 *
 * The fix (IMPORTED_NO_SONAME in cpp/CMakeLists.txt) is enforced at build time
 * by cpp/verify_dt_needed.cmake; THIS test is the device-side proof — it can
 * only pass on a real device/emulator with the real engine packaged:
 *
 *   ./gradlew connectedDebugAndroidTest
 *
 * (needs one connected device; NOT part of the JVM unit suite — an FFI load
 * cannot be verified by a JVM test, and CI runners have no device).
 */
package com.carryon.app.ffi

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class FfiSmokeTest {

    /** Class-load loads libcarryon_jni.so + its engine dependency — the exact
     *  call path that crashed at the first device run. */
    @Test
    fun engineLoadsOnDeviceAndReportsAbi_1_2() {
        val abi = CarryOnFfi.abiVersion()
        assertEquals("CARRYON ABI major (append-only within a major)", 1, abi[0])
        assertEquals("CARRYON ABI minor (engine 1.2 — discovery + pairing entries)", 2, abi[1])
    }
}
