/*
 * TopologyStore.kt — app-process state for the Live Device Topology screen.
 *
 * - Local node card: a per-process engine Identity (pin shown TRUNCATED; the key
 *   DER is never touched — §22.3). Durable secure storage is PLAT-002 (TARGET);
 *   this pilot regenerates per app start and says so.
 * - Trust records: real engine TrustStore JSON persisted in app storage via
 *   carryon_trust_to_json / carryon_trust_from_json; revoke via
 *   carryon_trust_revoke (§18.4.9).
 * - Sessions: recorded from demo runs with EXACT §16.2/plan.md readiness states
 *   (Idle, Preparing, CutProposed, CutSealed, Importing, ActionReady,
 *   ReadOnlyActive, Failed, Inconclusive, Aborted) — never the bare "Ready".
 */
package com.carryon.app.ui

import android.content.Context
import android.os.Build
import com.carryon.app.ffi.AbiVersion
import com.carryon.app.ffi.Identity
import com.carryon.app.ffi.Trust
import com.carryon.app.ffi.TrustRecord
import com.carryon.app.ffi.abiVersion
import com.carryon.app.demo.DemoState
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** Exact readiness states (plan.md §3 Sessions; engine model/session.rs). */
object ReadinessStates {
    val ALL = listOf(
        "Idle", "Preparing", "CutProposed", "CutSealed", "Importing",
        "ActionReady", "ReadOnlyActive", "Failed", "Inconclusive", "Aborted",
    )
}

data class IdentityCard(
    val deviceName: String,
    val shortPin: String,
    val abi: AbiVersion,
    val listenerAddr: String?,
)

/** One observed session row for the Topology/Sessions screens. */
data class SessionRow(
    val sessionId: String,
    val title: String,
    /** Exact state string — one of [ReadinessStates.ALL]; never bare "Ready". */
    val state: String,
    val authorityEpoch: Long?,
    val mayMutate: Boolean?,
    val cutNumber: Long?,
    val role: String,
)

class TopologyStore(private val context: Context) {

    private val trustFile = File(context.filesDir, "trust.json")

    private val _identity = MutableStateFlow<IdentityCard?>(null)
    val identity: StateFlow<IdentityCard?> = _identity.asStateFlow()

    private val _trust = MutableStateFlow<List<TrustRecord>>(emptyList())
    val trust: StateFlow<List<TrustRecord>> = _trust.asStateFlow()

    private val _sessions = MutableStateFlow<List<SessionRow>>(emptyList())
    val sessions: StateFlow<List<SessionRow>> = _sessions.asStateFlow()

    /** Engine handle for the app identity (open for the process; pilot scope). */
    private var engineIdentity: Identity? = null
    private var engineTrust: Trust? = null

    /** Lazily generate the local identity + load persisted trust. IO-safe. */
    suspend fun ensureLoaded(): IdentityCard? = withContext(Dispatchers.IO) {
        val id = engineIdentity ?: Identity.generate(deviceName()).also { engineIdentity = it }
        val abi = abiVersion()
        val card = IdentityCard(
            deviceName = deviceName(),
            shortPin = id.pinHex().take(16) + "…",
            abi = abi,
            listenerAddr = _identity.value?.listenerAddr,
        )
        _identity.update { card }

        engineTrust = engineTrust ?: loadTrust()
        _trust.update { engineTrust?.records().orEmpty() }
        card
    }

    private fun deviceName(): String =
        (Build.MODEL ?: "carryon-device").take(24).ifBlank { "carryon-device" }

    private fun loadTrust(): Trust = try {
        if (trustFile.exists()) {
            Trust.fromJson(trustFile.readText())
        } else {
            Trust.empty()
        }
    } catch (_: Exception) {
        Trust.empty() // corrupt store -> fail open as empty (no peers), never crash
    }

    private fun persistTrust() {
        engineTrust?.let { t ->
            runCatching {
                trustFile.writeText(t.toJson())
            }
        }
    }

    /** Record the demo run's real pairing into the app trust store. The demo's
     *  identities are ephemeral (per-run); their FULL pins are public material
     *  (EVD-006 excludes only private keys) and were genuinely pinned. */
    fun recordDemoPairing(sourcePinHex: String, destPinHex: String, utc: String) {
        val t = engineTrust ?: return
        if (sourcePinHex.isBlank() || destPinHex.isBlank()) return
        runCatching {
            t.pair("carryon-src (demo)", sourcePinHex, utc)
            t.pair("carryon-dst (demo)", destPinHex, utc)
            persistTrust()
            _trust.update { t.records() }
        }
    }

    /** §18.4.9 revocation — bumps trust_generation; reconnect fails closed. */
    fun revoke(pinHex: String) {
        val t = engineTrust ?: return
        runCatching {
            t.revoke(pinHex)
            persistTrust()
            _trust.update { t.records() }
        }
    }

    /** Record the completed demo run's mirror session with its exact state. */
    fun recordDemoSession(state: DemoState) {
        val bundleJson = state.evidenceBundleJson ?: return
        runCatching {
            val bundle = JSONObject(bundleJson)
            val sessionId = bundle.getString("session_id")
            val epoch = state.authorityEpoch
            val rows = listOf(
                SessionRow(
                    sessionId = sessionId,
                    title = "demo ${state.scenario.sessionTitle}",
                    state = if (state.destMayMutate == true) "ActionReady" else "Importing",
                    authorityEpoch = epoch,
                    mayMutate = state.destMayMutate,
                    cutNumber = null,
                    role = "destination (owner)",
                ),
                SessionRow(
                    sessionId = sessionId,
                    title = "demo source (same logical session)",
                    state = if (state.sourceMayMutate == false) "ReadOnlyActive" else "Failed",
                    authorityEpoch = null,
                    mayMutate = state.sourceMayMutate,
                    cutNumber = null,
                    role = "source (relinquished)",
                ),
            )
            _sessions.update { current -> (rows + current).take(20) }
        }
    }

    /** Called by the demo screen when a listener binds/unbinds (topology card). */
    fun setListenerAddr(addr: String?) {
        _identity.update { it?.copy(listenerAddr = addr) }
    }
}
