/*
 * Carry-On for Android — root build (Kotlin DSL).
 *
 * Versions are pinned here (single source of truth; no buildSrc for one module):
 *   AGP 8.7.3   — compileSdk 35, minSdk 29 (spec §9), externalNativeBuild CMake.
 *   Kotlin 2.0.21 + org.jetbrains.kotlin.plugin.compose (Compose compiler GRADLE plugin).
 * Requires JDK 17 to run Gradle (AGP 8.x); see README.md.
 */
plugins {
    id("com.android.application") version "9.5.1" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.0.21" apply false
}

tasks.register<Delete>("clean") {
    delete(rootProject.layout.buildDirectory)
}
