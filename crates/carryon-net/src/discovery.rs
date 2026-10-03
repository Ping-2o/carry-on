//! Service discovery contract (spec §18.3, DNS-SD `_carryon._tcp`).
//!
//! # Scope honesty
//!
//! Live mDNS/DNS-SD browsing requires a LAN and a platform responder; it is a
//! **TARGET** for the platform shells (§9) and is not exercised by the headless
//! loopback tests. What this module provides now is (a) the exact minimal TXT
//! record contract the responder MUST follow (§18.3 — no user/session/app names,
//! no hashes, no public keys), enforced and unit-tested, and (b) a `Discovery`
//! trait with a static in-memory implementation so the import/export flow can be
//! driven without a live network. A real mDNS backend implements the same trait.

use serde::{Deserialize, Serialize};

/// The production service type (§18.3).
pub const SERVICE_TYPE: &str = "_carryon._tcp";

/// Coarse device class exposed in discovery (§18.3 "coarse device class"). Never a
/// model string or anything user-identifying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceClass {
    Desktop,
    Mobile,
    Unknown,
}

impl DeviceClass {
    fn as_str(self) -> &'static str {
        match self {
            DeviceClass::Desktop => "desktop",
            DeviceClass::Mobile => "mobile",
            DeviceClass::Unknown => "unknown",
        }
    }
}

/// The minimal TXT record a responder advertises (§18.3). It MUST reveal only
/// these fields — enforced by construction here. In particular it carries no
/// user name, session/application names, object hashes, or public keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxtRecord {
    /// Protocol major version only (not the minor, not a feature set).
    pub protocol_major: u16,
    /// Short opaque instance identifier (not a device name, not a user).
    pub instance_id: String,
    /// Whether pairing is required before connection.
    pub pairing_required: bool,
    /// Coarse device class.
    pub device_class: DeviceClass,
    /// Ephemeral connection hint (e.g. a port), safe to expose.
    pub connection_hint: String,
}

impl TxtRecord {
    /// Render as `key=value` TXT pairs (the wire form a responder publishes).
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        vec![
            ("pm".into(), self.protocol_major.to_string()),
            ("id".into(), self.instance_id.clone()),
            ("pr".into(), (self.pairing_required as u8).to_string()),
            ("dc".into(), self.device_class.as_str().into()),
            ("ch".into(), self.connection_hint.clone()),
        ]
    }
}

/// A discovered peer instance (what browsing returns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPeer {
    pub instance_id: String,
    /// Socket address to dial (`host:port`).
    pub address: String,
    pub txt: TxtRecord,
}

/// Discovery backend (§18.3). A real mDNS responder and a static test source both
/// implement this; the import/export orchestration depends only on the trait.
pub trait Discovery {
    /// Browse for `_carryon._tcp` peers currently advertised.
    fn browse(&self) -> Vec<DiscoveredPeer>;
}

/// A static, in-memory discovery source for headless flows and tests. **Not** a
/// live network responder; it stands in for one behind the `Discovery` trait.
#[derive(Debug, Default, Clone)]
pub struct StaticDiscovery {
    peers: Vec<DiscoveredPeer>,
}

impl StaticDiscovery {
    pub fn new() -> Self {
        StaticDiscovery::default()
    }

    pub fn with_peer(mut self, peer: DiscoveredPeer) -> Self {
        self.peers.push(peer);
        self
    }
}

impl Discovery for StaticDiscovery {
    fn browse(&self) -> Vec<DiscoveredPeer> {
        self.peers.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_exposes_only_minimal_fields() {
        let txt = TxtRecord {
            protocol_major: 1,
            instance_id: "abc123".into(),
            pairing_required: true,
            device_class: DeviceClass::Desktop,
            connection_hint: "49152".into(),
        };
        let keys: Vec<_> = txt.to_pairs().into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["pm", "id", "pr", "dc", "ch"]);
        // §18.3: no field may carry a user/session/app name, hash, or key.
        let blob = format!("{:?}", txt.to_pairs());
        for forbidden in ["user", "session", "sha256", "pubkey", "BEGIN"] {
            assert!(!blob.contains(forbidden), "TXT leaked {forbidden}");
        }
    }

    #[test]
    fn static_discovery_returns_seeded_peers() {
        let peer = DiscoveredPeer {
            instance_id: "abc".into(),
            address: "127.0.0.1:49152".into(),
            txt: TxtRecord {
                protocol_major: 1,
                instance_id: "abc".into(),
                pairing_required: true,
                device_class: DeviceClass::Desktop,
                connection_hint: "49152".into(),
            },
        };
        let d = StaticDiscovery::new().with_peer(peer.clone());
        assert_eq!(d.browse(), vec![peer]);
    }
}
