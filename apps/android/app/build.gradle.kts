/*
 * Carry-On for Android — :app module.
 *
 * The engine binding (JNI shim + typed wrappers + libcarryon_ffi.so) now lives
 * in the distributable :carryon-kit library (stage 9 SDK); this app consumes
 * it as a normal project dependency — the same AAR a third-party customer
 * would. The app owns no C ABI code anymore.
 *
 * Labels (spec §2): build-ready TARGET; demo OBSERVED (LOCAL); no signing
 * (PLAT-006). See README.md for the honest claim table.
 */
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.carryon.app"
    compileSdk = 35

    // The NDK the prebuilt engine was cross-compiled with (r27c; proven by
    // Scripts/fetch_engine_lib.sh). Kept so the engine ABI and the NDK the
    // kit module builds its shim with stay pinned in lockstep.
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "com.carryon.app"
        minSdk = 29          // spec §9: Android 10 floor for the frontier target
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"

        // The engine is built for arm64-v8a only (fetch_engine_lib.sh). abiFilters
        // keeps the APK honest: no stub .so for other ABIs.
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
        // Instrumented smoke test (FFI load on a real device — regression for
        // the 2026-11-07 DT_NEEDED device crash; run: connectedDebugAndroidTest).
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
        debug {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // Roborazzi/Robolectric screenshot tests (JVM, no device): CI renders every
    // production screen light + dark and publishes the PNGs to the PR.
    testOptions {
        unitTests {
            isIncludeAndroidResources = true
            all {
                it.systemProperty("roborazzi.test.record", "true")
                it.maxHeapSize = "2g"
            }
        }
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
        // The engine .so + JNI shim now arrive inside the AAR (carryon-kit).
    }
}

dependencies {
    // The Android SDK (stage 9): the same AAR a third-party consumer gets.
    implementation(project(":carryon-kit"))

    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)

    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.navigation:navigation-compose:2.8.5")

    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    // Adaptive navigation: bottom bar on phones, rail on tablets/foldables.
    implementation("androidx.compose.material3:material3-adaptive-navigation-suite:1.3.1")
    implementation("androidx.compose.material:material-icons-extended")
    debugImplementation("androidx.compose.ui:ui-tooling")
    // ComponentActivity host for Compose tests / Roborazzi captures.
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    // Stage 5 (§18.4.1): QR RENDERING ONLY (pure-Java encoder, no camera, no
    // permission). Scanning on this pilot is the honest manual fallback — the
    // payload text + six-digit code are typed/compared by the human.
    implementation("com.google.zxing:core:3.5.4")

    // JVM unit tests (the zombie-isolation guard test; runs via
    // gradlew :app:testDebugUnitTest in CI).
    testImplementation("junit:junit:4.13.2")
    // Real org.json for JVM unit tests: the android.jar org.json classes are
    // stubbed in local unit tests and throw "not mocked" (the JSON-parse
    // regression tests need a working JSONObject/JSONArray).
    testImplementation("org.json:json:20260814")
    // Screenshot tests: Robolectric native graphics + Roborazzi.
    testImplementation("org.robolectric:robolectric:4.14.1")
    testImplementation("io.github.takahirom.roborazzi:roborazzi:1.32.2")
    testImplementation("io.github.takahirom.roborazzi:roborazzi-compose:1.32.2")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("androidx.test.ext:junit:1.2.1")

    // Instrumented tests (device only; the FFI smoke test loads the real engine).
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("junit:junit:4.13.2")
}
