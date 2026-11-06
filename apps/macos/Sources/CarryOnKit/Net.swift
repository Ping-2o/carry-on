/*
 * Net.swift — Swift binding for the transport entries (spec §18): device
 * identity (pin = SHA-256 of the cert DER), trust store (pair/revoke/JSON
 * round-trip), listener, and TLS 1.3 sessions with feature negotiation.
 *
 * Secrets law (EVD-006/§22.3): the identity KEY DER is SECRET. It is exposed
 * here ONLY as a explicit route to platform secure storage — never log it,
 * never put it in strings the UI can show, never bundle it. Only the pin hex
 * (a certificate fingerprint) may be shown, truncated in UI.
 */

import CCarryon
import Foundation

/// A device identity (`CarryonIdentity` in C).
///
/// @unchecked Sendable: raw C handle; one flow touches a given identity at a time.
public final class DeviceIdentity: @unchecked Sendable {
    public let handle: OpaquePointer
    public let name: String

    /// Generate a fresh identity (self-signed; pin = SHA-256 of cert DER, §18.4).
    public static func generate(name: String) throws -> DeviceIdentity {
        let h = name.withCString { carryon_identity_generate($0) }
        guard let handle = h else { throw CarryonError.nullReturn("carryon_identity_generate") }
        return DeviceIdentity(handle: handle, name: name)
    }

    /// Rehydrate from persisted DER bytes (stable pin across launches, §18.4).
    /// The key DER is SECRET — read it from secure storage only.
    public static func fromDER(name: String, cert: Data, key: Data) throws -> DeviceIdentity {
        let h = cert.withUnsafeBytes { cb in
            key.withUnsafeBytes { kb in
                name.withCString { nc in
                    carryon_identity_from_der(nc, cb.bindMemory(to: UInt8.self).baseAddress,
                                              cert.count, kb.bindMemory(to: UInt8.self).baseAddress,
                                              key.count)
                }
            }
        }
        guard let handle = h else { throw CarryonError.nullReturn("carryon_identity_from_der") }
        return DeviceIdentity(handle: handle, name: name)
    }

    init(handle: OpaquePointer, name: String) {
        self.handle = handle
        self.name = name
    }

    deinit { carryon_identity_free(handle) }

    /// The 64-hex pin (SHA-256 of the cert DER). A fingerprint, not a secret.
    public func pinHex() throws -> String {
        try outString("carryon_identity_pin_hex") { out in
            carryon_identity_pin_hex(handle, out)
        }
    }

    /// UI-safe truncated pin: first 12 hex + "…" (never more in display).
    public func truncatedPin() throws -> String {
        String(try pinHex().prefix(12)) + "…"
    }

    /// Certificate DER (public).
    public func certDER() throws -> Data {
        try queryBytes("carryon_identity_cert_der") { buf, len in
            carryon_identity_cert_der(handle, buf, len)
        }
    }

    /// PRIVATE KEY DER — **SECRET** (EVD-006/§22.3). Route straight to platform
    /// secure storage (Keychain §8.7). NEVER log, display, or bundle this.
    public func keyDER() throws -> Data {
        try queryBytes("carryon_identity_key_der") { buf, len in
            carryon_identity_key_der(handle, buf, len)
        }
    }

    /// The `*len`-query byte-copy protocol: null buffer pass returns
    /// CARRYON_ERR_BUFFER_TOO_SMALL with *len = required size.
    private func queryBytes(_ where_: String,
                            _ call: (UnsafeMutablePointer<UInt8>?, UnsafeMutablePointer<Int>?) -> Int32) throws -> Data {
        var len: Int = 0
        let q = call(nil, &len)
        guard q == CARRYON_ERR_BUFFER_TOO_SMALL, len > 0 else {
            try CarryonError.check(q, where_)
            return Data()
        }
        var buf = [UInt8](repeating: 0, count: len)
        try CarryonError.check(call(&buf, &len), where_)
        buf.removeSubrange(min(len, buf.count)..<buf.count)
        return Data(buf)
    }
}

/// One trust record (`TrustRecord` in carryon-net), as persisted in the
/// TrustStore JSON: device_name, pin_hex, trust_generation, revoked, paired_utc.
public struct TrustRecord: Codable, Sendable, Equatable, Identifiable {
    public var device_name: String
    public var pin_hex: String
    public var trust_generation: Int
    public var revoked: Bool
    public var paired_utc: String

    public var id: String { pin_hex }

    /// UI-safe truncated pin (12 hex + …).
    public var truncatedPin: String { String(pin_hex.prefix(12)) + "…" }

    private enum CodingKeys: String, CodingKey {
        case device_name, pin_hex, trust_generation, revoked, paired_utc
    }

    public init(device_name: String, pin_hex: String, trust_generation: Int,
                revoked: Bool, paired_utc: String) {
        self.device_name = device_name
        self.pin_hex = pin_hex
        self.trust_generation = trust_generation
        self.revoked = revoked
        self.paired_utc = paired_utc
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        device_name = try c.decodeIfPresent(String.self, forKey: .device_name) ?? "peer"
        pin_hex = try c.decodeIfPresent(String.self, forKey: .pin_hex) ?? ""
        trust_generation = try c.decodeIfPresent(Int.self, forKey: .trust_generation) ?? 0
        revoked = try c.decodeIfPresent(Bool.self, forKey: .revoked) ?? false
        paired_utc = try c.decodeIfPresent(String.self, forKey: .paired_utc) ?? ""
    }
}

/// The trust store (`CarryonTrust` in C) — pins peers by cert fingerprint.
///
/// @unchecked Sendable: raw C handle; pair/revoke are serialized by the caller.
public final class TrustStore: @unchecked Sendable {
    public let handle: OpaquePointer

    /// Build from JSON (`{"peers":{…}}`); used for both halves of a pairing and
    /// for persisted stores.
    public static func fromJSON(_ json: String) throws -> TrustStore {
        let h = json.withCString { carryon_trust_from_json($0) }
        guard let handle = h else { throw CarryonError.nullReturn("carryon_trust_from_json") }
        return TrustStore(handle: handle)
    }

    init(handle: OpaquePointer) {
        self.handle = handle
    }

    deinit { carryon_trust_free(handle) }

    /// Serialize for persistence (app storage; contains pins, never keys).
    public func toJSON() throws -> String {
        try outString("carryon_trust_to_json") { out in
            carryon_trust_to_json(handle, out)
        }
    }

    /// Record a newly paired peer by pin hex (§18.4.6-7).
    public func pair(name: String, pinHex: String, utc: String) throws {
        try name.withCString { nc in
            try pinHex.withCString { pc in
                try utc.withCString { uc in
                    try CarryonError.check(carryon_trust_pair(handle, nc, pc, uc),
                                           "carryon_trust_pair")
                }
            }
        }
    }

    /// Whether `pinHex` is currently trusted.
    public func isTrusted(pinHex: String) throws -> Bool {
        var out = false
        try pinHex.withCString { pc in
            try CarryonError.check(carryon_trust_is_trusted(handle, pc, &out),
                                   "carryon_trust_is_trusted")
        }
        return out
    }

    /// Revoke by pin (§18.4.9): increments trust generation, blocks reconnect.
    @discardableResult
    public func revoke(pinHex: String) throws -> Bool {
        var out = false
        try pinHex.withCString { pc in
            try CarryonError.check(carryon_trust_revoke(handle, pc, &out), "carryon_trust_revoke")
        }
        return out
    }

    /// Parsed snapshot of the records (for UI). Pins are fingerprints.
    public func records() throws -> [TrustRecord] {
        let json = try toJSON()
        struct Peers: Codable { var peers: [String: TrustRecord] }
        guard let peers = try? JSONDecoder().decode(Peers.self, from: Data(json.utf8)) else {
            return []
        }
        return peers.peers.values.sorted { $0.device_name < $1.device_name }
    }
}

/// The result of `carryon_pair_devices`: two serialized TrustStores (mutual pin).
public struct PairingResult: Sendable {
    public var localTrustJSON: String
    public var remoteTrustJSON: String
}

/// Pair two in-process identities (loopback/testing convenience, §18.4): each
/// side gets a TrustStore pinning the other. `utc` labels the pairing record.
public func pairDevices(_ a: DeviceIdentity, _ b: DeviceIdentity, utc: String) throws -> PairingResult {
    let json = try utc.withCString { uc in
        try outString("carryon_pair_devices") { out in
            carryon_pair_devices(a.handle, b.handle, uc, out)
        }
    }
    guard let v = parseAnyJSON(json), case .object(let o) = v,
          let lt = o["local_trust"]?.jsonEncoded, let rt = o["remote_trust"]?.jsonEncoded else {
        throw CarryonError(code: -4, message: "unparseable pairing json: \(json.prefix(400))")
    }
    // Round-trip each half through carryon_trust_from_json per the plan (the
    // stores used on the wire must be the parsed engine-native form).
    return PairingResult(localTrustJSON: lt, remoteTrustJSON: rt)
}

/// A TCP/TLS listener (`CarryonListener` in C).
///
/// @unchecked Sendable: raw C handle.
public final class NetListener: @unchecked Sendable {
    public let handle: OpaquePointer

    /// Bind, e.g. "127.0.0.1:0" (ephemeral port).
    public static func bind(_ addr: String) throws -> NetListener {
        let h = addr.withCString { carryon_listener_bind($0) }
        guard let handle = h else { throw CarryonError.nullReturn("carryon_listener_bind") }
        return NetListener(handle: handle)
    }

    init(handle: OpaquePointer) {
        self.handle = handle
    }

    deinit { carryon_listener_free(handle) }

    /// The bound `host:port` address.
    public func addr() throws -> String {
        try outString("carryon_listener_addr") { out in
            carryon_listener_addr(handle, out)
        }
    }
}

/// A TLS 1.3 transport session (`CarryonSession` in C) with mutual cert pinning.
///
/// @unchecked Sendable: raw C handle. The C ABI (carryon.h) guarantees NO
/// thread-safety for a CarryonSession — it must not be aliased across threads
/// (handle.rs as_mut contract: "not aliased elsewhere for the duration of the
/// borrow"). All short handle access here (negotiate, counters, teardown) is
/// serialized by `lock`; long wire calls (import/serve/authority) are confined
/// to one flow by the pipeline, and the demo's byte-counter poller is JOINED
/// (cancel + await) before any teardown free (see DemoPipeline.retirePoller).
public final class NetSession: @unchecked Sendable {
    private let lock = NSLock()
    var handle: OpaquePointer?

    /// Connect as client to `addr`, pinning the server against `trust` (§18.5).
    public static func connect(addr: String, identity: DeviceIdentity,
                               trust: TrustStore) throws -> NetSession {
        let h = addr.withCString { ac in
            carryon_session_connect(ac, identity.handle, trust.handle)
        }
        guard let handle = h else { throw CarryonError.nullReturn("carryon_session_connect") }
        return NetSession(handle: handle)
    }

    /// Accept one connection as server, pinning the client.
    public static func accept(listener: NetListener, identity: DeviceIdentity,
                              trust: TrustStore) throws -> NetSession {
        let h = carryon_session_accept(listener.handle, identity.handle, trust.handle)
        guard let handle = h else { throw CarryonError.nullReturn("carryon_session_accept") }
        return NetSession(handle: handle)
    }

    init(handle: OpaquePointer) {
        self.handle = handle
    }

    deinit {
        lock.lock()
        defer { lock.unlock() }
        if let handle { carryon_session_free(handle) }
    }

    /// Close the transport early (source-off continuation, per carryon_l3.c):
    /// frees the underlying handle immediately; idempotent. The durable
    /// authority receipts are NOT affected (§21.2).
    public func teardown() {
        lock.lock()
        defer { lock.unlock() }
        if let handle {
            carryon_session_free(handle)
            self.handle = nil
        }
    }

    /// Client negotiation: send Hello(features), expect Welcome; returns the
    /// peer's feature list.
    public func clientNegotiate(features: [String]) throws -> [String] {
        try negotiate(features: features) { f, out in
            lock.lock(); defer { lock.unlock() }
            if let h = handle { return carryon_session_client_negotiate(h, f, out) }
            return -5
        }
    }

    /// Server negotiation: expect Hello, reply Welcome(features).
    public func serverNegotiate(features: [String]) throws -> [String] {
        try negotiate(features: features) { f, out in
            lock.lock(); defer { lock.unlock() }
            if let h = handle { return carryon_session_server_negotiate(h, f, out) }
            return -5
        }
    }

    private func negotiate(features: [String],
                           _ call: (UnsafePointer<CChar>?, UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> Int32) throws -> [String] {
        let featuresJSON = (try? JSONEncoder().encode(features)).flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
        let theirs = try featuresJSON.withCString { fc in
            try outString("carryon_session_negotiate") { out in
                call(fc, out)
            }
        }
        guard let arr = try? JSONDecoder().decode([String].self, from: Data(theirs.utf8)) else {
            return []
        }
        return arr
    }

    /// Measured wire-byte counters (length prefix + body; real, not estimates).
    public func byteCounters() throws -> (sent: UInt64, recv: UInt64) {
        lock.lock()
        defer { lock.unlock() }
        guard let handle else { return (0, 0) }
        var sent: UInt64 = 0, recv: UInt64 = 0
        try CarryonError.check(carryon_session_bytes(handle, &sent, &recv), "carryon_session_bytes")
        return (sent, recv)
    }
}
