/*
 * Pairing.swift — the stage-5 PIN/QR pairing flow (spec §18.4 steps 1–9),
 * driving the REAL engine through the C ABI, for BOTH roles:
 *
 *   PairingFlow.armDestination — §18.4.1: bind a listener, derive the
 *       six-digit code, format the QR payload (engine-owned URI).
 *
 *   PairingFlow.awaitPairing — §18.4.3-7 (destination): accept the PROVISIONAL
 *       pairing channel (the engine records the source's pin; content is
 *       refused on it, §18.5), check the code echo (fail closed), show the
 *       verification string for the LOCAL user's confirmation, then exchange
 *       the mutual decision in a second negotiation round — NO side can end
 *       up half-paired: both learn Paired, or both learn Refused/Cancelled.
 *
 *   PairingFlow.joinDestination — §18.4.2-5 (source): enter the code (QR
 *       path: parse the payload and pin the destination BEFORE connecting —
 *       an already-authenticated channel; manual fallback: provisional
 *       connect + verification-string comparison on both screens), confirm
 *       locally, and learn the destination's decision in round 2.
 *
 * Labels (§2 law): loopback join → OBSERVED (LOCAL); a LAN join to a real
 * second device → PHYSICAL. Camera scanning is NOT implemented in this pilot
 * (honest fallback: paste the payload text / type the six-digit code); no
 * platform is "supported" (§30 unrun).
 *
 * Secrets law (EVD-006/§22.3): the transcript carries device names, pins
 * (public fingerprints), the code, and the verification string — never
 * private keys, never session content (§18.4.8).
 */

import Foundation

public enum PairingState: Sendable, Equatable {
    case armed
    /// The provisional channel is up; both sides must show the same string.
    case awaitingConfirm(peerName: String, peerPin: String, verification: String)
    case paired(peerName: String, peerPin: String, verification: String)
    case refused(why: String)
    case cancelled
}

/// The destination side's armed context (§18.4.1).
public final class PairingArmed: @unchecked Sendable {
    public let listener: NetListener
    public let pinHex: String
    /// The QR payload URI (engine-formatted).
    public let payloadURI: String
    /// The six-digit pairing code shown beside the QR.
    public let code: String
    /// The wire address bound ("0.0.0.0:port").
    public let wireAddr: String
    /// The display address for the source ("<site-local-ipv4>:<port>").
    public let displayAddr: String

    init(listener: NetListener, pinHex: String, payloadURI: String, code: String,
         wireAddr: String, displayAddr: String) {
        self.listener = listener
        self.pinHex = pinHex
        self.payloadURI = payloadURI
        self.code = code
        self.wireAddr = wireAddr
        self.displayAddr = displayAddr
    }

    /// Nothing to close by hand (the Android sibling leaks a blocked listener
    /// deliberately): the listener handle frees on ARC deinit once the last
    /// blocked accept completes; never free under a blocked call.
    public func close() {}
}

public enum PairingFlow {

    /// The site-local IPv4 for the display address (no entitlements needed).
    public static func siteLocalIPv4() -> String? {
        var address: String?
        var ifaddr: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifaddr) == 0, let first = ifaddr else { return nil }
        defer { freeifaddrs(ifaddr) }
        var ptr: UnsafeMutablePointer<ifaddrs>? = first
        while let p = ptr {
            defer { ptr = p.pointee.ifa_next }
            let ifa = p.pointee
            guard let sa = ifa.ifa_addr, sa.pointee.sa_family == UInt8(AF_INET) else { continue }
            var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(sa, socklen_t(sa.pointee.sa_len), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 {
                let s = String(cString: host)
                if s.hasPrefix("192.168.") || s.hasPrefix("10.") || s.hasPrefix("172.") {
                    address = s
                    break
                }
            }
        }
        return address
    }

    public static func utcNow() -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f.string(from: Date())
    }

    /// §18.4.1 — arm the destination: listener, engine-derived code, and the
    /// engine-formatted QR payload. Cheap; nothing blocks.
    public static func armDestination(identity: DeviceIdentity, deviceName: String) throws -> PairingArmed {
        let listener = try NetListener.bind("0.0.0.0:0")
        let wireAddr = try listener.addr()
        let port = UInt64(wireAddr.split(separator: ":").last ?? "0") ?? 0
        let pin = try identity.pinHex()
        let code = try pairingCode(pinHex: pin, seed: port > 0 ? port : UInt64(Date().timeIntervalSince1970))
        let display = "\(siteLocalIPv4() ?? String(wireAddr.split(separator: ":").first ?? "0.0.0.0")):\(port)"
        // The QR payload carries the SOURCE-DIALABLE display address (the
        // site-local IPv4), never the wildcard wire bind.
        let payload = try formatPairingPayload(deviceName: deviceName, addr: display, pinHex: pin, code: code)
        return PairingArmed(listener: listener, pinHex: pin, payloadURI: payload, code: code,
                            wireAddr: wireAddr, displayAddr: display)
    }

    /// Features the pairing channel carries — data only (§3.8).
    private static let confirmed = "confirmed=1"
    private static let accepted = "accepted=1"

    /// §18.4.3-7 — the destination half. `confirm` resolves from the local
    /// Confirm button (§18.4.5: the user confirms on BOTH devices). Round 1:
    /// code echo check (fail closed). Round 2: the mutual decision — the
    /// source learns `accepted=1` only if BOTH confirmed; anything else ends
    /// Refused/Cancelled on both sides (never half-paired).
    public static func awaitPairing(
        armed: PairingArmed,
        identity: DeviceIdentity,
        confirm: @escaping @Sendable (PairingState) async -> Bool,
        onEvent: @escaping @Sendable (String) -> Void = { _ in }
    ) async -> PairingState {
        onEvent("awaiting source on \(armed.displayAddr) (provisional §18.4.3; code \(armed.code))")
        do {
            let wire = try await det("accept_pairing") {
                try NetSession.acceptPairing(listener: armed.listener, identity: identity)
            }
            defer { wire.teardown() }
            onEvent("provisional channel up (pairing=\(try wire.isPairing()))")
            let sourcePin = try wire.peerPinHex()

            // Round 1: the source's Hello carries data only (§3.8).
            let theirs = try await det("server_negotiate") {
                try wire.serverNegotiate(features: [])
            }
            let name = theirs.first(where: { $0.hasPrefix("name=") })?.dropFirst(5).description ?? "source"
            let echo = theirs.first(where: { $0.hasPrefix("code=") })?.dropFirst(5).description
            if echo != armed.code {
                onEvent("code echo mismatch (got \(echo ?? "none")) → REFUSED (fail closed, §18.4.2)")
                return .refused(why: "pairing code mismatch")
            }

            let vs = try pairingVerificationString(sourcePin, armed.pinHex)
            onEvent("verification string: \(vs) — confirm on BOTH devices (§18.4.4-5)")
            let step = PairingState.awaitingConfirm(peerName: name, peerPin: sourcePin, verification: vs)
            let confirmed = await confirm(step)
            if !confirmed {
                onEvent("user declined → session closed, nothing pinned")
                return .cancelled
            }

            // Round 2: hold the channel for the source's decision; only a
            // clean `confirmed=1` pairs (anything else refuses closed).
            let round2 = try await det("server_negotiate_r2") {
                try wire.serverNegotiate(features: [Self.accepted])
            }
            if round2.contains(Self.confirmed) {
                onEvent("mutual confirm complete → pinning \(name) (\(sourcePin.prefix(16))…)")
                return .paired(peerName: name, peerPin: sourcePin, verification: vs)
            } else {
                onEvent("source did not send a clean confirm → REFUSED (fail closed)")
                return .refused(why: "source confirmation missing")
            }
        } catch {
            // Round-2 I/O failure (the other side declined/closed): both sides
            // end NOT paired — fail closed, never half-trust.
            onEvent("pairing channel closed before mutual confirm (\(error)) → NOT paired")
            return .refused(why: "\(error)")
        }
    }

    /// §18.4.2-5 — the source half. QR path (`payloadURI` from the
    /// destination's display): the payload carries the destination's pin, so
    /// the connect is a NORMAL pinned one and a MITM fails the handshake.
    /// Manual fallback (addr + code typed): provisional connect — then BOTH
    /// screens' verification strings are the mutual confirmation (§18.4.4-5).
    public static func joinDestination(
        identity: DeviceIdentity,
        addr: String,
        code: String,
        payloadURI: String?,
        confirm: @escaping @Sendable (PairingState) async -> Bool,
        onEvent: @escaping @Sendable (String) -> Void = { _ in }
    ) async -> PairingState {
        var destPin: String?
        var pinnedDest: TrustStore?
        if let payloadURI {
            guard let payload = try? parsePairingPayload(payloadURI) else {
                onEvent("payload unparseable/refused → REFUSED (fail closed)")
                return .refused(why: "pairing payload malformed")
            }
            if !code.isEmpty && payload.code != code {
                onEvent("typed code != payload code → REFUSED (fail closed)")
                return .refused(why: "pairing code mismatch")
            }
            do {
                let t = try TrustStore.fromJSON("{\"peers\":{}}")
                try t.pair(name: payload.deviceName, pinHex: payload.pinHex, utc: utcNow())
                pinnedDest = t
                destPin = payload.pinHex
            } catch {
                onEvent("pinning the QR payload failed → REFUSED (fail closed)")
                return .refused(why: "\(error)")
            }
            onEvent("QR payload pinned \(payload.deviceName) (\(payload.pinHex.prefix(16))…) — normal pinned connect")
        } else {
            onEvent("manual mode: provisional connect (§18.4.3) — compare BOTH verification strings")
        }

        // Copy to immutable bindings for the detached closure (Sendable).
        let pinnedTrust = pinnedDest
        let wire: NetSession
        do {
            wire = try await det("connect") {
                if let pinned = pinnedTrust {
                    return try NetSession.connect(addr: addr, identity: identity, trust: pinned)
                }
                return try NetSession.connectPairing(addr: addr, identity: identity)
            }
        } catch {
            onEvent("connect failed: \(error)")
            return .refused(why: "connect failed: \(error)")
        }
        defer { wire.teardown() }
        do {
            let dstPin: String
            if let p = destPin {
                dstPin = p
            } else {
                dstPin = try wire.peerPinHex()
            }

            // Round 1: send name + code echo (data only, §3.8).
            _ = try await det("client_negotiate") {
                try wire.clientNegotiate(features: ["name=carryon-mac", "code=\(code)"])
            }

            // Compute + show the SAME verification string the destination shows.
            let vs = try pairingVerificationString(try identity.pinHex(), dstPin)
            onEvent("verification string: \(vs) — confirm on BOTH devices (§18.4.4-5)")
            let step = PairingState.awaitingConfirm(peerName: "destination", peerPin: dstPin, verification: vs)
            let confirmed = await confirm(step)
            if !confirmed {
                onEvent("user declined → session closed, nothing pinned")
                return .cancelled
            }

            // Round 2: send OUR decision, learn THEIRS. A closed channel
            // (they refused the code / declined) fails → NOT paired.
            do {
                let accepted = try await det("client_negotiate_r2") {
                    try wire.clientNegotiate(features: [Self.confirmed])
                }
                if accepted.contains(Self.accepted) {
                    return .paired(peerName: "destination", peerPin: dstPin, verification: vs)
                } else {
                    return .refused(why: "destination did not accept")
                }
            } catch {
                onEvent("destination closed the pairing channel (\(error)) → NOT paired")
                return .refused(why: "\(error)")
            }
        } catch {
            onEvent("pairing failed (\(error))")
            return .refused(why: "\(error)")
        }
    }
}

/// Detached-blocking helper (the DemoPipeline discipline: blocking FFI on
/// Task.detached, never the main thread).
private func det<T>(_ where_: String, _ body: @escaping @Sendable () throws -> T) async throws -> T {
    try await withCheckedThrowingContinuation { cont in
        Task.detached(priority: .userInitiated) {
            do { cont.resume(returning: try body()) }
            catch { cont.resume(throwing: error) }
        }
    }
}
