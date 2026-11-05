/*
 * CarryonCore.swift — Swift binding for the engine lifecycle entries
 * (carryon_core_* / adapters / sessions / cuts / actions / evidence).
 *
 * Ownership discipline (carryon.h): out-strings are Rust-allocated and freed
 * with carryon_string_free; handles are freed by their matching *_free; every
 * fallible call returns int32_t (0 = OK, negative = FFI-local, positive =
 * core error family). Blocking entries (serve/import/authority) are invoked by
 * DemoPipeline on Task.detached, never on the main actor.
 */

import CCarryon
import Foundation

/// ABI version tuple from `carryon_abi_version`.
public struct AbiVersion: Sendable, Equatable, CustomStringConvertible {
    public let major: UInt32
    public let minor: UInt32
    public static func current() -> AbiVersion {
        var maj: UInt32 = 0, min: UInt32 = 0
        carryon_abi_version(&maj, &min)
        return AbiVersion(major: maj, minor: min)
    }
    public var description: String { "\(major).\(minor)" }
}

/// `{session, number}` cut reference (the C ABI's cut_json shape).
public struct CutRef: Codable, Sendable, Equatable {
    public var session: String
    public var number: UInt64
    public init(session: String, number: UInt64) {
        self.session = session
        self.number = number
    }
    public var jsonString: String {
        (try? JSONEncoder().encode(self)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
    }
}

/// Action availability row from `carryon_list_available_actions`.
public struct ActionAvailability: Codable, Sendable {
    public var `class`: String
    public var ready: Bool
}

/// Oracle outcome + output of an executed action (ABI `action_result_json`).
public struct ActionResult: Codable, Sendable {
    public struct Oracle: Codable, Sendable {
        public var checked: Bool
        public var agreed: Bool
        public var detail: String
    }
    public var output: AnyJSON
    public var output_hash: String
    public var oracle: Oracle

    public var oracleAgreed: Bool { oracle.checked && oracle.agreed }
}

/// Recovery report fields (§19.3), as emitted by `carryon_core_recovery_report_json`.
/// The lists carry the offending ids (empty = none); counts are their sizes.
public struct RecoveryReport: Codable, Sendable, Equatable {
    public var interrupted_transfers: [String]
    public var hidden_incomplete_objects: [String]
    public var ambiguous_sessions: [String]
    public var journal_truncated: Bool
    public var clean: Bool
}

/// The engine core handle (`CarryonCore` in C; named Engine here because the
/// Swift type would otherwise collide with the imported C struct name).
///
/// @unchecked Sendable: wraps a raw C handle. Cross-thread use follows the C
/// ABI contract — one flow touches a given core at a time, except
/// `carryon_core_request_suspend`, which the header documents as the one entry
/// safe to call while an import runs on another thread.
public final class Engine: @unchecked Sendable {
    public let handle: OpaquePointer
    public let dataDir: String

    /// Open (or create) an engine rooted at `dataDir` (§8 lifecycle start).
    public static func open(dataDir: String) throws -> Engine {
        let rc = dataDir.withCString { carryon_core_open($0) }
        guard let handle = rc else { throw CarryonError.nullReturn("carryon_core_open") }
        return Engine(handle: handle, dataDir: dataDir)
    }

    init(handle: OpaquePointer, dataDir: String) {
        self.handle = handle
        self.dataDir = dataDir
    }

    deinit { carryon_core_free(handle) }

    /// Recovery report JSON (§19.3): interrupted transfers, hidden staged
    /// objects, ambiguous sessions, journal truncation, overall cleanliness.
    public func recoveryReport() throws -> RecoveryReport {
        let json = try outString("carryon_core_recovery_report_json") { out in
            carryon_core_recovery_report_json(handle, out)
        }
        guard let report = try? JSONDecoder().decode(RecoveryReport.self, from: Data(json.utf8)) else {
            throw CarryonError(code: -4, message: "unparseable recovery report: \(json)")
        }
        return report
    }

    /// Set the transfer chunk size in bytes (§18.6); 0 resets to the default.
    /// The DESTINATION controls chunking, so set this on the importing core.
    public func setChunkSize(_ bytes: UInt64) throws {
        try CarryonError.check(carryon_core_set_chunk_size(handle, bytes), "carryon_core_set_chunk_size")
    }

    /// Set the active budget from JSON (§6.8). Budget rejection SHALL be visible —
    /// a tiny `total_net_bytes` makes the import return a partial admission.
    public func setBudget(json: String) throws {
        try CarryonError.check(json.withCString { carryon_core_set_budget(handle, $0) },
                               "carryon_core_set_budget")
    }

    /// Foreground (true) / background (false); background shrinks the budget (§20.2).
    public func setForeground(_ fg: Bool) throws {
        try CarryonError.check(carryon_core_set_foreground(handle, fg), "carryon_core_set_foreground")
    }

    /// Request cooperative suspension of an in-flight transfer (§11.2).
    /// Thread-safe — the only entry safe to call while an import runs on another
    /// thread; the flag may also be armed BEFORE the import begins.
    public func requestSuspend() throws {
        try CarryonError.check(carryon_core_request_suspend(handle), "carryon_core_request_suspend")
    }

    /// Register one of the compiled-in adapters by id + JSON params (§3.8: data,
    /// never behavior). Returns the AdapterInfo JSON.
    @discardableResult
    public func registerAdapter(id: String, paramsJSON: String) throws -> String {
        try id.withCString { idc in
            try paramsJSON.withCString { pc in
                try outString("carryon_register_adapter") { out in
                    carryon_register_adapter(handle, idc, pc, out)
                }
            }
        }
    }

    /// Create a session. Returns the session id (UUID string).
    /// `authorityMode`: "single_writer" | "read_only_replica".
    public func createSession(adapterID: String, title: String,
                              privacy: String = "public",
                              authorityMode: String = "read_only_replica") throws -> String {
        let req: AnyJSON = .object([
            "adapter_id": .string(adapterID),
            "title": .string(title),
            "privacy": .string(privacy),
            "authority_mode": .string(authorityMode),
        ])
        return try outString("carryon_create_session") { out in
            req.jsonEncoded.withCString { carryon_create_session(handle, $0, out) }
        }
    }

    /// Seal a cut for `sessionID`. Returns `{session, number}`.
    public func createCut(sessionID: String) throws -> CutRef {
        let json = try outString("carryon_create_cut") { out in
            sessionID.withCString { carryon_create_cut(handle, $0, out) }
        }
        guard let cut = try? JSONDecoder().decode(CutRef.self, from: Data(json.utf8)) else {
            throw CarryonError(code: -4, message: "unparseable cut json: \(json)")
        }
        return cut
    }

    /// Which of `classes` are runnable at `cut`.
    public func listAvailableActions(cut: CutRef, classes: [String]) throws -> [ActionAvailability] {
        let classesJSON = (try? JSONEncoder().encode(classes)).flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
        let json = try cut.jsonString.withCString { cj in
            try classesJSON.withCString { cl in
                try outString("carryon_list_available_actions") { out in
                    carryon_list_available_actions(handle, cj, cl, out)
                }
            }
        }
        guard let rows = try? JSONDecoder().decode([ActionAvailability].self, from: Data(json.utf8)) else {
            throw CarryonError(code: -4, message: "unparseable actions json: \(json)")
        }
        return rows
    }

    /// Execute an action at `cut` with the session's recorded adapter.
    public func executeAction(cut: CutRef, class: String, params: AnyJSON = .null) throws -> ActionResult {
        try action(cut: cut, class: `class`, params: params, adapterID: nil)
    }

    /// Execute an action at `cut` driven by an explicitly named adapter — needed
    /// for mirror sessions (recorded adapter id `imported`).
    public func executeActionAs(cut: CutRef, class: String, params: AnyJSON = .null,
                                adapterID: String) throws -> ActionResult {
        try action(cut: cut, class: `class`, params: params, adapterID: adapterID)
    }

    private func action(cut: CutRef, class: String, params: AnyJSON,
                        adapterID: String?) throws -> ActionResult {
        let req: AnyJSON = .object(["class": .string(`class`), "params": params])
        let resultJSON: String = try req.jsonEncoded.withCString { rc in
            try cut.jsonString.withCString { cj in
                if let adapterID {
                    return try adapterID.withCString { ai in
                        try outString("carryon_execute_action_as") { out in
                            carryon_execute_action_as(handle, cj, rc, ai, out)
                        }
                    }
                }
                return try outString("carryon_execute_action") { out in
                    carryon_execute_action(handle, cj, rc, out)
                }
            }
        }
        guard let result = try? JSONDecoder().decode(ActionResult.self, from: Data(resultJSON.utf8)) else {
            throw CarryonError(code: -4, message: "unparseable action result: \(resultJSON)")
        }
        return result
    }

    /// Whether an object with `contentHash` (64 hex) is present + verified locally.
    public func hasObject(hex contentHash: String) throws -> Bool {
        var present = false
        try CarryonError.check(
            contentHash.withCString { carryon_has_object_hex(handle, $0, &present) },
            "carryon_has_object_hex")
        return present
    }

    /// Export an evidence bundle for `sessionID` (§23.3) as JSON text.
    public func exportEvidence(sessionID: String) throws -> String {
        try exportEvidence(sessionID: sessionID, extraMetricsJSON: nil)
    }

    /// Export an evidence bundle, merging caller-measured continuation metrics
    /// into `metrics` (they become part of the bundle's section hashes).
    public func exportEvidence(sessionID: String, extraMetricsJSON: String?) throws -> String {
        try sessionID.withCString { sid in
            try (extraMetricsJSON ?? "null").withCString { extra in
                try outString("carryon_export_evidence_with") { out in
                    carryon_export_evidence_with(handle, sid, extra, out)
                }
            }
        }
    }

    /// Verify a bundle on disk — parse + hash-check only; NEVER executes bundle
    /// content (EVD-005). Returns `{ok, message}`.
    public func verifyEvidence(atPath path: String) throws -> (ok: Bool, message: String) {
        let json = try path.withCString { pc in
            try outString("carryon_verify_evidence") { out in
                carryon_verify_evidence(handle, pc, out)
            }
        }
        guard let v = parseAnyJSON(json), case .object(let o) = v,
              let ok = o["ok"]?.boolValue, let msg = o["message"]?.stringValue else {
            throw CarryonError(code: -4, message: "unparseable verify report: \(json)")
        }
        return (ok, msg)
    }

    /// Read a content-addressed object's bytes by 64-hex digest (the `*len` query
    /// protocol: the null-buffer query pass returns CARRYON_ERR_BUFFER_TOO_SMALL
    /// and sets *len to the required size).
    public func readObject(hex contentHash: String) throws -> Data {
        try contentHash.withCString { hc in
            var len: Int = 0
            let q = carryon_read_object(handle, hc, nil, &len)
            guard q == CARRYON_ERR_BUFFER_TOO_SMALL, len > 0 else {
                try CarryonError.check(q, "carryon_read_object(query)")
                return Data()
            }
            var buf = [UInt8](repeating: 0, count: len)
            try CarryonError.check(carryon_read_object(handle, hc, &buf, &len),
                                   "carryon_read_object(copy)")
            buf.removeSubrange(min(len, buf.count)..<buf.count)
            return Data(buf)
        }
    }

    // ---- Transfer (handoff, §18.6) ------------------------------------------

    /// Serve a sealed cut's bytes until the destination reports ImportComplete
    /// (source side; BLOCKING — run on Task.detached).
    public func serveCut(session: NetSession) throws {
        try CarryonError.check(carryon_serve_cut(handle, session.handle), "carryon_serve_cut")
    }

    /// Import a remote cut (destination side; BLOCKING — run on Task.detached).
    /// Returns the raw outcome JSON: `{"completed":true,"mirror_cut":N}` or
    /// `{"suspended":true,"resume_token":{…}}` or `{"partial_budget":true,…}`.
    public func importCut(session: NetSession, remoteSession: String, remoteCut: UInt64) throws -> String {
        try remoteSession.withCString { rs in
            try outString("carryon_import_cut") { out in
                carryon_import_cut(handle, session.handle, rs, remoteCut, out)
            }
        }
    }

    /// Resume a suspended import from a token JSON (as emitted by import_cut).
    public func resumeImport(session: NetSession, resumeTokenJSON: String) throws -> String {
        try resumeTokenJSON.withCString { tk in
            try outString("carryon_resume_import") { out in
                carryon_resume_import(handle, session.handle, tk, out)
            }
        }
    }

    // ---- Authority transfer (L4, §21.2/§21.3) --------------------------------

    /// Whether this core may authoritatively mutate `sessionID` right now
    /// (AUTH-004/005).
    public func mayMutate(sessionID: String) throws -> Bool {
        var out = false
        let rc = sessionID.withCString { carryon_may_mutate(handle, $0, &out) }
        try CarryonError.check(rc, "carryon_may_mutate")
        return out
    }

    /// Source side: offer + relinquish single-writer authority (BLOCKING — the
    /// destination's acceptance is awaited on the wire). Drops this device to a
    /// read-only replica. Returns the receipt set JSON (`new_epoch`, receipts).
    public func serveAuthorityTransfer(session: NetSession, sessionID: String,
                                       cutNumber: UInt64) throws -> String {
        try sessionID.withCString { sid in
            try outString("carryon_serve_authority_transfer") { out in
                carryon_serve_authority_transfer(handle, session.handle, sid, cutNumber, out)
            }
        }
    }

    /// Destination side: accept an offered authority transfer onto the local
    /// mirror `mirrorSessionID`, continued by the registered `adapterID`.
    /// Becomes the owner. Returns the receipt set JSON.
    public func requestAuthorityTransfer(session: NetSession, mirrorSessionID: String,
                                         adapterID: String) throws -> String {
        try mirrorSessionID.withCString { mid in
            try adapterID.withCString { aid in
                try outString("carryon_request_authority_transfer") { out in
                    carryon_request_authority_transfer(handle, session.handle, mid, aid, out)
                }
            }
        }
    }
}

/// Helper: call `body` with a writable out-pointer, take the returned
/// Rust-allocated string, and free it with `carryon_string_free`.
func outString(_ where_: String, _ body: (UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> Int32) throws -> String {
    var out: UnsafeMutablePointer<CChar>? = nil
    try CarryonError.check(body(&out), where_)
    guard let ptr = out else { throw CarryonError.nullReturn(where_) }
    defer { carryon_string_free(ptr) }
    return String(cString: ptr)
}

/// Helper: take an out-string that is returned on SUCCESS only (pointer-returning
/// entries use null + last_error instead; this is for `char**` after rc == 0).
func takeAndFree(_ ptr: UnsafeMutablePointer<CChar>?) -> String {
    guard let ptr else { return "" }
    defer { carryon_string_free(ptr) }
    return String(cString: ptr)
}
