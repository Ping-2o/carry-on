# SDK versioning and releases

Policy for the two SDKs, CarryOnKit (Swift, `sdk/swift`) and carryon-kit
(Android AAR, `apps/android/carryon-kit`), and how the SDK release workflow
(`.github/workflows/sdk-release.yml`) ships them (issue #22).

**Label: `TARGET`.** The workflow is written but has not run with real secrets,
and no sample app has consumed a published artifact on a real device yet.

## The rule: the SDK version follows the ABI

`SDK = MAJOR.MINOR.PATCH`, where `MAJOR.MINOR` is always the engine ABI
(`CARRYON_ABI_MAJOR` / `CARRYON_ABI_MINOR` in `crates/carryon-ffi/include/carryon.h`).

| Change | ABI | SDK |
|---|---|---|
| bindings, docs or packaging only, no new C entry points | unchanged | **PATCH** + 1 (1.3.0 → 1.3.1) |
| new entry points (the C ABI is append-only within a major) | MINOR + 1 | **MINOR** + 1, PATCH = 0 (1.3.x → 1.4.0) |
| anything removed or changed in place | MAJOR + 1 | **MAJOR** + 1 (→ 2.0.0) |

- Both SDKs always release together with the same version.
- A version is never reused, and published artifacts are never replaced.
- No SDK release spans two ABI minors.
- The runtime gate (`requireCompatibleEngine()` in both SDKs) refuses a different
  ABI major and an older ABI minor, and accepts a newer minor (append-only).

### Where the version lives

| File | Value |
|---|---|
| `sdk/swift/Sources/CarryOnKit/KitVersion.swift` | `CarryOnSDK.version` |
| `apps/android/carryon-kit/build.gradle.kts` | `version = "…"` (+ the `carryonAbiMinor` placeholder) |
| `apps/android/carryon-kit/src/main/AndroidManifest.xml` | `dev.carryon.kit.SDK_VERSION` and `dev.carryon.kit.ABI_MINOR` |

The `abi-gate` job reads all of them plus `carryon.h` and fails closed if any
disagree, if the SDK does not start with the ABI `MAJOR.MINOR`, or if the tag does
not match.

## Cutting an SDK release

1. Update the version in the three files above (and the ABI if it changed).
2. Tag `main`: `git tag sdk-v1.3.0 && git push origin sdk-v1.3.0`. App releases
   use `v*` tags (docs/RELEASING.md); the namespaces never overlap.
3. The workflow builds:
   - **Swift:** `CarryOnEngine-<v>.xcframework.zip` (the static engine) + its SwiftPM
     checksum, a distribution tree `CarryOnKit-<v>-spm.zip` whose `Package.swift`
     uses `.binaryTarget(url:checksum:)` pointing at this release's asset (the tree
     is first built against the local archive to prove it links);
   - **Android:** `dev.carryon:carryon-kit:<v>` (AAR + POM + sources jar), PGP-signed,
     as a Maven repo zip and the bare AAR.
4. Review the **draft** release and publish it. Publishing makes the Swift asset URL
   live, so the checksum-pinned manifest starts resolving.

Manual runs build everything and publish nothing. Without the PGP secrets the AAR
is built unsigned and no release is created.

## Secrets and variables

| Name | Kind | What |
|---|---|---|
| `SDK_PGP_PRIVATE_KEY` | secret | ASCII-armored PGP private key used to sign the Maven artifacts |
| `SDK_PGP_PASSPHRASE` | secret | its passphrase |
| `SDK_PUBLISH_GITHUB_PACKAGES` | variable | `true` to also publish the AAR to GitHub Packages on a tag (off by default) |

Never commit keys, and never paste them into evidence (EVD-006).

## Consuming the SDKs

**Swift.** SwiftPM needs `Package.swift` at the repository root and plain semver tags
(`1.3.0`). This repo has neither for the SDK (the root is the Cargo workspace, and
`v*` tags belong to the apps). So the generated `CarryOnKit-<v>-spm.zip` is meant
to be pushed to a dedicated distribution repo (proposed: `carryon-kit-swift`) and
tagged `<v>` there:

```swift
.package(url: "https://github.com/Ping-2o/carryon-kit-swift", from: "1.3.0")
```

Until that repo exists, use the path dependency from `sdk/swift/README.md`.

**Android.** From GitHub Packages (reading needs a token with `read:packages`):

```kotlin
repositories {
    maven {
        url = uri("https://maven.pkg.github.com/Ping-2o/carry-on")
        credentials { username = "<user>"; password = "<token>" }
    }
}
dependencies { implementation("dev.carryon:carryon-kit:1.3.0") }
```

Or unzip `carryon-kit-<v>-maven-repo.zip` and point a `maven { url = uri("…") }`
repository at it.

## Still open

- [ ] First real run with secrets.
- [ ] A sample app consuming the **published** artifacts on a real device (Done-when
      item; record it as evidence).
- [ ] Create the Swift distribution repo, or decide on another way to make the
      package consumable by tag.
- [ ] Maven Central: needs a verified `dev.carryon` namespace; not wired.
- [ ] The engine archive is macOS arm64 only and the AAR is arm64-v8a only (same as
      today's builds).
- [ ] No Javadoc jar yet (Dokka not wired).
