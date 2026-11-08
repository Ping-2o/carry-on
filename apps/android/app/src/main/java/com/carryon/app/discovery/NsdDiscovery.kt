/*
 * NsdDiscovery.kt — ENGINE-CONTRACT discovery on Android (stage 4).
 *
 * The platform NSD responder registers + browses `_carryon._tcp` (native
 * NSD — the Android side of the §18.3 split; the engine's own mdns backend is
 * the macOS/desktop path), but the ENGINE owns the TXT contract:
 *
 *   - the TXT this device ADVERTISES is built by the engine
 *     (DiscoveryContract.encode — it can only ever emit the §18.3 minimal set);
 *   - every RECEIVED record is validated by the engine
 *     (DiscoveryContract.validate — fail closed: a malformed record is DROPPED,
 *     never repaired; foreign keys are ignored, never interpreted).
 *
 * TXT keys (§18.3 minimal metadata ONLY):
 *   pm — protocol major version
 *   id — short instance identifier (random 8-hex, not a user name)
 *   pr — pairing-required flag
 *   dc — coarse device class: desktop | mobile | unknown
 *   ch — ephemeral connection hint (the bound listener port, or "none")
 *
 * Battery (plan stage 4): foreground-first — the UI owns the lifecycle
 * (advertise on the Receive screen while armed, browse on the Topology screen;
 * nothing self-schedules). A small peer cache with a TTL keeps recent peers
 * visible, revalidated by the connect itself.
 *
 * LABEL (§2): a discovered peer is a NETWORK FACT. Connection still pins the
 * peer's certificate fingerprint (manual pin until stage-5 pairing UX); no
 * platform is "supported" (§30 unrun). Discovery runs are labeled per the
 * physical procedure in docs/evidence/.
 */
package com.carryon.app.discovery

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import dev.carryon.kit.CoreException
import dev.carryon.kit.DiscoveryContract
import dev.carryon.kit.DiscoveryTxt
import dev.carryon.kit.abiVersion
import dev.carryon.kit.Err
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import java.util.UUID

/** The §18.3 service type. */
const val CARRYON_SERVICE_TYPE: String = "_carryon._tcp"

/** How long a resolved peer stays listed without a re-announcement
 *  (mirrors the engine's mdns cache TTL on the desktop side). */
const val PEER_CACHE_TTL_MS: Long = 30_000

/** UI copy — the label is law (§2). */
const val DISCOVERY_LABEL: String =
    "Engine-contract discovery (§18.3) — the engine builds + validates the TXT; " +
        "NSD registers/browses. Discovery is a network fact, never a pairing; " +
        "platform supported: NO (§30 unrun)."

/** One engine-validated peer (minimal metadata only). */
data class DiscoveredPeer(
    val serviceName: String,
    val instanceId: String,
    val protocolMajor: Int,
    val pairingRequired: Boolean,
    val deviceClass: String,
    /** The resolved SRV address ("<ipv4>:<port>") — a dial candidate,
     * revalidated by the connect itself (discovery never vouches). */
    val host: String,
    val port: Int,
    val validatedAtElapsedMs: Long,
    /** Foreign TXT keys the engine ignored (diagnostics only). */
    val ignoredKeys: List<String>,
)

data class DiscoveryState(
    val advertising: Boolean = false,
    val advertisedName: String = "",
    val browsing: Boolean = false,
    val peers: List<DiscoveredPeer> = emptyList(),
    val lastError: String? = null,
    /** Net-003 (plan stage 4): wall-clock ms from browse start to the FIRST
     * engine-validated peer — measured with the monotonic clock (§18.10),
     * shown in the UI + recorded in the physical-run evidence. */
    val firstPeerLatencyMs: Long? = null,
    /** Why the engine's fail-closed filter dropped records (diagnostics). */
    val rejections: List<String> = emptyList(),
)

/**
 * NSD register + browse behind the ENGINE's §18.3 TXT contract. Callbacks
 * arrive on the main thread (where discover/register are called); state is a
 * plain StateFlow the UI collects.
 */
class NsdDiscovery(private val context: Context) {

    private val nsd: NsdManager? =
        context.getSystemService(Context.NSD_SERVICE) as? NsdManager

    private val _state = MutableStateFlow(DiscoveryState())
    val state: StateFlow<DiscoveryState> = _state.asStateFlow()

    /** Short random instance identifier (NOT a user/device name — §18.3). */
    private val instanceId: String =
        UUID.randomUUID().toString().replace("-", "").take(8)

    /** Monotonic-base for cache TTL + NET-003 (uptime ms). */
    private var browseStartedAtElapsedMs: Long? = null

    private var regListener: NsdManager.RegistrationListener? = null
    /** The port the CURRENT registration advertises (re-arm re-registers). */
    private var advertisedPort: Int? = null
    private var discListener: NsdManager.DiscoveryListener? = null

    /**
     * Advertise this device. `port` = the bound engine listener port, or 0 for
     * presence-only (`ch` = "none"). The TXT is built by the ENGINE
     * (fail-closed: an encode error surfaces in state and nothing is
     * advertised).
     */
    @Suppress("DEPRECATION") // pre-API-34 NsdManager API; fine for minSdk 29
    fun advertise(port: Int, deviceClass: String = "mobile", pairingRequired: Boolean = true) {
        val manager = nsd ?: run {
            _state.update { it.copy(lastError = "NSD unavailable on this device") }
            return
        }
        // Re-arm means a NEW listener/port: the old registration advertises a
        // dead port (found by the 2026-11-08 physical run — connection
        // refused). Re-register when the port changed, never silently keep
        // the stale one.
        if (regListener != null) {
            if (advertisedPort == port) return // already advertising THIS port
            runCatching { manager.unregisterService(regListener) }
            regListener = null
        }
        // ---- engine owns the advertised TXT (§18.3) ----
        val txt = DiscoveryTxt(
            protocolMajor = abiVersion().major,
            instanceId = instanceId,
            pairingRequired = pairingRequired,
            deviceClass = deviceClass,
            connectionHint = if (port > 0) port.toString() else "none",
        )
        val pairs: List<Pair<String, String>> = try {
            NsdTxtPairs.decodePairs(DiscoveryContract.encode(txt))
        } catch (e: CoreException) {
            _state.update {
                it.copy(lastError = "engine refused the TXT contract: ${e.message}")
            }
            return
        }
        val info = NsdServiceInfo().apply {
            serviceName = "carryon-$instanceId"
            serviceType = CARRYON_SERVICE_TYPE
            setPort(if (port > 0) port else 0)
            pairs.forEach { (k, v) -> setAttribute(k, v.take(255)) }
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
                advertisedPort = null
            }

            override fun onServiceUnregistered(serviceInfo: NsdServiceInfo) {
                _state.update { it.copy(advertising = false, advertisedName = "") }
                advertisedPort = null
            }

            override fun onUnregistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                _state.update { it.copy(lastError = "unregister failed (code $errorCode)") }
            }
        }
        regListener = listener
        try {
            manager.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener)
            advertisedPort = port
        } catch (e: IllegalArgumentException) {
            _state.update { it.copy(lastError = "register rejected: ${e.message}") }
            regListener = null
            advertisedPort = null
        }
    }

    /** Browse for `_carryon._tcp` peers; resolve + ENGINE-validate their TXT. */
    @Suppress("DEPRECATION")
    fun startBrowse() {
        val manager = nsd ?: run {
            _state.update { it.copy(lastError = "NSD unavailable on this device") }
            return
        }
        if (discListener != null) return // already browsing
        browseStartedAtElapsedMs = android.os.SystemClock.elapsedRealtime()
        _state.update { it.copy(browsing = true, firstPeerLatencyMs = null) }
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
                @Suppress("DEPRECATION")
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
            // A failed resolve is a stale/foreign instance — fail closed, drop.
            val why = "${info.serviceName}: resolve failed (code $errorCode)"
            _state.update { st ->
                st.copy(
                    rejections = (st.rejections + why).distinct().takeLast(8),
                    peers = st.peers.filter { it.serviceName != info.serviceName },
                )
            }
        }

        override fun onServiceResolved(info: NsdServiceInfo) {
            // ---- ENGINE owns the received TXT contract (§18.3 fail-closed) ---
            // ALL attributes go in (foreign keys included); the engine reports
            // what it ignored. A malformed record throws E_PLATFORM_DISCOVERY
            // and the peer is DROPPED, never repaired.
            val now = android.os.SystemClock.elapsedRealtime()
            val pairs = NsdTxtPairs.fromAttributes(info.attributes)
            val peer: DiscoveredPeer = try {
                val (txt, ignored) = DiscoveryContract.validate(pairs)
                if (txt.protocolMajor != abiVersion().major) {
                    reject("${info.serviceName}: pm=${txt.protocolMajor} != our ${abiVersion().major}")
                    return
                }
                DiscoveredPeer(
                    serviceName = info.serviceName,
                    instanceId = txt.instanceId,
                    protocolMajor = txt.protocolMajor,
                    pairingRequired = txt.pairingRequired,
                    deviceClass = txt.deviceClass,
                    // getHost() is InetAddress pre-API-34; hostAddress is the dial string.
                    host = info.host?.hostAddress ?: "",
                    port = info.port,
                    validatedAtElapsedMs = now,
                    ignoredKeys = ignored,
                )
            } catch (e: CoreException) {
                reject("${info.serviceName}: ${Err.name(e.code)} ${e.message}")
                return
            }
            _state.update { st ->
                var next = st.copy(peers = (st.peers.filter { p ->
                    // TTL: drop stale sightings of the same instance.
                    p.serviceName != peer.serviceName &&
                        now - p.validatedAtElapsedMs < PEER_CACHE_TTL_MS
                } + peer).sortedBy { it.serviceName })
                if (next.firstPeerLatencyMs == null && browseStartedAtElapsedMs != null) {
                    next = next.copy(firstPeerLatencyMs = now - browseStartedAtElapsedMs!!)
                }
                next
            }
        }

        private fun reject(why: String) {
            _state.update { st -> st.copy(rejections = (st.rejections + why).distinct().takeLast(8)) }
        }
    }

    /** Tear down registration + browsing (the screen left — foreground-first). */
    @Suppress("DEPRECATION")
    fun stop() {
        val manager = nsd ?: return
        regListener?.let {
            runCatching { manager.unregisterService(it) }
            regListener = null
            advertisedPort = null
        }
        discListener?.let {
            runCatching { manager.stopServiceDiscovery(it) }
            discListener = null
        }
        _state.update { DiscoveryState(peers = it.peers) }
    }
}

/**
 * Pure extraction of DNS-SD TXT attributes into pairs (unit-testable without
 * the engine; the engine does the actual validating). `info.attributes` keys
 * with NULL values are empty-value keys — they pair with "".
 */
object NsdTxtPairs {
    fun fromAttributes(attrs: Map<String, ByteArray?>): List<Pair<String, String>> =
        attrs.entries.map { (k, v) -> k to (v?.toString(Charsets.US_ASCII) ?: "") }

    /** Decode the engine's canonical "[[key,value],...]" pairs JSON. */
    fun decodePairs(pairsJson: String): List<Pair<String, String>> {
        val arr = org.json.JSONArray(pairsJson)
        return List(arr.length()) { i ->
            val pair = arr.getJSONArray(i)
            pair.getString(0) to pair.getString(1)
        }
    }
}
