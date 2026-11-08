/*
 * PackageDocs.kt — package-level documentation for the Android SDK
 * (dev.carryon.kit). Kotlin has no package-info convention for kdoc; this
 * file carries the module contract so it appears in IDE quick-docs and the
 * Dokka/KDoc pipeline.
 *
 * # Carry-On Kit (Android SDK)
 *
 * Typed bindings over the REAL Carry-On engine C ABI
 * (`carryon.h`, ABI 1.2, append-only within major 1). Everything here is a
 * thin, documented translation of the C surface:
 *
 * - [Core] — engine lifecycle (§6), adapters by id + JSON params (§3.8,
 *   compiled-in only), sessions, cuts, actions, object reads, evidence.
 * - [Identity] — device identity; the public surface is the SHA-256 pin hex
 *   ONLY. Private key material never leaves the engine (EVD-006/§22.3).
 * - [Trust] — pinned peers (§18.4/§18.5): pair, revoke, serialize.
 * - [Listener]/[Session] — TLS 1.3 with mutual certificate pinning.
 * - [DiscoveryAdvertiser]/[DiscoveryBrowser] — §18.3 `_carryon._tcp` with
 *   minimal TXT (the engine validates every received record, fail closed).
 * - [serveCut]/[importCut]/[resumeImport] — the transfer legs (BLOCKING; run
 *   them off the main thread; a `CarryonSession` is not thread-safe).
 * - [exportEvidenceWith]/[verifyEvidence] — evidence bundles (§23.3).
 *
 * ## Labels are law (spec §2)
 *
 * Nothing in this SDK upgrades a platform claim: a loopback run is
 * OBSERVED (LOCAL), a two-machine run OBSERVED (PHYSICAL), and no platform is
 * "supported" until §30 acceptance passes on real hardware. The SDK version
 * follows the engine ABI major.minor — call [CarryOnKit.requireCompatibleEngine]
 * at startup.
 *
 * ## Threading
 *
 * The engine is callable from any thread, but blocking entries
 * ([serveCut], [importCut], [resumeImport], [serveAuthorityTransfer],
 * [requestAuthorityTransfer], accept/connect) must run off the main thread and
 * a session handle must never be aliased across threads during a blocking
 * call (carryon.h documents no thread-safety for `CarryonSession`).
 */
package dev.carryon.kit
