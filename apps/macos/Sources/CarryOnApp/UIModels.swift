/*
 * UIModels.swift — presentation models for the redesigned panes.
 *
 * Screens render these value types instead of reaching into engine models, so
 * the same views can be driven by live state or by deterministic fixtures
 * (the `--snapshots` renderer used by CI screenshots). Live derivation is a
 * pure read of AppState — no engine call happens here.
 */

import AppKit
import CarryOnKit
import Foundation
import SwiftUI

// MARK: - Devices

struct DeviceItem: Identifiable, Hashable {
    enum Kind: Hashable {
        case laptop, desktop, phone, tablet, unknown

        init(deviceClass: String) {
            switch deviceClass.lowercased() {
            case "mobile", "phone", "handset": self = .phone
            case "tablet": self = .tablet
            case "laptop", "notebook": self = .laptop
            case "desktop", "workstation", "mac": self = .desktop
            default: self = .unknown
            }
        }

        /// Best-effort guess from a human device name (trust records carry no class).
        init(guessingFrom name: String) {
            let n = name.lowercased()
            if n.contains("pixel") || n.contains("phone") || n.contains("galaxy") || n.contains("android") {
                self = .phone
            } else if n.contains("ipad") || n.contains("tab") {
                self = .tablet
            } else if n.contains("macbook") || n.contains("laptop") {
                self = .laptop
            } else if n.contains("imac") || n.contains("mac mini") || n.contains("studio") || n.contains("desktop") {
                self = .desktop
            } else {
                self = .unknown
            }
        }
    }

    enum Status: Hashable {
        case nearby, paired, revoked, thisDevice

        var title: String {
            switch self {
            case .nearby: return "Nearby"
            case .paired: return "Paired"
            case .revoked: return "Revoked"
            case .thisDevice: return "This Mac"
            }
        }

        var symbol: String {
            switch self {
            case .nearby: return "wifi"
            case .paired: return "checkmark.shield.fill"
            case .revoked: return "xmark.shield"
            case .thisDevice: return "person.crop.circle"
            }
        }

        var tone: StatusTone {
            switch self {
            case .nearby: return .info
            case .paired: return .success
            case .revoked: return .danger
            case .thisDevice: return .neutral
            }
        }
    }

    let id: String
    let name: String
    let kind: Kind
    let detail: String
    let status: Status
    /// host:port when the device is reachable (discovered); nil otherwise.
    let address: String?
    /// Truncated public pin for paired devices.
    var pin: String?

    var symbol: String {
        switch kind {
        case .laptop: return "laptopcomputer"
        case .desktop: return "desktopcomputer"
        case .phone: return "iphone"
        case .tablet: return "ipad"
        case .unknown: return "display"
        }
    }
}

// MARK: - Sessions

struct SessionItem: Identifiable, Hashable {
    enum Role: Hashable {
        case sent, received, prepared, demoSource, demoDestination

        var title: String {
            switch self {
            case .sent: return "Sent"
            case .received: return "Received"
            case .prepared: return "Prepared"
            case .demoSource: return "Demo · source"
            case .demoDestination: return "Demo · destination"
            }
        }
    }

    let id: String
    let title: String
    let subtitle: String
    let state: SessionState
    let role: Role
    var when: Date?

    var tone: StatusTone { state.tone }
    var symbol: String {
        switch role {
        case .sent: return "paperplane.fill"
        case .received: return "tray.and.arrow.down.fill"
        case .prepared: return "shippingbox.fill"
        case .demoSource, .demoDestination: return "play.rectangle.fill"
        }
    }
}

extension SessionState {
    /// Human explanation shown next to the verbatim §16.2 state name.
    var explanation: String {
        switch self {
        case .idle: return "Not started"
        case .preparing: return "Sealing a cut of the session"
        case .cutProposed: return "Cut proposed, waiting for seal"
        case .cutSealed: return "Sealed and ready to transfer"
        case .importing: return "Arriving on the other device"
        case .actionReady: return "Opened on the other device"
        case .readOnlyActive: return "Open read-only on the other device"
        case .failed: return "Transfer failed — nothing was opened"
        case .inconclusive: return "Outcome could not be verified"
        case .aborted: return "Cancelled"
        }
    }

    var tone: StatusTone {
        switch self {
        case .actionReady, .readOnlyActive: return .success
        case .cutSealed, .importing, .preparing, .cutProposed: return .info
        case .failed: return .danger
        case .inconclusive, .aborted: return .warning
        case .idle: return .neutral
        }
    }
}

// MARK: - Send flow

struct PreparedItem: Identifiable, Hashable {
    let id: UUID
    let fileName: String
    let detail: String
    let state: SessionState
}

struct SendPresentation {
    var fileName: String?
    var fileDetail: String?
    var filePath: String?
    var prepared: [PreparedItem] = []
    var selectedPreparedID: UUID?
    var devices: [DeviceItem] = []
    var receiverAddress = ""
    var usePairedTrust = true
    var hasPairedDevices = false
    var preparing = false
    var running = false
    var bytesSent: UInt64 = 0
    var bytesReceived: UInt64 = 0
    var elapsedMS: Double?
    var verified: Bool?
    var legsUsed: Int?
    var opened: StageOutcome?
    var carried: StageOutcome?
    var evidenceClass: String?
    var error: String?
    var canPrepare = false
    var canSend = false

    var selectedDevice: DeviceItem? {
        devices.first { $0.address == receiverAddress }
    }

    var destinationName: String {
        if let d = selectedDevice { return d.name }
        return receiverAddress.isEmpty ? "No device selected" : receiverAddress
    }

    var steps: [FlowStep] {
        let fileDone = fileName != nil
        let preparedDone = selectedPreparedID != nil
        let deviceDone = !receiverAddress.isEmpty
        let sentDone = verified == true
        let openedDone: Bool = {
            if case .ok = opened { return true }
            return false
        }()
        let failed = error != nil
        func state(_ done: Bool, active: Bool) -> FlowStepState {
            done ? .done : (active ? .active : .pending)
        }
        return [
            FlowStep(id: 0, title: "File", systemImage: "doc", state: state(fileDone, active: !fileDone)),
            FlowStep(id: 1, title: "Prepare", systemImage: "shippingbox",
                     state: state(preparedDone, active: fileDone && !preparedDone)),
            FlowStep(id: 2, title: "Device", systemImage: "laptopcomputer.and.iphone",
                     state: state(deviceDone, active: preparedDone && !deviceDone)),
            FlowStep(id: 3, title: "Send", systemImage: "paperplane",
                     state: failed ? .failed : state(sentDone, active: preparedDone && deviceDone && !sentDone)),
            FlowStep(id: 4, title: "Opened", systemImage: "checkmark.seal",
                     state: state(openedDone, active: sentDone && !openedDone)),
        ]
    }
}

// MARK: - Fixtures (snapshot renderer)

/// Deterministic presentation data for CI screenshots. Inactive in normal runs.
@MainActor
enum UIFixtures {
    static var isActive = false
    static var localName = "Studio MacBook Pro"
    static var localPin = "7f3a9c21d04e…"
    static var nearby: [DeviceItem] = []
    static var paired: [DeviceItem] = []
    static var sessions: [SessionItem] = []
    static var send = SendPresentation()

    static func install() {
        isActive = true
        let now = Date()
        nearby = [
            DeviceItem(id: "n1", name: "Pixel 9 Pro", kind: .phone, detail: "192.168.1.24:47110 · Android",
                       status: .nearby, address: "192.168.1.24:47110"),
            DeviceItem(id: "n2", name: "Galaxy Tab S10", kind: .tablet, detail: "192.168.1.31:47110 · Android",
                       status: .nearby, address: "192.168.1.31:47110"),
        ]
        paired = [
            DeviceItem(id: "p1", name: "Pixel 9 Pro", kind: .phone, detail: "Paired 3 Nov 2026 · generation 2",
                       status: .paired, address: nil, pin: "c41e0b9a77d2…"),
            DeviceItem(id: "p2", name: "Mac mini (Office)", kind: .desktop, detail: "Paired 28 Oct 2026 · generation 1",
                       status: .paired, address: nil, pin: "09ab44f1c3e8…"),
            DeviceItem(id: "p3", name: "Old Nexus", kind: .phone, detail: "Revoked 12 Oct 2026",
                       status: .revoked, address: nil, pin: "5d0e81aa2b6f…"),
        ]
        sessions = [
            SessionItem(id: "s1", title: "Quarterly-plan.key", subtitle: "To Pixel 9 Pro · 4.8 MB · verified",
                        state: .actionReady, role: .sent, when: now.addingTimeInterval(-6 * 60)),
            SessionItem(id: "s2", title: "Field-notes.md", subtitle: "Sealed cut #3 · waiting for a device",
                        state: .cutSealed, role: .prepared, when: now.addingTimeInterval(-42 * 60)),
            SessionItem(id: "s3", title: "Kiwix · wikipedia_en_top", subtitle: "From Galaxy Tab S10 · read-only",
                        state: .readOnlyActive, role: .received, when: now.addingTimeInterval(-3 * 3600)),
            SessionItem(id: "s4", title: "Poster-draft.psd", subtitle: "To Mac mini (Office) · integrity check refused",
                        state: .failed, role: .sent, when: now.addingTimeInterval(-26 * 3600)),
        ]
        let prepID = UUID(uuidString: "8B0C6C55-1F7C-4B52-9E0A-3E7C2B0D9A11")!
        send = SendPresentation(
            fileName: "Quarterly-plan.key",
            fileDetail: "Keynote presentation · 4.8 MB",
            filePath: "~/Documents/Quarterly-plan.key",
            prepared: [PreparedItem(id: prepID, fileName: "Quarterly-plan.key", detail: "Cut #1 · 4.8 MB",
                                    state: .cutSealed)],
            selectedPreparedID: prepID,
            devices: nearby,
            receiverAddress: "192.168.1.24:47110",
            usePairedTrust: true,
            hasPairedDevices: true,
            preparing: false,
            running: false,
            bytesSent: 5_033_165,
            bytesReceived: 5_033_165,
            elapsedMS: 412,
            verified: true,
            legsUsed: 1,
            opened: .ok,
            carried: .ok,
            evidenceClass: "physical",
            error: nil,
            canPrepare: true,
            canSend: true)
    }
}

// MARK: - Live derivation

extension AppState {
    var localDeviceName: String {
        UIFixtures.isActive ? UIFixtures.localName : (Host.current().localizedName ?? "This Mac")
    }

    var localPinShort: String {
        UIFixtures.isActive ? UIFixtures.localPin : carry.durablePinTruncated
    }

    var nearbyDevices: [DeviceItem] {
        if UIFixtures.isActive { return UIFixtures.nearby }
        return discovery.peers.map { peer in
            DeviceItem(id: peer.id,
                       name: DeviceItem.displayName(fromInstance: peer.instanceName),
                       kind: DeviceItem.Kind(deviceClass: peer.txt.device_class),
                       detail: "\(peer.address) · \(peer.txt.device_class)",
                       status: .nearby,
                       address: peer.address)
        }
    }

    var pairedDevices: [DeviceItem] {
        if UIFixtures.isActive { return UIFixtures.paired }
        return topology.records.map { r in
            DeviceItem(id: r.pin_hex,
                       name: r.device_name,
                       kind: DeviceItem.Kind(guessingFrom: r.device_name),
                       detail: r.revoked ? "Revoked" : "Paired \(DeviceItem.friendlyDate(r.paired_utc)) · generation \(r.trust_generation)",
                       status: r.revoked ? .revoked : .paired,
                       address: nil,
                       pin: r.truncatedPin)
        }
    }

    var sessionItems: [SessionItem] {
        if UIFixtures.isActive { return UIFixtures.sessions }
        var items: [SessionItem] = carry.armedSessions.map { s in
            SessionItem(id: s.id.uuidString,
                        title: s.title,
                        subtitle: s.subtitle,
                        state: s.state,
                        role: s.servedResult == nil ? .prepared : .sent,
                        when: s.preparedAt)
        }
        items += demo.sourceSessions.map { s in
            SessionItem(id: "src-" + s.id, title: "Demo session", subtitle: s.id,
                        state: s.state, role: .demoSource, when: nil)
        }
        items += demo.destSessions.map { s in
            SessionItem(id: "dst-" + s.id, title: "Demo session", subtitle: s.id,
                        state: s.state, role: .demoDestination, when: nil)
        }
        return items
    }

    var sendPresentation: SendPresentation {
        if UIFixtures.isActive { return UIFixtures.send }
        let c = carry
        var p = SendPresentation()
        if let url = c.fileURL {
            p.fileName = url.lastPathComponent
            p.filePath = (url.path as NSString).abbreviatingWithTildeInPath
            let values = try? url.resourceValues(forKeys: [.fileSizeKey, .localizedTypeDescriptionKey])
            let size = values?.fileSize ?? 0
            let kind = values?.localizedTypeDescription
            p.fileDetail = [kind, CarryFormat.bytes(UInt64(max(size, 0)))].compactMap { $0 }.joined(separator: " · ")
        }
        p.prepared = c.armedSessions.map {
            PreparedItem(id: $0.id, fileName: $0.title, detail: $0.subtitle, state: $0.state)
        }
        p.selectedPreparedID = c.selectedArmedID
        p.devices = nearbyDevices
        p.receiverAddress = c.receiverAddr
        p.usePairedTrust = c.usePairedTrust
        p.hasPairedDevices = !topology.records.filter { !$0.revoked }.isEmpty
        p.preparing = c.preparing
        p.running = c.running
        p.bytesSent = c.bytesSent
        p.bytesReceived = c.bytesRecv
        if let r = c.lastResult {
            p.elapsedMS = r.elapsedMS
            p.verified = r.verifyOK
            p.legsUsed = Int(r.serveLegsUsed)
        }
        p.carried = c.ladderStates[.l0Carry]
        p.opened = c.ladderStates[.l1Open]
        p.evidenceClass = c.evidenceClass
        p.error = c.lastError
        p.canPrepare = c.prepareValid && !c.preparing && !c.running
        p.canSend = c.transferValid && !c.running
        return p
    }
}

extension DeviceItem {
    /// DNS-SD instance names look like "Pixel-9-Pro (carryon)"; keep it human.
    static func displayName(fromInstance raw: String) -> String {
        var s = raw
        if let r = s.range(of: " (") { s = String(s[..<r.lowerBound]) }
        return s.replacingOccurrences(of: "-", with: " ")
    }

    static func friendlyDate(_ utc: String) -> String {
        let iso = ISO8601DateFormatter()
        if let d = iso.date(from: utc) {
            return d.formatted(date: .abbreviated, time: .omitted)
        }
        if let secs = Double(utc) {
            return Date(timeIntervalSince1970: secs).formatted(date: .abbreviated, time: .omitted)
        }
        return utc
    }
}
