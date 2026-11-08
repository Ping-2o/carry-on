/*
 * Carry-On for Android — root build (Kotlin DSL).
 *
 * Versions are pinned here (single source of truth; no buildSrc for one module):
 *   AGP 9.5.1   — compileSdk 36, minSdk 29 (spec §9), externalNativeBuild CMake.
 *                 AGP 9 compiles Kotlin itself ("built-in Kotlin"), so modules no
 *                 longer apply org.jetbrains.kotlin.android (#46).
 *   Kotlin 2.4.21 — pinned through org.jetbrains.kotlin.plugin.compose (Compose
 *                 compiler GRADLE plugin); declaring it here puts KGP 2.4.21 on
 *                 the build classpath, which AGP's built-in Kotlin then uses.
 * Requires JDK 17+ to run Gradle (AGP 9 needs Gradle 9.1+; wrapper is 9.8.2).
 */
plugins {
    id("com.android.application") version "9.5.1" apply false
    id("com.android.library") version "9.5.1" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.21" apply false
}

tasks.register<Delete>("clean") {
    delete(rootProject.layout.buildDirectory)
}
