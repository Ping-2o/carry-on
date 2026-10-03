//! Trust store (spec §18.4/§18.5/§22.3). Records the pinned identity of each
//! paired peer. Revocation increments a local trust generation and blocks
//! reconnect (§18.4.9); a connection whose peer pin is unknown or revoked fails
//! closed (NET-001).
//!
//! The store is plain serde data so a platform shell can persist it in secure
//! storage. It holds only **public** pins and device names — never private keys
//! (EVD-006).

use crate::identity::Pin;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One paired peer's trust record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustRecord {
    pub device_name: String,
    /// The pinned SHA-256 of the peer's certificate DER, hex.
    pub pin_hex: String,
    /// Local trust generation; revocation bumps it and blocks the old pin.
    pub trust_generation: u64,
    pub revoked: bool,
    pub paired_utc: String,
}

/// A set of trusted peers keyed by pin hex.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    peers: HashMap<String, TrustRecord>,
}

impl TrustStore {
    pub fn new() -> Self {
        TrustStore::default()
    }

    /// Record a newly paired peer, pinning its certificate (§18.4.6-7).
    pub fn pair(
        &mut self,
        device_name: impl Into<String>,
        pin: Pin,
        paired_utc: impl Into<String>,
    ) {
        let pin_hex = pin.to_hex();
        self.peers.insert(
            pin_hex.clone(),
            TrustRecord {
                device_name: device_name.into(),
                pin_hex,
                trust_generation: 0,
                revoked: false,
                paired_utc: paired_utc.into(),
            },
        );
    }

    /// Whether a peer presenting `pin` is currently trusted (paired, not revoked).
    pub fn is_trusted(&self, pin: &Pin) -> bool {
        self.peers
            .get(&pin.to_hex())
            .map(|r| !r.revoked)
            .unwrap_or(false)
    }

    /// Revoke a peer (§18.4.9): mark revoked and bump trust generation. A later
    /// reconnect with the same pin fails closed.
    pub fn revoke(&mut self, pin: &Pin) -> bool {
        if let Some(r) = self.peers.get_mut(&pin.to_hex()) {
            r.revoked = true;
            r.trust_generation += 1;
            true
        } else {
            false
        }
    }

    pub fn get(&self, pin: &Pin) -> Option<&TrustRecord> {
        self.peers.get(&pin.to_hex())
    }

    /// All records (for a device-list UI).
    pub fn records(&self) -> impl Iterator<Item = &TrustRecord> {
        self.peers.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;

    #[test]
    fn pair_then_trusted() {
        let peer = DeviceIdentity::generate("bob").unwrap();
        let mut store = TrustStore::new();
        assert!(!store.is_trusted(&peer.pin()));
        store.pair("bob", peer.pin(), "2026-10-03T00:00:00Z");
        assert!(store.is_trusted(&peer.pin()));
    }

    #[test]
    fn revoke_blocks_reconnect() {
        let peer = DeviceIdentity::generate("bob").unwrap();
        let mut store = TrustStore::new();
        store.pair("bob", peer.pin(), "2026-10-03T00:00:00Z");
        assert!(store.revoke(&peer.pin()));
        assert!(!store.is_trusted(&peer.pin()));
        assert_eq!(store.get(&peer.pin()).unwrap().trust_generation, 1);
    }

    #[test]
    fn unknown_peer_not_trusted() {
        let peer = DeviceIdentity::generate("eve").unwrap();
        let store = TrustStore::new();
        assert!(!store.is_trusted(&peer.pin()));
    }

    #[test]
    fn store_serde_roundtrip() {
        let peer = DeviceIdentity::generate("bob").unwrap();
        let mut store = TrustStore::new();
        store.pair("bob", peer.pin(), "2026-10-03T00:00:00Z");
        let json = serde_json::to_string(&store).unwrap();
        let back: TrustStore = serde_json::from_str(&json).unwrap();
        assert!(back.is_trusted(&peer.pin()));
    }
}
