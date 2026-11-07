/*
 * Carry-On for Android — :app module.
 *
 * Binds the REAL engine through the C ABI: libcarryon_ffi.so is cross-compiled by
 * Scripts/fetch_engine_lib.sh into src/main/jniLibs/<ABI>/ (fetched, never
 * committed); app/src/main/cpp/carryon_jni.c is the app-local JNI shim linked
 * against the prebuilt engine (IMPORTED in CMakeLists.txt).
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
    // Scripts/fetch_engine_lib.sh). Pinned explicitly so CI/AGP resolve exactly
    // this NDK instead of a silently different AGP default.
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
        externalNativeBuild {
            cmake {
                cppFlags += ""
                arguments += listOf("-DANDROID_STL=none")
            }
        }
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
            version = "3.22.1"
        }
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

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
        // libcarryon_ffi.so comes from jniLibs (not from a dependency); nothing to
        // dedupe. Keep default compression (minSdk 29 > 23, uncompressed is fine).
    }

    sourceSets {
        getByName("main") {
            // Explicit for clarity: prebuilt engine + placeholder until fetched.
            jniLibs.srcDir("src/main/jniLibs")
        }
    }
}

dependencies {
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
    debugImplementation("androidx.compose.ui:ui-tooling")

    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    // JVM unit tests (the zombie-isolation guard test; runs via
    // gradlew :app:testDebugUnitTest in CI).
    testImplementation("junit:junit:4.13.2")
}
