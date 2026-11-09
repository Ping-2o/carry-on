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
 * scenarios run on the host JVM against a desktop shim (contract tests).
 * Maven publication + PGP signing are wired for the SDK release workflow
 * (#22, docs/SDK-VERSIONING.md); nothing has been published from it yet.
 */
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
    id("maven-publish")
    id("signing")
}

// SDK version follows the engine ABI major.minor (plan.md stage 9): ABI 1.3 →
// SDK 1.3.x. Recorded in the AAR's metadata and republished in the merged
// manifest (dev.carryon.kit.SDK_VERSION).
version = "1.3.0"
group = "dev.carryon"

android {
    namespace = "dev.carryon.kit"
    compileSdk = 35

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
        manifestPlaceholders["carryonAbiMinor"] = "3"
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
    kotlinOptions {
        jvmTarget = "17"
    }

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

    // Maven publication of the release AAR (#22). The sources jar ships with
    // it; a Javadoc jar is not produced yet (Dokka is not wired).
    publishing {
        singleVariant("release") {
            withSourcesJar()
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

// ---- Maven publication (#22, docs/SDK-VERSIONING.md) -----------------------
// dev.carryon:carryon-kit:<version>. Always publishable to a local staging repo
// (build/staging-repo) so CI can attach it to a GitHub Release; GitHub Packages
// only when the SDK release workflow hands over credentials. PGP-signed only
// when CARRYON_SIGNING_KEY is set, so local and PR builds are unchanged.
publishing {
    publications {
        register<MavenPublication>("release") {
            groupId = "dev.carryon"
            artifactId = "carryon-kit"
            version = project.version.toString()
            afterEvaluate {
                from(components["release"])
            }
            pom {
                name.set("Carry-On Kit")
                description.set("Android SDK (AAR) over the Carry-On engine C ABI, append-only within major 1.")
                url.set("https://github.com/Ping-2o/carry-on")
                licenses {
                    license {
                        name.set("MIT")
                        url.set("https://opensource.org/licenses/MIT")
                    }
                    license {
                        name.set("Apache-2.0")
                        url.set("https://www.apache.org/licenses/LICENSE-2.0")
                    }
                }
                scm {
                    url.set("https://github.com/Ping-2o/carry-on")
                    connection.set("scm:git:https://github.com/Ping-2o/carry-on.git")
                }
            }
        }
    }
    repositories {
        maven {
            name = "staging"
            url = uri(layout.buildDirectory.dir("staging-repo"))
        }
        val ghPackagesToken = System.getenv("CARRYON_GH_PACKAGES_TOKEN")
        if (!ghPackagesToken.isNullOrBlank()) {
            maven {
                name = "GitHubPackages"
                url = uri("https://maven.pkg.github.com/Ping-2o/carry-on")
                credentials {
                    username = System.getenv("CARRYON_GH_PACKAGES_USER")
                    password = ghPackagesToken
                }
            }
        }
    }
}

signing {
    val signingKey = System.getenv("CARRYON_SIGNING_KEY")
    if (!signingKey.isNullOrBlank()) {
        useInMemoryPgpKeys(signingKey, System.getenv("CARRYON_SIGNING_PASSWORD"))
        sign(publishing.publications)
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
