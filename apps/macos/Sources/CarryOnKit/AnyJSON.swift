/*
 * AnyJSON.swift — minimal Codable JSON value for the engine's free-form sections
 * (session_json, journal[], actions[], metrics{} in an EvidenceBundle, and action
 * outputs). Bundles are parsed and DISPLAYED only; no bundle content is ever
 * executed (EVD-005).
 */

import Foundation

public enum AnyJSON: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([AnyJSON])
    case object([String: AnyJSON])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let b = try? container.decode(Bool.self) {
            self = .bool(b)
        } else if let n = try? container.decode(Double.self) {
            self = .number(n)
        } else if let s = try? container.decode(String.self) {
            self = .string(s)
        } else if let a = try? container.decode([AnyJSON].self) {
            self = .array(a)
        } else if let o = try? container.decode([String: AnyJSON].self) {
            self = .object(o)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container, debugDescription: "unsupported JSON value")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let b): try container.encode(b)
        case .number(let n): try container.encode(n)
        case .string(let s): try container.encode(s)
        case .array(let a): try container.encode(a)
        case .object(let o): try container.encode(o)
        }
    }

    public var stringValue: String? {
        if case .string(let s) = self { return s }
        return nil
    }

    public var numberValue: Double? {
        if case .number(let n) = self { return n }
        return nil
    }

    public var boolValue: Bool? {
        if case .bool(let b) = self { return b }
        return nil
    }

    /// Compact JSON rendering for the console (repo style: strings unquoted for
    /// readability). NOT machine-readable JSON — use `jsonEncoded` for that.
    public var rendered: String {
        switch self {
        case .null: return "null"
        case .bool(let b): return b ? "true" : "false"
        case .number(let n):
            if n.rounded() == n, abs(n) < 1e15 {
                return String(Int64(n))
            }
            return String(format: "%.3f", n)
        case .string(let s): return s
        case .array(let a): return "[" + a.map(\.rendered).joined(separator: ",") + "]"
        case .object(let o):
            let sorted = o.sorted { $0.key < $1.key }
            return "{" + sorted.map { "\($0.key)=\($0.value.rendered)" }.joined(separator: ",") + "}"
        }
    }

    /// Valid JSON text (strings quoted/escaped, keys sorted) — the form fed back
    /// to the engine's JSON-parsing entry points.
    public var jsonEncoded: String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let data = try? encoder.encode(self),
              let text = String(data: data, encoding: .utf8) else { return "null" }
        return text
    }
}

/// Parse a JSON string, returning nil on malformed input.
public func parseAnyJSON(_ text: String) -> AnyJSON? {
    guard let data = text.data(using: .utf8) else { return nil }
    return try? JSONDecoder().decode(AnyJSON.self, from: data)
}
