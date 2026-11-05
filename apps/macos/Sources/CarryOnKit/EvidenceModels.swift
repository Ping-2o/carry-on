/*
 * EvidenceModels.swift — Codable mirror of the EvidenceBundle JSON emitted by
 * crates/carryon-core/src/evidence.rs (§23.3). Field names follow the Rust
 * serde names exactly (snake_case on the wire). A bundle is parsed and
 * DISPLAYED only — no bundle content is ever executed (EVD-005).
 */

import Foundation

public struct EvidenceBundle: Codable, Sendable, Equatable {
    public var bundle_version: Int
    public var session_id: String
    public var created_utc: String
    public var verifier_version: String
    public var build: BuildInfo
    /// Session + cut + object metadata (no payloads).
    public var session_json: AnyJSON
    /// Journal events for the session (§23.1).
    public var journal: [AnyJSON]
    /// Action results + oracle outcomes.
    public var actions: [AnyJSON]
    /// Aggregate metrics (§23.2) + injected continuation metrics.
    public var metrics: AnyJSON
    /// Per-section integrity hashes as [name, sha256] pairs.
    public var section_hashes: [SectionHash]
    /// The explicit what-it-proves / what-it-does-not-prove statement (§23.3).
    public var disclosure: String

    public struct BuildInfo: Codable, Sendable, Equatable {
        public var crate_version: String
        public var schema_version: Int
        public var phase: String
    }

    /// `section_hashes` is `Vec<(String, String)>` — JSON arrays of two strings.
    public struct SectionHash: Codable, Sendable, Equatable, Identifiable {
        public var name: String
        public var sha256: String
        public var id: String { name }

        public init(name: String, sha256: String) {
            self.name = name
            self.sha256 = sha256
        }

        public init(from decoder: Decoder) throws {
            var arr = try decoder.unkeyedContainer()
            name = try arr.decode(String.self)
            sha256 = try arr.decode(String.self)
        }

        public func encode(to encoder: Encoder) throws {
            var arr = encoder.unkeyedContainer()
            try arr.encode(name)
            try arr.encode(sha256)
        }
    }

    /// Parse a bundle JSON string (nil if malformed — never execute it).
    public static func parse(_ json: String) -> EvidenceBundle? {
        guard let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(EvidenceBundle.self, from: data)
    }

    /// Metric lookup for the viewer (`metrics` is a free-form object).
    public func metric(_ key: String) -> AnyJSON? {
        guard case .object(let o) = metrics else { return nil }
        return o[key]
    }
}

/// `{ok, message}` report from `carryon_verify_evidence`.
public struct VerifyReport: Codable, Sendable, Equatable {
    public var ok: Bool
    public var message: String
}
