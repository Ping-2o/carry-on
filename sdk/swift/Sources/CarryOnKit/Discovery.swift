/*
 * Discovery.swift — APP-LEVEL discovery pilot (plan.md §3): advertise + browse
 * `_carryon._tcp` via NWListener/NWBrowser with the §18.3 minimal TXT metadata.
 *
 * Label law (§2): this is an app-level pilot ONLY. Engine-side discovery is
 * TARGET (FRONTIER Phase 2); nothing here upgrades that claim. UI copy:
 * "App-level discovery pilot — engine-side discovery is TARGET (FRONTIER Phase 2)."
 *
 * TXT keys per §18.3 (minimal metadata ONLY — never user/session/app names,
 * object hashes, or key material):
 *   pm — protocol major version
 *   id — short instance identifier (ephemeral, per advertise session)
 *   pr — pairing-required flag
 *   dc — coarse device class (desktop|mobile|unknown)
 *   ch — ephemeral connection hint
 */

import Foundation
import Network

/// One discovered (or advertised) instance.
public struct DiscoveryPeer: Identifiable, Equatable, Sendable {
    public var id: String            // instance name from DNS-SD (stable per browse)
    public var endpoint: String      // host:port hint
    public var txt: [String: String]
    public var lastSeen: Date

    public var protocolMajor: String { txt["pm"] ?? "?" }
    public var pairingRequired: Bool { txt["pr"] == "true" || txt["pr"] == "1" }
    public var deviceClass: String { txt["dc"] ?? "unknown" }
    public var connectionHint: String { txt["ch"] ?? "" }
}

/// App-level Bonjour advertise + browse for `_carryon._tcp`.
@MainActor
@Observable
public final class DiscoveryPilot {
    public static let serviceType = "_carryon._tcp"

    public private(set) var advertising = false
    public private(set) var browsing = false
    public private(set) var advertisedName: String = ""
    public private(set) var advertisedPort: UInt16 = 0
    public private(set) var peers: [DiscoveryPeer] = []
    public private(set) var lastError: String = ""

    private var advertiser: NWListener?
    private var browser: NWBrowser?
    /// Ephemeral instance id for the TXT `id` key (not a key, not a session name).
    private let instanceID = UUID().uuidString.prefix(8).lowercased()

    public init() {}

    public static let pilotLabel =
        "App-level discovery pilot — engine-side discovery is TARGET (FRONTIER Phase 2)."

    /// §18.3 TXT record: minimal metadata only.
    func txtRecord(pairingRequired: Bool) -> NWTXTRecord {
        var t = NWTXTRecord()
        t["pm"] = "1" // CARRYON_ABI_MAJOR
        t["id"] = String(instanceID)
        t["pr"] = pairingRequired ? "true" : "false"
        t["dc"] = "desktop"
        t["ch"] = "none"
        return t
    }

    /// Advertise this app as `_carryon._tcp` on an ephemeral port. `name` is the
    /// DNS-SD instance name (a device label the user recognizes, not a secret).
    public func startAdvertising(name: String, pairingRequired: Bool = true) {
        stopAdvertising()
        let params = NWParameters.tcp
        let listener: NWListener
        do {
            listener = try NWListener(using: params, on: .any)
        } catch {
            lastError = "advertise: \(error.localizedDescription)"
            return
        }
        listener.service = NWListener.Service(
            name: name,
            type: Self.serviceType,
            txtRecord: txtRecord(pairingRequired: pairingRequired))
        listener.stateUpdateHandler = { [weak self] state in
            Task { @MainActor in
                guard let self else { return }
                switch state {
                case .ready:
                    self.advertising = true
                    self.advertisedPort = listener.port?.rawValue ?? 0
                    self.lastError = ""
                case .failed(let err):
                    self.lastError = "advertise failed: \(err)"
                    self.advertising = false
                case .cancelled:
                    self.advertising = false
                default:
                    break
                }
            }
        }
        listener.newConnectionHandler = { [weak self] conn in
            // Pilot scope: connections are listed, not driven — the engine's own
            // transport (carryon-net) owns real sessions.
            Task { @MainActor in
                self?.lastError = ""
            }
            conn.cancel()
        }
        advertiser = listener
        advertisedName = name
        listener.start(queue: .global(qos: .utility))
    }

    public func stopAdvertising() {
        advertiser?.cancel()
        advertiser = nil
        advertising = false
        advertisedPort = 0
    }

    public func startBrowsing() {
        stopBrowsing()
        let params = NWParameters()
        let b = NWBrowser(for: .bonjour(type: Self.serviceType, domain: nil), using: params)
        b.browseResultsChangedHandler = { [weak self] results, _ in
            let found: [DiscoveryPeer] = results.map { result in
                var endpoint = ""
                if case .service(let name, _, _, _) = result.endpoint {
                    endpoint = name
                }
                var txt: [String: String] = [:]
                if case .bonjour(let record) = result.metadata {
                    txt = record.dictionary
                }
                return DiscoveryPeer(id: endpoint, endpoint: endpoint, txt: txt,
                                     lastSeen: Date())
            }
            Task { @MainActor in
                self?.peers = found.sorted { $0.id < $1.id }
            }
        }
        b.stateUpdateHandler = { [weak self] state in
            Task { @MainActor in
                guard let self else { return }
                switch state {
                case .ready:
                    self.browsing = true
                    self.lastError = ""
                case .failed(let err):
                    self.lastError = "browse failed: \(err)"
                    self.browsing = false
                case .cancelled:
                    self.browsing = false
                default:
                    break
                }
            }
        }
        browser = b
        b.start(queue: .global(qos: .utility))
    }

    public func stopBrowsing() {
        browser?.cancel()
        browser = nil
        browsing = false
        peers = []
    }

    public func stopAll() {
        stopAdvertising()
        stopBrowsing()
    }
}
