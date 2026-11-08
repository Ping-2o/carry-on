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
    static var send = SendP

[Truncated after 242 lines. To continue, call readFile again with offset=243.]