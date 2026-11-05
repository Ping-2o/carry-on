/*
 * Carry-On for Android — settings (Kotlin DSL).
 *
 * App wrapper over the REAL engine C ABI (crates/carryon-ffi/include/carryon.h).
 * No Rust changes; the shim in app/src/main/cpp is app-local (plan.md).
 *
 * Claim labels (spec §2): the project is build-ready TARGET — it is NOT compiled
 * on the authoring machine (no Android SDK present); the demo is OBSERVED (LOCAL)
 * once run on a device; no signed APK (PLAT-006). See README.md.
 */
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "CarryOn"
include(":app")
