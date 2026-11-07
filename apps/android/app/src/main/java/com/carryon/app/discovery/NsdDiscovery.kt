/*
 * NsdDiscovery.kt — APP-LEVEL discovery pilot (plan.md §3 "Discovery pilot").
 *
 * NSD register + browse for `_carryon._tcp` with the §18.3 MINIMAL metadata TXT
 * contract only:
 *
 *   pm — protocol major version
 *   id — short instance identifier (random 8-hex, not a user name)
 *   pr — pairing-required flag
 *   dc — coarse device class: desktop | mobile | unknown
 *   ch — ephemeral connection hint (empty until a listener is bound)
 *
 * TXT records MUST NOT expose user names, session names, application names,
 * object hashes, public keys, or capability details (§18.3) — enforced here by
 * construction: only the five keys above are ever set or parsed.
 *
 * LABEL (§2): this is an app-level pilot. Engine-side discovery is TARGET
 * (FRONTIER Phase 2); UI copy states this verbatim. Nothing here claims the
 * engine can discover.
 */
package com.carryon.app.discovery

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.util.UUID

/** UI copy — the label is law (plan.md §3). */
const val DISCOVERY_PILOT_LABEL: String =
    "App-level discovery pilot — engine-side discovery is TARGET (FRONTIER Phase 2)."

/** The §18.3 service type. */
const val CARRYON_SERVICE_TYPE: String = "_carryon._tcp"

/** One browsed peer (minimal metadata only). */
data class DiscoveredPeer(
    val serviceName: String,
    val instanceId: String,
    val protocolMajor: Int,
    val pairingRequired: Boolean,
    val deviceClass: String,
    val connectionHint: String,
)

data class DiscoveryState(
    val advertising: Boolean = false,
    val advertisedName: String = "",
    val browsing: Boolean = false,
    val peers: List<DiscoveredPeer> = emptyList(),
    val lastError: String? = null,
)

/** NSD register + browse pilot. Callbacks arrive on the main thread; state is a
 *  plain StateFlow the UI collects. */
class NsdDiscovery(private val context: Context) {

    private val nsd: NsdManager? =
        context.getSystemService(Context.NSD_SERVICE) as? NsdManager

    private val _state = MutableStateFlow(DiscoveryState())
    val state: StateFlow<DiscoveryState> = _state.asStateFlow()

    /** Short random instance identifier (NOT a user/device name — §18.3). */
    private val instanceId: String =
        UUID.randomUUID().toString().replace("-", "").take(8)

    private var regListener: NsdManager.RegistrationListener? = null
    private var discListener: NsdManager.DiscoveryListener? = null

    /** Advertise this device. `port` = the bound engine listener port, or 0 when
     *  nothing is listening (pilot placeholder, labeled as such in the UI). */
    @Suppress("DEPRECATION") // pre-API-34 NsdManager API; fine for the pilot
    fun advertise(port: Int, deviceClass: String = "mobile", connectionHint: String = "") {
        val manager = nsd ?: run {
            _state.update { it.copy(lastError = "NSD unavailable on this device") }
            return
        }
        if (regListener != null) return // already advertising
        val info = NsdServiceInfo().apply {
            serviceName = "carryon-$instanceId"
            serviceType = CARRYON_SERVICE_TYPE
            setPort(port)
            setAttribute("pm", "1")           // protocol major version (§18.3)
            setAttribute("id", instanceId)    // short instance id
            setAttribute("pr", "1")           // pairing required
            setAttribute("dc", deviceClass)   // coarse device class
            setAttribute("ch", connectionHint.take(64)) // ephemeral hint
        }
        val listener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(serviceInfo: NsdServiceInfo) {
                _state.update {
                    it.copy(advertising = true, advertisedName = serviceInfo.serviceName)
                }
            }

            override fun onRegistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                _state.update { it.copy(lastError = "register failed (code $errorCode)") }
                regListener = null
            }

            override fun onServiceUnregistered(serviceInfo: NsdServiceInfo) {
                _state.update { it.copy(advertising = false, advertisedName = "") }
            }

            override fun onUnregistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                _state.update { it.copy(lastError = "unregister failed (code $errorCode)") }
            }
        }
        regListener = listener
        try {
            manager.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener)
        } catch (e: IllegalArgumentException) {
            // TXT key/value constraint violations (§18.3 keys are safe; defensive).
            _state.update { it.copy(lastError = "register rejected: ${e.message}") }
            regListener = null
        }
    }

    /** Browse for `_carryon._tcp` peers and resolve their minimal TXT records. */
    @Suppress("DEPRECATION")
    fun startBrowse() {
        val manager = nsd ?: run {
            _state.update { it.copy(lastError = "NSD unavailable on this device") }
            return
        }
        if (discListener != null) return // already browsing
        val listener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {
                _state.update { it.copy(browsing = true) }
            }

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                _state.update {
                    it.copy(browsing = false, lastError = "browse start failed (code $errorCode)")
                }
                discListener = null
            }

            override fun onServiceFound(serviceInfo: NsdServiceInfo) {
                if (!serviceInfo.serviceType.contains(CARRYON_SERVICE_TYPE)) return
                manager.resolveService(serviceInfo, resolveListener)
            }

            override fun onServiceLost(serviceInfo: NsdServiceInfo) {
                _state.update { st ->
                    st.copy(peers = st.peers.filter { it.serviceName != serviceInfo.serviceName })
                }
            }

            override fun onDiscoveryStopped(serviceType: String) {
                _state.update { it.copy(browsing = false) }
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
                _state.update { it.copy(lastError = "browse stop failed (code $errorCode)") }
            }
        }
        discListener = listener
        try {
            manager.discoverServices("$CARRYON_SERVICE_TYPE.", NsdManager.PROTOCOL_DNS_SD, listener)
        } catch (e: IllegalArgumentException) {
            _state.update { it.copy(lastError = "browse rejected: ${e.message}") }
            discListener = null
        }
    }

    @Suppress("DEPRECATION")
    private val resolveListener = object : NsdManager.ResolveListener {
        override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
            _state.update { it.copy(lastError = "resolve failed (code $errorCode)") }
        }

        override fun onServiceResolved(info: NsdServiceInfo) {
            val attrs = info.attributes
            fun txt(key: String): String =
                attrs[key]?.let { String(it, Charsets.US_ASCII) } ?: ""
            // Only the §18.3 minimal keys are read; everything else is ignored.
            // `pr` accepts both encodings: the Android pilot advertises "1",
            // the macOS pilot "true"/"false" — a strict "== \"1\"" read made
            // macOS peers look pairing-NOT-required here (review finding).
            val peer = DiscoveredPeer(
                serviceName = info.serviceName,
                instanceId = txt("id"),
                protocolMajor = txt("pm").toIntOrNull() ?: 0,
                pairingRequired = txt("pr") == "1" || txt("pr") == "true",
                deviceClass = txt("dc").ifBlank { "unknown" },
                connectionHint = txt("ch"),
            )
            _state.update { st ->
                st.copy(peers = (st.peers.filter { it.serviceName != peer.serviceName } + peer)
                    .sortedBy { it.serviceName })
            }
        }
    }

    /** Tear down registration + browsing (called when the topology screen leaves). */
    @Suppress("DEPRECATION")
    fun stop() {
        val manager = nsd ?: return
        regListener?.let {
            runCatching { manager.unregisterService(it) }
            regListener = null
        }
        discListener?.let {
            runCatching { manager.stopServiceDiscovery(it) }
            discListener = null
        }
        _state.update { DiscoveryState(peers = it.peers) }
    }
}
