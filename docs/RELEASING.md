# Releasing signed apps

How the release workflow (`.github/workflows/release.yml`) builds and signs the
Android and macOS apps (issue #21, PLAT-006).

**Label: `TARGET`.** The workflow is written but has not run with real secrets.
PLAT-006 is not met until a signed build from it installs and launches on a clean
device and a clean Mac (see "Still open").

## What the workflow does

| Trigger | Android | macOS | GitHub Release |
|---|---|---|---|
| `v*` tag, all secrets set | release APK + AAB signed with the upload keystore, checked with `apksigner verify` | `CarryOn.app` signed with Developer ID (hardened runtime), notarized and stapled; `.dmg` signed, notarized and stapled | **draft** release with both artifacts + `SHA256SUMS-*.txt`, notes generated from `.github/release.yml` |
| `v*` tag, secrets missing | unsigned APK/AAB as a workflow artifact | ad-hoc (LOCAL-ONLY) `.dmg` as a workflow artifact | **not created** |
| manual run (any branch) | as above, depending on secrets | as above | not created (tags only) |

The release job also refuses to publish if any `-unsigned` file shows up. The
release stays a draft: the owner reviews and publishes it by hand.

The Kiwix backend is never enabled in release builds, so the artifacts stay
`MIT OR Apache-2.0` ([`kiwix-bridge/LICENSING.md`](kiwix-bridge/LICENSING.md)).

## Secrets

Set them under **Settings → Secrets and variables → Actions**. Never commit them,
and never paste them into evidence (EVD-006).

| Secret | What it is |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | the upload keystore (`.jks`), base64-encoded |
| `ANDROID_KEYSTORE_PASSWORD` | keystore password |
| `ANDROID_KEY_ALIAS` | key alias inside the keystore |
| `ANDROID_KEY_PASSWORD` | key password |
| `MACOS_CERT_P12_BASE64` | "Developer ID Application" certificate + private key exported as `.p12`, base64-encoded |
| `MACOS_CERT_PASSWORD` | password of that `.p12` |
| `MACOS_SIGN_IDENTITY` | e.g. `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple ID used for notarization |
| `APPLE_TEAM_ID` | 10-character team ID |
| `APPLE_APP_PASSWORD` | app-specific password for that Apple ID (not the account password) |

Creating the Android keystore once (keep a backup: losing it means you can no
longer update the app):

```sh
keytool -genkeypair -v -keystore carryon-release.jks -keyalg RSA -keysize 4096 -validity 10000 -alias carryon
base64 -i carryon-release.jks | pbcopy   # paste into ANDROID_KEYSTORE_BASE64
```

In CI the keystore is decoded to a temp file, passed to Gradle through
`CARRYON_KEYSTORE_FILE` / `CARRYON_KEYSTORE_PASSWORD` / `CARRYON_KEY_ALIAS` /
`CARRYON_KEY_PASSWORD`, and deleted at the end of the job. Without
`CARRYON_KEYSTORE_FILE`, `apps/android/app/build.gradle.kts` configures no signing,
so local and PR builds behave exactly as before. The Developer ID certificate goes
into a temporary keychain that is deleted at the end of the job.

## Cutting a release

1. Bump `versionCode` / `versionName` in `apps/android/app/build.gradle.kts`. The
   macOS version is stamped from the tag (`v1.2.3` → `1.2.3`).
2. Tag `main`: `git tag v1.2.3 && git push origin v1.2.3`.
3. Wait for the **Release** workflow, then review the draft release (artifacts,
   checksums, generated notes) and publish it.
4. Run the install check below and record it as evidence.

## Still open

- [ ] First real run with secrets (nothing here has been run).
- [ ] Install check: the signed APK on a clean Android device; the notarized
      `.dmg` on a clean Mac (Gatekeeper opens it without warnings). This is what
      PLAT-006 needs.
- [ ] The release build type has R8 minify + resource shrinking on, and PR CI only
      builds `assembleDebug`. The first release run is the first time the
      minified app is built, so check the engine/JNI classes survive R8.
- [ ] `versionCode` is still bumped by hand.
- [ ] Play Store upload (Play App Signing) is not part of this workflow; the AAB is
      only attached to the GitHub Release.
