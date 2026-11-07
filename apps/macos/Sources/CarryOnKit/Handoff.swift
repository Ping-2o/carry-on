/*
 * Handoff.swift — the L0/L1 file handoff: a REAL file, picked by the user, is
 * captured by the compiled-in file adapter (§10.7 content-hash identity),
 * carried over LAN TLS 1.3 to a manually-entered receiver, and there verified
 * byte-for-byte (the receiver's side of this ladder lives in the Android
 * "Receive" screen; the engine's part is identical through the C ABI).
 *
 * Manual-IP mode (stage 3, pre-discovery): the receiver binds a listener and
 * shows its address + pin; this side connects by hand-entered address. Until
 * engine-level discovery (FRONTIER Phase 2) there is no zero-config path —
 * the copy says so honestly.
 *
 * Descriptor channel (§3.8-safe): the session id, cut number, declared content
 * hash, file name, mime type and evidence class travel in the engine's
 * Hello/Welcome feature negotiation (Vec<String>, relayed verbatim by both
 * FFIs; the engine attaches no semantics). It is DATA ONLY — ids, one digest,
 * one name, one mime — never a path to executable code, a class name, or a
 * shell string.
 *
 * Claim label (§2 law): a run to a loopback address is OBSERVED (LOCAL);
 * a run to a LAN address of a real second device (e.g. the 2026-11-07 Galaxy
 * A04 run) is PHYSICAL cross-device evidence — two machines, two NICs, real
 * TLS. Neither upgrades any platform to "supported" (§30 unrun).
 */

import CryptoKit
import Foundation

// ---- pure validation (unit-tested; mirrors the Android HandoffValidation) ----

/// Configuration validation for the manual-IP handoff. Honest and strict:
/// IPv4 `host:port` only (discovery is FRONTIER Phase 2), 64-hex pins.
public enum HandoffValidation {
    /// The engine admits chunk sizes in [4096, 8 MiB] (§18.6). Clamp the UI knob
    /// into the admitted range instead of failing the run — the stage-1 device
    /// bug (2026-11-07: a 256 B toggle died at OPEN with E_TRANSFER_QUOTA).
    /// NOTE: chunking is DESTINATION-controlled (§18.6) — this side of the
    /// ladder sets it only when driving the destination in-process (tests).
    public static let chunkMinBytes: UInt64 = 4096
    public static let chunkMaxBytes: UInt64 = 8_388_608

    public static func clampChunkSize(_ requested: UInt64) -> UInt64 {
        min(max(requested, chunkMinBytes), chunkMaxBytes)
    }

    /// A receiver address for the manual mode: `IPv4:port`. Loopback is valid
    /// (it is how the LOCAL evidence class is produced and labeled honestly).
    public static func isValidReceiverAddr(_ s: String) -> Bool {
        guard let colon = s.lastIndex(of: ":"), colon != s.startIndex else { return false }
        let host = String(s[s.startIndex..<colon])
        let portText = String(s[s.index(after: colon)...])
        guard let port = UInt32(portText), (1...65535).contains(port), portText.count <= 5,
              !portText.hasPrefix("0") else { return false }
        let parts = host.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return false }
        for p in parts {
            guard let n = UInt32(p), (0...255).contains(n), p.count <= 3,
                  p.allSatisfy({ $0.isASCII && $0.isNumber }) else { return false }
        }
        return true
    }

    /// A pairing pin is the 64-hex SHA-256 of a certificate DER (§18.4) — a
    /// fingerprint, never a secret (EVD-006 excludes only private keys).
    public static func isValidPinHex(_ s: String) -> Bool {
        s.count == 64 && s.allSatisfy { ($0 >= "0" && $0 <= "9") || ($0 >= "a" && $0 <= "f") }
    }

    /// The evidence class of a run, derived from the address this side dials:
    /// loopback → LOCAL (one device), anything else → PHYSICAL (two machines).
    public static func evidenceClass(dialing addr: String) -> String {
        let host = String(addr.prefix(while: { $0 != ":" }))
        return host == "127.0.0.1" || host == "localhost" ? "local" : "physical"
    }

    /// sha256 of a file's bytes, 64 lowercase hex — identical to the engine's
    /// content-addressed digest of `file.document.v1` (adapter-file hex_sha256).
    public static func sha256Hex(of data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    /// MIME type from a file extension — the honest, small, testable map used
    /// for the ACTION_VIEW intent on the receiver (§10.3 PLAT-003). Unknown
    /// extensions stay `application/octet-stream` (the receiver still carries
    /// and verifies the file; only the launch intent depends on the mime).
    public static func mime(forExtension ext: String) -> String {
        switch ext.lowercased() {
        case "png": return "image/png"
        case "jpg", "jpeg": return "image/jpeg"
        case "gif": return "image/gif"
        case "webp": return "image/webp"
        case "heic": return "image/heic"
        case "pdf": return "application/pdf"
        case "txt": return "text/plain"
        case "md": return "text/markdown"
        case "json": return "application/json"
        case "csv": return "text/csv"
        case "mp4": return "video/mp4"
        case "webm": return "video/webm"
        case "mp3": return "audio/mpeg"
        case "html": return "text/html"
        default: return "application/octet-stream"
        }
    }
}

// ---- the handoff descriptor (Hello features; data only, §3.8) ---------------

/// What the source tells the destination over the engine's Hello/Welcome
/// negotiation. Every field is data (ids, one digest, one name, one mime, one
/// evidence class); nothing here names executable code (§3.8).
public struct HandoffDescriptor: Sendable, Equatable {
    public static let marker = "org.carryon.handoff.v1"

    public var session: String    // source session id (import_cut's remote_session)
    public var cut: UInt64        // sealed cut number (import_cut's remote_cut)
    public var contentHash: String // declared sha256 of file.document.v1 (64 hex)
    public var fileName: String   // display name for the reconstructed file
    public var mimeType: String   // ACTION_VIEW intent type (§10.3 PLAT-003)
    public var evidenceClass: String // "local" | "physical" — the source's honest label

    public init(session: String, cut: UInt64, contentHash: String,
                fileName: String, mimeType: String, evidenceClass: String) {
        self.session = session
        self.cut = cut
        self.contentHash = contentHash
        self.fileName = fileName
        self.mimeType = mimeType
        self.evidenceClass = evidenceClass
    }

    /// The Hello features this source sends (key=value strings after a marker).
    public var features: [String] {
        [
            Self.marker,
            "session=\(session)",
            "cut=\(cut)",
            "hash=\(contentHash)",
            "name=\(fileName)",
            "mime=\(mimeType)",
            "evid=\(evidenceClass)",
        ]
    }

    /// Parse the descriptor out of the peer's Hello features. Fail closed on a
    /// missing marker, duplicate keys, or a malformed hash: a destination never
    /// guesses its import parameters (§3.8 — refuse, do not infer).
    public static func parse(features: [String]) -> HandoffDescriptor? {
        guard features.contains(marker) else { return nil }
        var session: String?, cut: UInt64?, hash: String?, name: String?,
            mime: String?, evid: String?
        for f in features {
            guard let eq = f.firstIndex(of: "=") else { continue }
            let key = String(f[f.startIndex..<eq])
            let value = String(f[f.index(after: eq)...])
            switch key {
            case "session": if session == nil { session = value }
            case "cut": if cut == nil { cut = UInt64(value) }
            case "hash": if hash == nil { hash = value }
            case "name": if name == nil { name = value }
            case "mime": if mime == nil { mime = value }
            case "evid": if evid == nil { evid = value }
            default: break // unknown keys are ignored, never interpreted
            }
        }
        guard let s = session, let c = cut, let h = hash, HandoffValidation.isValidPinHex(h),
              let n = name, !n.isEmpty, let m = mime, !m.isEmpty,
              let e = evid, e == "local" || e == "physical" else { return nil }
        return HandoffDescriptor(session: s, cut: c, contentHash: h,
                                fileName: n, mimeType: m, evidenceClass: e)
    }
}

// ---- integrity gate (CORE-004, the app-level half) ---------------------------

/// The destination-side CORE-004 discipline as a pure, testable decision: bytes
/// reconstructed from the store are exposed to the user (or to an ACTION_VIEW
/// intent) ONLY when their measured sha256 equals the declared content hash.
/// The engine guarantees the store itself (stage → verify → atomic publish);
/// this gate guards the app's own reconstruction path and refuses closed.
public enum IntegrityGate {
    public static func verify(bytes: Data, declaredHash: String) -> Bool {
        HandoffValidation.sha256Hex(of: bytes) == declaredHash.lowercased()
    }
}

// ---- the source-side pipeline ------------------------------------------------

/// UI ladder steps of the L0/L1 handoff (spec ladder semantics: L0 = files,
/// L1 = launcher activation — the same order as the onboarding ladder).
public enum HandoffLadderStep: Int, CaseIterable, Sendable, Identifiable {
    case l0Carry = 0
    case l1Open = 1

    public var id: Int { rawValue }
    public var label: String {
        switch self {
        case .l0Carry: return "L0 ─ file carried + verified"
        case .l1Open: return "L1 ─ opened in a real app"
        }
    }
}

public enum HandoffEvent: Sendable {
    case console(ConsoleLine)
    case ladder(HandoffLadderStep, StageOutcome)
    case state(SessionState)
    case counters(sent: UInt64, recv: UInt64)
    /// The full 64-hex source pin, emitted as soon as it exists so the
    /// receiver can pin this Mac (a public fingerprint, EVD-006-clean; the
    /// UI truncates for display, the clipboard/terminal carries it in full).
    case sourcePin(String)
    case finished(FileHandoffResult)
    case failed(String)
}

/// Demo knobs for the file handoff (manual-IP mode, pre-discovery).
public struct FileHandoffConfig: Sendable {
    /// The real file to carry (picked with NSOpenPanel; last dir cached).
    public var fileURL: URL
    /// Receiver address as entered by hand: `IPv4:port` (cached last entry).
    public var receiverAddr: String
    /// The receiver's pairing pin (read off its Receive screen; 64 hex).
    public var receiverPin: String
    /// Pre-generated source identity — the UI arms it BEFORE serving so the
    /// receiver can pin this Mac first (manual mode; the smoke binary makes
    /// its own when nil).
    public var identity: DeviceIdentity?
    /// Integrity negative test: declare the hash of TAMPERED bytes so the
    /// receiver's CORE-004 gate must refuse (fail closed, visible FAIL).
    public var integrityNegative = false
    /// Root for the temp data dir + the evidence file (default: temp dir).
    public var workRoot: String? = nil
    /// Destination chunk size for in-process (loopback/test) runs only;
    /// over the wire the DESTINATION controls chunking (§18.6).
    public var loopbackChunkSize: UInt64 = 0

    public init(fileURL: URL, receiverAddr: String, receiverPin: String,
                identity: DeviceIdentity? = nil, integrityNegative: Bool = false,
                workRoot: String? = nil, loopbackChunkSize: UInt64 = 0) {
        self.fileURL = fileURL
        self.receiverAddr = receiverAddr
        self.receiverPin = receiverPin
        self.identity = identity
        self.integrityNegative = integrityNegative
        self.workRoot = workRoot
        self.loopbackChunkSize = loopbackChunkSize
    }
}

/// Everything one handoff produced.
public struct FileHandoffResult: Sendable {
    public var ok: Bool
    public var workRoot: String
    public var evidencePath: String
    public var bundleJSON: String
    public var verifyOK: Bool
    public var verifyMessage: String
    public var sessionID: String
    public var cutNumber: UInt64
    public var declaredHash: String
    public var tampered: Bool
    public var sourcePin: String
    public var receiverPin: String
    public var bytesSent: UInt64
    public var bytesRecv: UInt64
    public var elapsedMS: Double
    public var evidenceClass: String
    public var failure: String?

    /// The honest footer for a handoff run — distinct from the demo footer
    /// (a handoff can be PHYSICAL; the demo is always LOCAL).
    public static func disclosureLines(class_: String) -> [String] {
        class_ == "physical"
            ? [
                "DISCLOSURE: PHYSICAL cross-device handoff over LAN TLS 1.3 (two machines, two NICs).",
                "Proves the L0/L1 file pipeline across devices; NOT a supported platform (spec §2/§30).",
              ]
            : [
                "DISCLOSURE: LOCAL run on ONE DEVICE via loopback TLS 1.3. Proves the FFI binding and",
                "engine pipeline, NOT physical cross-device transfer or a supported platform (spec §2/§30).",
              ]
    }
}

/// The source side of the L0/L1 file handoff (this Mac serves; the receiver
/// accepts — manual-IP mode). Blocking FFI runs on Task.detached, exactly like
/// DemoPipeline; counters are sampled only between completed wire ops.
public actor FileHandoffPipeline {
    public private(set) var lines: [ConsoleLine] = []

    public init() {}

    @discardableResult
    public func run(_ config: FileHandoffConfig,
                    emit: @escaping @Sendable (HandoffEvent) -> Void = { _ in }) async throws -> FileHandoffResult {
        lines = []
        let t0 = nowMs()

        func log(_ key: String, _ value: String, indent: Int = 1) {
            let line = ConsoleLine(indent: indent, key: key, value: value)
            lines.append(line)
            emit(.console(line))
        }
        func logPlain(_ value: String) {
            let line = ConsoleLine(indent: 0, value: value)
            lines.append(line)
            emit(.console(line))
        }

        // ---- validate the hand-made configuration (fail BEFORE any engine call)
        guard HandoffValidation.isValidReceiverAddr(config.receiverAddr) else {
            throw CarryonError(code: -2, message: "receiver address must be IPv4:port (e.g. 192.168.1.20:48961) — discovery arrives at stage 4")
        }
        guard HandoffValidation.isValidPinHex(config.receiverPin) else {
            throw CarryonError(code: -2, message: "receiver pin must be the 64-hex fingerprint shown on its Receive screen")
        }
        let fileData = try Data(contentsOf: config.fileURL)
        guard !fileData.isEmpty else {
            throw CarryonError(code: -2, message: "refusing to carry an empty file")
        }
        guard fileData.count <= 16 * 1024 * 1024 else {
            // The file adapter declares max_object_bytes = 16 MiB (§26.5.3) —
            // refuse up front instead of failing mid-cut.
            throw CarryonError(code: -2, message: "file exceeds the file adapter's 16 MiB object limit (§26.5.3)")
        }

        let evidenceClass = HandoffValidation.evidenceClass(dialing: config.receiverAddr)
        let fileName = config.fileURL.lastPathComponent
        let mime = HandoffValidation.mime(forExtension: config.fileURL.pathExtension)
        let trueHash = HandoffValidation.sha256Hex(of: fileData)
        // Integrity negative: declare the digest of TAMPERED bytes. The engine
        // still carries and verifies the real bytes against the manifest; the
        // receiver's CORE-004 gate (declared == measured) must REFUSE — the
        // visible-failure counterpart of the demo's §6.8 budget showcase.
        let declaredHash = config.integrityNegative
            ? HandoffValidation.sha256Hex(of: fileData + [0x00])
            : trueHash

        logPlain("== Carry-On macOS file handoff — L0/L1 (manual IP, pre-discovery) ==")
        log("file", "\(config.fileURL.path)  (\(fileData.count) bytes)")
        log("sha256", "\(trueHash)  (declared)")
        if config.integrityNegative {
            log("integrity_test", "NEGATIVE — declared hash deliberately tampered; receiver MUST refuse (CORE-004)")
        }
        log("evidence_class", evidenceClass == "physical"
            ? "PHYSICAL — cross-device LAN TLS 1.3 (two machines, two NICs)"
            : "LOCAL — loopback within one device")

        // ---- engine: register the file adapter, create session + sealed cut
        emit(.state(.preparing))
        let root = config.workRoot
            ?? (NSTemporaryDirectory() as NSString)
                .appendingPathComponent("carryon-filehandoff-\(Int(Date().timeIntervalSince1970))")
        let srcDir = root + "/source"
        try FileManager.default.createDirectory(atPath: srcDir, withIntermediateDirectories: true)
        let engine = try await det("core_open") { try Engine.open(dataDir: srcDir) }
        log("source_dir", srcDir)

        let filePath = config.fileURL.path
        try await det("register_adapter") {
            // JSON-escape the path (data, never behavior — §3.8).
            let escaped = filePath.replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
            _ = try engine.registerAdapter(id: "org.carryon.file",
                                           paramsJSON: "{\"path\":\"\(escaped)\"}")
        }
        log("adapter", "org.carryon.file  (compiled-in, §10.7 L1 content-hash identity)")
        let session = try await det("create_session") {
            try engine.createSession(adapterID: "org.carryon.file",
                                      title: "file handoff: \(fileName)",
                                      privacy: "personal",
                                      authorityMode: "read_only_replica")
        }
        let cut = try await det("create_cut") { try engine.createCut(sessionID: session) }
        emit(.state(.cutProposed))
        emit(.state(.cutSealed))
        log("session", session)
        log("cut", "{session: \(cut.session), number: \(cut.number)}  (two-phase publish, CORE-004)")

        // ---- identity + trust: pair the hand-entered receiver pin
        let identity: DeviceIdentity
        if let pre = config.identity {
            identity = pre
        } else {
            identity = try await det("identity_generate") {
                try DeviceIdentity.generate(name: "carryon-mac-source")
            }
        }
        let sourcePin = try identity.pinHex()
        emit(.sourcePin(sourcePin))
        log("source_pin", "\(sourcePin.prefix(12))…  (truncated; full pin copied/emitted for the receiver)")
        let trust = try TrustStore.fromJSON("{\"peers\":{}}")
        try trust.pair(name: "receiver", pinHex: config.receiverPin, utc: nowUTC())
        log("trust", "receiver pin \(config.receiverPin.prefix(12))… paired  (mutual pin, §18.4)")

        // ---- connect to the manually-entered receiver (client side)
        log("connect", "\(config.receiverAddr)  (TLS 1.3, mutual certificate pinning §18.5)")
        let wire = try await det("session_connect") {
            try NetSession.connect(addr: config.receiverAddr, identity: identity, trust: trust)
        }
        // The descriptor travels in the engine's Hello/Welcome negotiation —
        // data only (§3.8): session, cut, declared hash, name, mime, class.
        let descriptor = HandoffDescriptor(
            session: session, cut: cut.number, contentHash: declaredHash,
            fileName: fileName, mimeType: mime, evidenceClass: evidenceClass)
        let theirs = try await det("client_negotiate") {
            try wire.clientNegotiate(features: descriptor.features)
        }
        log("negotiate", "descriptor sent (\(descriptor.features.count) features, data only §3.8); theirs: \(theirs.joined(separator: ","))")

        // ---- serve the sealed cut (BLOCKING until ImportComplete)
        emit(.state(.importing))
        try await det("serve_cut") { try engine.serveCut(session: wire) }
        let (sent, recv) = try wire.byteCounters() // between completed ops only
        emit(.counters(sent: sent, recv: recv))
        log("serve", "cut served — destination reported ImportComplete")
        log("bytes", "sent=\(sent) recv=\(recv)")
        emit(.ladder(.l0Carry, .ok))
        emit(.state(.actionReady))

        // ---- L1 happens on the RECEIVER (its engine action + ACTION_VIEW);
        // this side shows the honest half-step, never a claim it cannot see.
        let note = "launcher activation runs on the receiver (engine document.open + OS ACTION_VIEW) — see its Receive screen"
        log("L1", note)
        emit(.ladder(.l1Open, .skipped("on receiver")))

        // ---- evidence for the source session
        let extra: AnyJSON = .object([
            "level": .string("L0-file-carry"),
            "l1_note": .string("launcher activation on receiver"),
            "source_session_id": .string(session),
            "source_cut": .number(Double(cut.number)),
            "file_name": .string(fileName),
            "file_bytes": .number(Double(fileData.count)),
            "content_sha256_declared": .string(declaredHash),
            "content_sha256_true": .string(trueHash),
            "integrity_negative_test": .bool(config.integrityNegative),
            "evidence_class": .string(evidenceClass),
            "bytes_transferred_sent": .number(Double(sent)),
            "bytes_transferred_recv": .number(Double(recv)),
            "adapter": .string("org.carryon.file"),
            "authority_transfer": .string("none — L0/L1 is activation-only (honest: no unsaved app state claimed, §10.7)"),
        ])
        let bundleJSON = try await det("export_evidence_with") {
            try engine.exportEvidence(sessionID: session, extraMetricsJSON: extra.jsonEncoded)
        }
        let evidencePath = srcDir + "/handoff-evidence.json"
        try bundleJSON.write(toFile: evidencePath, atomically: true, encoding: .utf8)
        let verify = try await det("verify_evidence") {
            try engine.verifyEvidence(atPath: evidencePath)
        }
        log("evidence", evidencePath)
        log("verify", "{\"ok\":\(verify.ok),\"message\":\"\(verify.message)\"}")

        let elapsed = nowMs() - t0
        logPlain("")
        logPlain(String(format: "done:  file handoff served in %.1f ms", elapsed))
        for l in FileHandoffResult.disclosureLines(class_: evidenceClass) {
            logPlain(l)
        }

        try await det("session_teardown") { wire.teardown() }
        try await det("core_free") { engine.close() }

        let result = FileHandoffResult(
            ok: verify.ok, workRoot: root, evidencePath: evidencePath,
            bundleJSON: bundleJSON, verifyOK: verify.ok, verifyMessage: verify.message,
            sessionID: session, cutNumber: cut.number, declaredHash: declaredHash,
            tampered: config.integrityNegative, sourcePin: sourcePin,
            receiverPin: config.receiverPin, bytesSent: sent, bytesRecv: recv,
            elapsedMS: elapsed, evidenceClass: evidenceClass, failure: nil)
        emit(.finished(result))
        return result
    }
}
