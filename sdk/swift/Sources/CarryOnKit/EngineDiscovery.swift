/*
 * EngineDiscovery.swift — ENGINE-SIDE discovery (stage 4): the Rust engine owns
 * the §18.3 TXT contract (construction, validation, fail-closed receive filter)
 * and, on macOS, the live mDNS advertise/browse (mdns-sd) behind the C ABI.
 *
 * What changed vs the app-level pilot (Discovery.swift, kept as a pilot artifact):
 * the browser and advertiser are the engine's; the app only polls snapshots and
 * renders. A received TXT record is validated by the ENGINE (E_PLATFORM_DISCOVERY
 * on a malformed record — the shell drops the peer, never repairs it), foreign
 * TXT keys are ignored by the engine and surfaced only as diagnostics, and peers
 * advertising a different protocol major are filtered before they are ever shown.
 *
 * Lifecycle is FOREGROUND-FIRST (battery): the shell starts/stops the advertiser
 * and browser from screen appear/disappear. Nothing here self-schedules, and the
 * browse cache TTL prunes stale peers on every snapshot.
 *
 * Label (§2 law): a discovered peer is a NETWORK FACT, not a pairing. Connecting
 * still pins the peer's certificate fingerprint (manual pin until stage-5
 * pairing). No platform is "supported" (§30 unrun).
 */

import CCarryon
import Foundation

// ---- the engine-owned TXT contract ------------------------------------------

/// The validated fields of a §18.3 TXT record (engine form).
public struct TxtFields: Codable, Equatable, Sendable {
    public var protocol_major: Int
    public var instance_id: String
    public var pairing_required: Bool
    public var device_class: String
    public var connection_hint: String

    public init(protocolMajor: Int, instanceID: String, pairingRequired: Bool,
                deviceClass: String, connectionHint: String) {
        self.protocol_major = protocolMajor
        self.instance_id = instanceID
        self.pairing_required = pairingRequired
        self.device_class = deviceClass
        self.connection_hint = connectionHint
    }
}

public enum EngineDiscoveryContractError: Error, CustomStringConvertible {
    case engine(code: Int32, message: String)

    public var description: String {
        switch self {
        case .engine(let code, let message): return "discovery contract (code \(code)): \(message)"
        }
    }
}

/// The §18.3 TXT contract as exposed by the engine (ABI 1.2). Both directions
/// go through the engine — the shell never constructs or interprets TXT itself.
public enum EngineDiscoveryContract {
    /// Canonical wire pairs (`[[key,value],…]`, exactly pm/id/pr/dc/ch) for
    /// these field values. Throws on a contract violation (e.g. a device class
    /// that is not desktop|mobile|unknown).
    public static func encodeTxt(fields: TxtFields) throws -> String {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .useDefaultKeys
        let data = try encoder.encode(fields)
        let json = String(decoding: data, as: UTF8.self)
        return try outString("carryon_discovery_txt_encode") { out in
            json.withCString { carryon_discovery_txt_encode($0, out) }
        }
    }

    /// Validate a RECEIVED record's pairs. Returns the engine-validated fields
    /// plus the foreign keys the engine ignored. Throws (E_PLATFORM_DISCOVERY)
    /// on any missing/malformed required key — the caller MUST drop the peer.
    public static func validateTxt(pairs: [(String, String)]) throws
        -> (fields: TxtFields, ignoredKeys: [String]) {
        let json = try String(
            decoding: JSONEncoder().encode(pairs.map { [$0.0, $0.1] }), as: UTF8.self)
        let reply = try outString("carryon_discovery_txt_validate") { out in
            json.withCString { carryon_discovery_txt_validate($0, out) }
        }
        struct Validated: Codable {
            var txt: TxtFields
            var ignored_keys: [String]
        }
        let v = try JSONDecoder().decode(Validated.self, from: Data(reply.utf8))
        return (v.txt, v.ignored_keys)
    }
}

// ---- one discovered peer ------------------------------------------------------

/// One engine-validated peer (what browsing shows). `address` comes from the
/// engine's SRV resolution; it is a dial CANDIDATE, revalidated by the connect
/// itself (a failed TLS handshake drops the peer — discovery never vouches).
public struct EnginePeer: Identifiable, Equatable, Sendable {
    public var id: String { txt.instance_id }
    public var instanceName: String  // DNS-SD instance fullname base
    public var address: String       // host:port (resolved)
    public var txt: TxtFields
    /// Foreign TXT keys the engine observed and ignored (diagnostics only).
    public var ignoredKeys: [String]
}

// ---- the advertiser / browser handles ----------------------------------------

/// Engine-side advertiser (`CarryonDiscoveryAdvertiser`). One per foreground
/// screen; `stop()` is idempotent-ish (handle freed exactly once).
public final class EngineDiscoveryAdvertiser: @unchecked Sendable {
    private var handle: OpaquePointer?
    private let lock = NSLock()

    public init(name: String, port: UInt16, txtPairsJSON: String) throws {
        let h = name.withCString { nc in
            txtPairsJSON.withCString { tc in
                carryon_discovery_advertise_start(nc, port, tc)
            }
        }
        guard let handle = h else {
            throw CarryonError.nullReturn("carryon_discovery_advertise_start")
        }
        self.handle = handle
    }

    deinit { stop() }

    public func stop() {
        lock.lock()
        defer { lock.unlock() }
        if let handle {
            carryon_discovery_advertise_stop(handle)
            self.handle = nil
        }
    }
}

/// Engine-side browser (`CarryonDiscoveryBrowser`) with the engine's TTL cache.
public final class EngineDiscoveryBrowser: @unchecked Sendable {
    private var handle: OpaquePointer?
    private let lock = NSLock()

    public init() throws {
        let h = carryon_discovery_browse_start()
        guard let handle = h else {
            throw CarryonError.nullReturn("carryon_discovery_browse_start")
        }
        self.handle = handle
    }

    deinit { stop() }

    /// One poll of the engine's view: drained, TTL-pruned, filtered peers plus
    /// honest rejection diagnostics.
    public func snapshot() throws -> (peers: [EnginePeer], rejections: [String]) {
        struct Peer: Codable {
            var instance: String
            var instance_id: String
            var address: String
            var txt: TxtFields
        }
        struct FK: Codable {
            var instance: String
            var keys: [String]
        }
        struct Reply: Codable {
            var peers: [Peer]
            var rejected: [String]
            var foreign_keys: [FK]
        }
        let json: String = {
            lock.lock()
            defer { lock.unlock() }
            guard let handle else { return "{}" }
            return (try? outString("carryon_discovery_browse_snapshot") { out in
                carryon_discovery_browse_snapshot(handle, out)
            }) ?? "{}"
        }()
        let reply = try JSONDecoder().decode(Reply.self, from: Data(json.utf8))
        // The instance NAME (DNS-SD label) is the fullname prefix; rejections
        // and foreign-key rows carry fullnames like
        // "carryon-ab12cd34._carryon._tcp.local.".
        func nameBase(_ fullname: String) -> String {
            fullname.split(separator: ".", maxSplits: 1).first.map(String.init) ?? fullname
        }
        var foreign: [String: [String]] = [:]
        for f in reply.foreign_keys { foreign[nameBase(f.instance), default: []] += f.keys }
        let peers = reply.peers.map { p in
            EnginePeer(
                instanceName: nameBase(p.instance),
                address: p.address,
                txt: p.txt,
                ignoredKeys: foreign[nameBase(p.instance)] ?? [])
        }
        return (peers, reply.rejected.map(nameBase))
    }

    public func stop() {
        lock.lock()
        defer { lock.unlock() }
        if let handle {
            carryon_discovery_browse_stop(handle)
            self.handle = nil
        }
    }
}

// ---- the observable model both screens share ----------------------------------

/// Snapshot of the model's own state for tests and UI copy.
@MainActor
@Observable
public final class EngineDiscovery {
    public private(set) var browsing = false
    public private(set) var advertising = false
    public private(set) var advertisedName = ""
    public private(set) var peers: [EnginePeer] = []
    public private(set) var rejections: [String] = []
    public private(set) var lastError = ""

    /// NET-003 gate: wall time from `startBrowsing()` to the FIRST validated
    /// peer. Measured with a monotonic clock (spec §18.10), shown in the UI and
    /// recorded in the physical-run evidence.
    public private(set) var firstPeerLatencyMs: Double?

    private var browser: EngineDiscoveryBrowser?
    private var advertiser: EngineDiscoveryAdvertiser?
    private var poll: Task<Void, Never>?
    private var browseStartedAt: ContinuousClock.Instant?

    /// Ephemeral §18.3 instance id (8 hex, NOT a user/device name).
    private let instanceID = UUID().uuidString.prefix(8).lowercased()

    public init() {}

    public var instanceName: String { "carryon-\(instanceID)" }

    // ---- browse ----

    public func startBrowsing() {
        guard !browsing else { return }
        do {
            browser = try EngineDiscoveryBrowser()
            browsing = true
            lastError = ""
            firstPeerLatencyMs = nil
            browseStartedAt = ContinuousClock.now
            poll = Task { [weak self] in
                while !Task.isCancelled {
                    self?.pollSnapshot()
                    try? await Task.sleep(nanoseconds: 500_000_000)
                }
            }
        } catch {
            lastError = "browse: \(error.localizedDescription)"
        }
    }

    public func stopBrowsing() {
        poll?.cancel()
        poll = nil
        browser?.stop()
        browser = nil
        browsing = false
        peers = []
        rejections = []
    }

    private func pollSnapshot() {
        guard let browser else { return }
        do {
            let snap = try browser.snapshot()
            peers = snap.peers
            rejections = snap.rejections
            if firstPeerLatencyMs == nil, !peers.isEmpty, let started = browseStartedAt {
                let d = ContinuousClock.now - started
                firstPeerLatencyMs =
                    Double(d.components.seconds) * 1_000
                    + Double(d.components.attoseconds) / 1e15
            }
            lastError = ""
        } catch {
            lastError = "snapshot: \(error.localizedDescription)"
        }
    }

    // ---- advertise ----

    /// Advertise presence (the engine validates + publishes the §18.3 minimal
    /// TXT). `port` 0 advertises presence only (`ch` = "none").
    @discardableResult
    public func startAdvertising(pairingRequired: Bool = true, port: UInt16 = 0) -> Bool {
        stopAdvertising()
        do {
            let fields = TxtFields(
                protocolMajor: 1,
                instanceID: String(instanceID),
                pairingRequired: pairingRequired,
                deviceClass: "desktop",
                connectionHint: port == 0 ? "none" : String(port))
            let pairsJSON = try EngineDiscoveryContract.encodeTxt(fields: fields)
            let a = try EngineDiscoveryAdvertiser(
                name: instanceName, port: port, txtPairsJSON: pairsJSON)
            advertiser = a
            advertising = true
            advertisedName = instanceName
            lastError = ""
            return true
        } catch {
            lastError = "advertise: \(error.localizedDescription)"
            return false
        }
    }

    public func stopAdvertising() {
        advertiser?.stop()
        advertiser = nil
        advertising = false
        advertisedName = ""
    }

    public func stopAll() {
        stopBrowsing()
        stopAdvertising()
    }
}
