/*
 * Carry-On for Android — carryon-kit library module (§9 SDK, plan.md stage 9).
 *
 * The distributable Kotlin SDK over the REAL Carry-On engine C ABI
 * (crates/carryon-ffi/include/carryon.h, ABI 1.3, append-only within major 1).
 * Ships as an AAR: the JNI shim (built here with CMake against the fetched
 * engine .so), the engine .so itself (jniLibs, fetched — never committed), the
 * typed wrappers (dev.carryon.kit), kdoc, and consumer ProGuard rules.
 *
 * Version policy: the SDK version FOLLOWS the engine ABI major.minor — ABI 1.3
 * → SDK 1.3.x (append-only minor; patch = SDK-only fixes).
 *
 * Claim labels (spec §2): the AAR is IMPLEMENTED (builds); the engine calls it
 * wraps are OBSERVED on a device (instrumented tests) and the same contract
 * scenarios run on the host JVM against a desktop shim (contract tests). No
 * signed release artifact (PLAT-006 out of scope).
 */
plugins {
    id("com.android.library")
}

// SDK version follows the engine ABI major.minor (plan.md stage 9): ABI 1.3 →
// SDK 1.3.x. Recorded in the AAR's metadata and republished in the merged
// manifest (dev.carryon.kit.SDK_VERSION).
version = "1.3.0"
group = "dev.carryon"

android {
    namespace = "dev.carryon.kit"
    compileSdk = 36

    // The NDK the prebuilt engine was cross-compiled with (r27c; proven by
    // Scripts/fetch_engine_lib.sh). Pinned so AGP resolves exactly this NDK.
    ndkVersion = "27.2.12479018"

    defaultConfig {
        minSdk = 29          // spec §9: Android 10 floor

        // The engine is built for arm64-v8a only (fetch script); no stub .so
        // for other ABIs — the AAR is honest about what it contains.
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
        externalNativeBuild {
            cmake {
                cppFlags += ""
                arguments += listOf("-DANDROID_STL=none")
            }
        }
        // Consumer ProGuard rules ship inside the AAR: the JNI class and its
        // native method names must survive minification.
        consumerProguardFiles("consumer-rules.pro")
        // ABI metadata in the merged manifest (consumers can read it).
        manifestPlaceholders["carryonAbiMajor"] = "1"
        manifestPlaceholders["carryonAbiMinor"] = "2"
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
            version = "3.22.1"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false // the module itself is already compact
        }
        debug {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    // No kotlinOptions {}: AGP 9 built-in Kotlin derives jvmTarget from
    // compileOptions.targetCompatibility (17).

    buildFeatures {
        buildConfig = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDir("src/main/jniLibs")
        }
    }

    // ---- host-JVM contract tests ------------------------------------------
    // The SAME contract scenarios as the Swift SDK run on the host JVM against
    // a desktop JNI shim (Scripts/build_desktop_shim.sh). The engine + shim are
    // looked up at test run time via carryonDesktopLibDir; without it the
    // engine-level tests skip (pure-logic tests still run everywhere).
    testOptions {
        unitTests.isReturnDefaultValues = false
    }
}

tasks.withType(org.gradle.api.tasks.testing.Test::class.java).configureEach {
    val libDir = providers.gradleProperty("carryonDesktopLibDir").orNull
        ?: System.getenv("CARRYON_DESKTOP_LIBDIR")
    if (libDir != null) {
        systemProperty("carryon.desktop.libdir", libDir)
        systemProperty("java.library.path", libDir)
        environment("DYLD_LIBRARY_PATH", libDir)
        environment("LD_LIBRARY_PATH", libDir)
    }
}

dependencies {
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20260814")
}
