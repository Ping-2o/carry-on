/*
 * PairingFlow.kt — the stage-5 PIN/QR pairing flow (spec §18.4 steps 1–9)
 * driving the REAL engine through the C ABI, for BOTH roles:
 *
 *   armDestination — §18.4.1: the destination binds a listener, derives the
 *                    six-digit pairing code, and formats the QR payload
 *                    (engine-owned `carryon://pair?v=1&n=…&a=…&k=…&r=…`).
 *
 *   awaitPairing   — §18.4.3-7: accepts the PROVISIONAL pairing channel
 *                    (the engine records the source's pin; content is refused
 *                    on it, §18.5), checks the source's code echo (fail closed),
 *                    shows the verification string for the LOCAL user's
 *                    confirmation (§18.4.5), then — in a second negotiation
 *                    round — exchanges the mutual decision with the source, so
 *                    NO side can end up half-paired: both learn Paired, or
 *                    both learn Refused/Cancelled.
 *
 *   joinDestination — §18.4.2-5: the source enters the code (QR path: parses
 *                    the payload and pins the destination BEFORE connecting —
 *                    an already-authenticated channel; manual fallback:
 *                    provisional connect + the verification-string comparison
 *                    on both screens), confirms locally, and learns the
 *                    destination's decision in round 2.
 *
 * Labels (§2 law): loopback join → OBSERVED (LOCAL); a join over the LAN to
 * a real second device → PHYSICAL. Camera scanning is NOT implemented in
 * this pilot (honest fallback: the human reads the QR payload text / the
 * six-digit code); no platform is "supported" (§30 unrun).
 *
 * Secrets law (EVD-006/§22.3): the transcript the flows emit contains device
 * names, pins (public fingerprints), the code, and the verification string —
 * never private keys, never session content (§18.4.8).
 */
package com.carryon.app.pairing

import com.carryon.app.ffi.Identity
import com.carryon.app.ffi.Listener
import com.carryon.app.ffi.Session
import com.carryon.app.ffi.Trust
import com.carryon.app.ffi.pairingCode
import com.carryon.app.ffi.pairingPayloadFormat
import com.carryon.app.ffi.pairingPayloadParse
import com.carryon.app.ffi.pairingVerificationString
import com.carryon.app.handoff.HandoffFlow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import java.time.Instant
import java.time.format.DateTimeFormatter

/** §18.4 state of one pairing attempt (the UI confirms AwaitingConfirm). */
sealed class PairingState {
    object Armed : PairingState()
    /** The provisional channel is up; both sides must show the same string. */
    data class AwaitingConfirm(
        val peerName: String,
        val peerPin: String,
        val verification: String,
    ) : PairingState()
    data class Paired(val peerName: String, val peerPin: String, val verification: String) : PairingState()
    data class Refused(val why: String) : PairingState()
    object Cancelled : PairingState()
}

/** The destination side's armed context (§18.4.1). */
class PairingArmed internal constructor(
    val identity: Identity,
    val listener: Listener,
    val pinHex: String,
    /** The QR payload URI (engine-formatted). */
    val payloadUri: String,
    /** The six-digit pairing code shown beside the QR. */
    val code: String,
    /** The wire address bound ("0.0.0.0:port"). */
    val wireAddr: String,
    /** The display address for the source ("<site-local-ipv4>:<port>"). */
    val displayAddr: String,
) : java.io.Closeable {
    /** Closes the LISTENER only. The identity is BORROWED (the app's durable
     *  Keystore-backed identity, or a test's): freeing it here would be a
     *  double free when the owner closes it (device-run bug 2026-11-08). */
    override fun close() {
        runCatching { listener.close() }
    }
}

/** Features the pairing channel carries — data only (§3.8). */
private const val CONFIRMED = "confirmed=1"
private const val ACCEPTED = "accepted=1"

object PairingFlow {

    fun utcNow(): String =
        DateTimeFormatter.ISO_INSTANT.format(Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS))

    /**
     * §18.4.1 — arm the destination: identity, listener, engine-derived
     * six-digit code, engine-formatted QR payload. Cheap; nothing blocks.
     */
    suspend fun armDestination(
        identity: Identity,
        deviceName: String,
    ): PairingArmed = withContext(Dispatchers.IO) {
        val listener = Listener.bind("0.0.0.0:0")
        val wireAddr = listener.addr()
        val port = wireAddr.substringAfterLast(':').toLongOrNull() ?: 0L
        val pin = identity.pinHex()
        val code = pairingCode(pin, if (port > 0) port else System.nanoTime())
        val display = "${HandoffFlow.siteLocalIpv4() ?: wireAddr.substringBefore(':')}:$port"
        // The QR payload carries the SOURCE-DIALABLE display address (the
        // site-local IPv4), never the wildcard wire bind.
        val payload = pairingPayloadFormat(deviceName, display, pin, code)
        PairingArmed(
            identity = identity,
            listener = listener,
            pinHex = pin,
            payloadUri = payload,
            code = code,
            wireAddr = wireAddr,
            displayAddr = display,
        )
    }

    /**
     * §18.4.3-7 — the destination half. `confirm` resolves from the local
     * Confirm button (§18.4.5: the user confirms on BOTH devices). Round 1:
     * code echo check (fail closed). Round 2: the mutual decision — the
     * source learns `accepted=1` only if BOTH confirmed; anything else ends
     * Refused/Cancelled on both sides (never half-paired).
     */
    suspend fun awaitPairing(
        armed: PairingArmed,
        confirm: suspend (PairingState.AwaitingConfirm) -> Boolean,
        onEvent: (String) -> Unit = {},
    ): PairingState = withContext(Dispatchers.IO) {
        onEvent("awaiting source on ${armed.displayAddr} (provisional §18.4.3; code ${armed.code})")
        try {
            Session.acceptPairing(armed.listener, armed.identity).use { wire ->
                onEvent("provisional channel up (pairing=${wire.isPairing()})")
                val sourcePin = wire.peerPinHex()

                // Round 1: the source's Hello carries data only (§3.8) —
                // its name + the code echo.
                val theirs = JSONArray(wire.serverNegotiate("[]"))
                val features = List(theirs.length()) { theirs.getString(it) }
                val name = features.firstOrNull { it.startsWith("name=") }?.removePrefix("name=")
                    ?: "source"
                val echo = features.firstOrNull { it.startsWith("code=") }?.removePrefix("code=")
                if (echo != armed.code) {
                    onEvent("code echo mismatch (got ${echo ?: "none"}) → REFUSED (fail closed, §18.4.2)")
                    // Closing here makes the source's round 2 fail closed too.
                    return@use PairingState.Refused("pairing code mismatch")
                }

                val vs = pairingVerificationString(sourcePin, armed.pinHex)
                onEvent("verification string: $vs — confirm on BOTH devices (§18.4.4-5)")
                if (!confirm(PairingState.AwaitingConfirm(name, sourcePin, vs))) {
                    onEvent("user declined → session closed, nothing pinned")
                    return@use PairingState.Cancelled
                }

                // Round 2: hold the channel open for the source's decision;
                // only a clean `confirmed=1` pairs (refuse anything else).
                // (Features are a JSON array of strings — data only, §3.8.)
                val theirRound = JSONArray(wire.serverNegotiate("""["$ACCEPTED"]"""))
                val roundFeatures = List(theirRound.length()) { theirRound.getString(it) }
                if (CONFIRMED in roundFeatures) {
                    onEvent("mutual confirm complete → pinning $name (${sourcePin.take(16)}…)")
                    PairingState.Paired(name, sourcePin, vs)
                } else {
                    onEvent("source did not send a clean confirm → REFUSED (fail closed)")
                    PairingState.Refused("source confirmation missing")
                }
            }
        } catch (e: Exception) {
            // Round-2 I/O failure (the other side declined/closed): both
            // sides end NOT paired — fail closed, never half-trust.
            onEvent("pairing channel closed before mutual confirm (${e.message}) → NOT paired")
            PairingState.Refused(e.message ?: "pairing channel closed")
        }
    }

    /**
     * §18.4.2-5 — the source half. QR path (`payloadUri` from the
     * destination's display): the payload carries the destination's pin, so
     * the connect is a NORMAL pinned one and a MITM fails the handshake.
     * Manual fallback (addr + code typed, no payload): provisional connect —
     * then BOTH screens' verification strings are the mutual confirmation
     * (§18.4.4-5). Round 2 delivers the destination's decision.
     */
    suspend fun joinDestination(
        identity: Identity,
        addr: String,
        code: String,
        payloadUri: String?,
        confirm: suspend (PairingState.AwaitingConfirm) -> Boolean,
        onEvent: (String) -> Unit = {},
    ): PairingState = withContext(Dispatchers.IO) {
        var destPin: String?
        var pinnedDest: Trust? = null
        if (payloadUri != null) {
            val payload = pairingPayloadParse(payloadUri)
            if (code.isNotBlank() && payload.code != code) {
                onEvent("typed code != payload code → REFUSED (fail closed)")
                return@withContext PairingState.Refused("pairing code mismatch")
            }
            val t = Trust.empty()
            t.pair(payload.deviceName, payload.pinHex, utcNow())
            pinnedDest = t
            destPin = payload.pinHex
            onEvent("QR payload pinned ${payload.deviceName} (${destPin.take(16)}…) — normal pinned connect")
        } else {
            destPin = null
            onEvent("manual mode: provisional connect (§18.4.3) — compare BOTH verification strings")
        }

        val wire = try {
            if (pinnedDest != null) {
                Session.connect(addr, identity, pinnedDest)
            } else {
                Session.connectPairing(addr, identity)
            }
        } catch (e: Exception) {
            onEvent("connect failed: ${e.message}")
            return@withContext PairingState.Refused(e.message ?: "connect failed")
        }
        pinnedDest?.close()
        wire.use {
            val dstPin = destPin ?: it.peerPinHex()

            // Round 1: send name + code echo (data only, §3.8).
            val hello = JSONArray()
                .put("name=carryon-android") // TODO stage-6: the real device name rendezvous
                .put("code=$code")
            it.clientNegotiate(hello.toString())

            // Compute + show the SAME verification string the destination shows.
            val vs = pairingVerificationString(identity.pinHex(), dstPin)
            onEvent("verification string: $vs — confirm on BOTH devices (§18.4.4-5)")
            if (!confirm(PairingState.AwaitingConfirm("destination", dstPin, vs))) {
                onEvent("user declined → session closed, nothing pinned")
                return@use PairingState.Cancelled
            }

            // Round 2: send OUR decision, learn THEIRS. A closed channel
            // (they refused the code / declined) throws → NOT paired.
            try {
                // (Features are a JSON array of strings — data only, §3.8.)
                val accepted = JSONArray(it.clientNegotiate("""["$CONFIRMED"]"""))
                val features = List(accepted.length()) { accepted.getString(it) }
                if (ACCEPTED in features) {
                    PairingState.Paired("destination", dstPin, vs)
                } else {
                    PairingState.Refused("destination did not accept")
                }
            } catch (e: Exception) {
                onEvent("destination closed the pairing channel (${e.message}) → NOT paired")
                PairingState.Refused(e.message ?: "destination refused")
            }
        }
    }
}
