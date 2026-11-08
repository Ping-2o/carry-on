/*
 * CarryonError.swift — Swift mirror of the stable i32 error-code table from
 * crates/carryon-ffi/src/errors.rs (spec §24 requires machine-readable codes at
 * the boundary). Scheme: 0 = OK; negative = FFI-local failures; positive =
 * family_base + ordinal. `carryon_last_error` supplies the message.
 */

import CCarryon
import Foundation

public struct CarryonError: Error, CustomStringConvertible, Sendable {
    public let code: Int32
    public let message: String

    public var description: String {
        "carryon error \(code) [\(CarryonError.name(of: code))]: \(message)"
    }

    /// Capture the thread-local last error from the engine (the `*len` query
    /// protocol: pass a null buffer to learn the required size first).
    init(code: Int32, message fallback: String) {
        self.code = code
        self.message = Self.lastErrorMessage() ?? fallback
    }

    /// Throw unless `rc == CARRYON_OK` (0). `where_` names the FFI entry for context.
    @discardableResult
    public static func check(_ rc: Int32, _ where_: String) throws -> Int32 {
        guard rc != CARRYON_OK else { return rc }
        throw CarryonError(code: rc, message: "at \(where_)")
    }

    /// Throw a descriptive error for a null-handle / null-return outcome
    /// (functions returning a pointer return NULL on error; check last_error).
    public static func nullReturn(_ where_: String) -> CarryonError {
        CarryonError(code: -2, message: "null return at \(where_)")
    }

    static func lastErrorMessage() -> String? {
        var len: Int = 0
        // Query pass: null buffer returns CARRYON_ERR_BUFFER_TOO_SMALL and sets
        // *len to the required size (incl. NUL) — the header's *len protocol.
        let q = carryon_last_error(nil, &len)
        guard q == CARRYON_ERR_BUFFER_TOO_SMALL, len > 1 else { return nil }
        var buf = [UInt8](repeating: 0, count: len)
        let rc = carryon_last_error(&buf, &len)
        guard rc == CARRYON_OK else { return nil }
        buf.removeLast() // drop the trailing NUL
        return String(decoding: buf, as: UTF8.self)
    }

    /// The full code-name table, mirrored exhaustively from errors.rs
    /// (`code_of` there is exhaustive by design; adding a core variant forces a
    /// compile error in Rust, and this mirror documents the stable numbers).
    public static func name(of code: Int32) -> String {
        func family(_ base: Int32, _ names: [String]) -> String? {
            let idx = code - base
            guard idx >= 0, Int(idx) < names.count else { return nil }
            return names[Int(idx)]
        }
        switch code {
        case 0: return "OK"
        case -1: return "ERR_PANIC"
        case -2: return "ERR_NULL_ARG"
        case -3: return "ERR_BAD_UTF8"
        case -4: return "ERR_BAD_JSON"
        case -5: return "ERR_BAD_HANDLE"
        case -6: return "ERR_BUFFER_TOO_SMALL"
        default: break
        }
        if let n = family(100, ["ADAPTER_MISSING", "ADAPTER_INCOMPATIBLE", "ADAPTER_CRASHED",
                                "ADAPTER_TIMEOUT", "ADAPTER_MALFORMED", "ADAPTER_CONSENT_REQUIRED",
                                "ADAPTER_STALE_GENERATION"]) { return n }
        if let n = family(200, ["SCHEMA_UNSUPPORTED", "SCHEMA_INVALID"]) { return n }
        if let n = family(300, ["OBJECT_MISSING", "OBJECT_CORRUPT", "OBJECT_STALE",
                                "OBJECT_OVERSIZED", "OBJECT_INVALID", "OBJECT_DIGEST_MISMATCH",
                                "OBJECT_INCOMPLETE_STAGED", "OBJECT_SECRET_EXCLUDED"]) { return n }
        if let n = family(400, ["TRANSFER_TIMEOUT", "TRANSFER_CANCELLED", "TRANSFER_DIGEST_MISMATCH",
                                "TRANSFER_CONFLICTING_CHUNK", "TRANSFER_QUOTA",
                                "TRANSFER_RESUME_FAILURE"]) { return n }
        if let n = family(500, ["BUDGET_NETWORK", "BUDGET_CPU", "BUDGET_MEMORY",
                                "BUDGET_STORAGE", "BUDGET_TIME"]) { return n }
        if let n = family(600, ["ACTION_UNSUPPORTED", "ACTION_DEPENDENCY_FAILURE",
                                "ACTION_EXECUTION_FAILURE", "ACTION_ORACLE_FAILURE"]) { return n }
        if let n = family(700, ["AUTH_PERMISSION", "AUTH_WRONG_EPOCH", "AUTH_AMBIGUOUS",
                                "AUTH_READ_ONLY"]) { return n }
        if code == 800 { return "MATH" }
        if let n = family(900, ["INTERNAL_INVARIANT", "INTERNAL_DB_CORRUPT", "INTERNAL_IO",
                                "INTERNAL_SERIALIZATION", "INTERNAL_IDEMPOTENCY"]) { return n }
        if let n = family(1000, ["PROTO_VERSION", "PROTO_FRAMING", "PROTO_SEQUENCE",
                                 "PROTO_REPLAY", "PROTO_STATE"]) { return n }
        if let n = family(1100, ["PLATFORM_PERMISSION", "PLATFORM_ACTIVATION",
                                 "PLATFORM_BACKGROUND", "PLATFORM_SECURE_STORAGE",
                                 "PLATFORM_PACKAGE"]) { return n }
        return "CODE_\(code)"
    }
}
