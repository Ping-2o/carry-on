//! Service discovery contract (spec §18.3, DNS-SD `_carryon._tcp`).
//!
//! # Scope honesty
//!
//! The TXT contract, its fail-closed receive filter, and the live mDNS backend
//! ([`crate::discovery_mdns`], stage 4) live in the ENGINE. Headless loopback
//! tests drive the flow through the `Discovery` trait with a static in-memory
//! source; the real multicast backend is exercised on a LAN (its live test is
//! `#[ignore]`d — CI runners cannot multicast).
use serde::{Deserialize, Serialize};

/// The production service type (§18.3).
pub const SERVICE_TYPE: &str = "_carryon._tcp";

/// The five §18.3 TXT keys, in canonical order. Nothing else is ever set,
/// read, or interpreted — foreign keys on a received record are IGNORED
/// (counted, never trusted) and construction here can only ever emit these.
pub const TXT_KEYS: [&str; 5] = ["pm", "id", "pr", "dc", "ch"];

/// One DNS-SD TXT key/value pair is length-prefixed by a single byte
/// (RFC 6762 §6.4 / RFC 6763 §6.1) — a pair longer than 255 bytes is not
/// representable and must fail closed rather than be truncated.
pub const MAX_TXT_PAIR_BYTES: usize = 255;

/// A TXT-contract violation on a RECEIVED record (§18.3 fail-closed filter).
/// A malformed record never becomes a peer: the engine refuses it, it does
/// not guess.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("TXT contract violation: {0}")]
pub struct TxtError(pub String);

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

    fn parse(s: &str) -> Result<Self, TxtError> {
        match s {
            "desktop" => Ok(DeviceClass::Desktop),
            "mobile" => Ok(DeviceClass::Mobile),
            "unknown" => Ok(DeviceClass::Unknown),
            other => Err(TxtError(format!("unknown device class {other:?}"))),
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
    /// Canonical: exactly the five §18.3 keys, in `TXT_KEYS` order.
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        vec![
            ("pm".into(), self.protocol_major.to_string()),
            ("id".into(), self.instance_id.clone()),
            ("pr".into(), (self.pairing_required as u8).to_string()),
            ("dc".into(), self.device_class.as_str().into()),
            ("ch".into(), self.connection_hint.clone()),
        ]
    }

    /// Fail-closed parse of a RECEIVED TXT record (§18.3 receive filter — the
    /// engine owns the contract; the platform responder never decides).
    ///
    /// Rules:
    /// - every required key (`pm id pr dc ch`) must be present and well-formed;
    ///   anything missing or malformed is a [`TxtError`] — the peer is dropped,
    ///   never repaired by guessing;
    /// - foreign (unknown) keys are IGNORED — they are returned separately and
    ///   carry no meaning here (TXT minimality is enforced on receive too);
    /// - `pr` accepts the two encodings the pilots have used (`1`/`true`) and
    ///   canonicalizes on `1`/`0`;
    /// - a pair over [`MAX_TXT_PAIR_BYTES`] is not representable on the wire
    ///   and fails closed.
    pub fn from_pairs(pairs: &[(String, String)]) -> Result<(TxtRecord, Vec<String>), TxtError> {
        let mut pm: Option<u16> = None;
        let mut id: Option<String> = None;
        let mut pr: Option<bool> = None;
        let mut dc: Option<DeviceClass> = None;
        let mut ch: Option<String> = None;
        let mut ignored: Vec<String> = Vec::new();

        for (key, value) in pairs {
            if key.len() + value.len() + 1 > MAX_TXT_PAIR_BYTES {
                return Err(TxtError(format!(
                    "TXT pair {key:?} is {} bytes, over the {MAX_TXT_PAIR_BYTES}-byte DNS limit",
                    key.len() + value.len() + 1
                )));
            }
            match key.as_str() {
                "pm" => {
                    if pm.is_some() {
                        return Err(TxtError("duplicate key pm".into()));
                    }
                    let v: u16 = value.parse().map_err(|_| {
                        TxtError(format!("pm {value:?} is not a protocol major version"))
                    })?;
                    pm = Some(v);
                }
                "id" => {
                    if id.is_some() {
                        return Err(TxtError("duplicate key id".into()));
                    }
                    if value.is_empty() || value.len() > 64 {
                        return Err(TxtError(format!(
                            "id must be 1..=64 chars, got {}",
                            value.len()
                        )));
                    }
                    if !value.bytes().all(|b| (0x21..=0x7E).contains(&b)) {
                        return Err(TxtError("id must be printable ASCII without spaces".into()));
                    }
                    id = Some(value.clone());
                }
                "pr" => {
                    if pr.is_some() {
                        return Err(TxtError("duplicate key pr".into()));
                    }
                    pr = Some(match value.as_str() {
                        "1" | "true" => true,
                        "0" | "false" => false,
                        other => {
                            return Err(TxtError(format!("pr {other:?} is not a pairing flag")))
                        }
                    });
                }
                "dc" => {
                    if dc.is_some() {
                        return Err(TxtError("duplicate key dc".into()));
                    }
                    dc = Some(DeviceClass::parse(value)?);
                }
                "ch" => {
                    if ch.is_some() {
                        return Err(TxtError("duplicate key ch".into()));
                    }
                    // A hint, never a dial target: empty, "none", or a port number.
                    if value.len() > 64 {
                        return Err(TxtError(format!("ch is {} chars, max 64", value.len())));
                    }
                    if !value.is_empty() && value != "none" {
                        let port: u16 = value.parse().map_err(|_| {
                            TxtError(format!("ch {value:?} is not a connection hint"))
                        })?;
                        if port == 0 {
                            return Err(TxtError("ch port 0 is not a connection hint".into()));
                        }
                    }
                    ch = Some(value.clone());
                }
                // §18.3 minimality on receive: a foreign key is data we do not
                // have semantics for — ignore it, never interpret it.
                _ => ignored.push(key.clone()),
            }
        }

        // Fail closed on ANY missing required key.
        let Some(protocol_major) = pm else {
            return Err(TxtError("required key pm missing".into()));
        };
        let Some(instance_id) = id else {
            return Err(TxtError("required key id missing".into()));
        };
        let Some(pairing_required) = pr else {
            return Err(TxtError("required key pr missing".into()));
        };
        let Some(device_class) = dc else {
            return Err(TxtError("required key dc missing".into()));
        };
        let Some(connection_hint) = ch else {
            return Err(TxtError("required key ch missing".into()));
        };

        Ok((
            TxtRecord {
                protocol_major,
                instance_id,
                pairing_required,
                device_class,
                connection_hint,
            },
            ignored,
        ))
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

    // ---- §18.3 fail-closed receive filter ----------------------------------

    fn canonical_pairs() -> Vec<(String, String)> {
        TxtRecord {
            protocol_major: 1,
            instance_id: "ab12cd34".into(),
            pairing_required: true,
            device_class: DeviceClass::Mobile,
            connection_hint: "54321".into(),
        }
        .to_pairs()
    }

    #[test]
    fn txt_roundtrip_construct_then_parse() {
        let (parsed, ignored) =
            TxtRecord::from_pairs(&canonical_pairs()).expect("canonical parses");
        assert_eq!(parsed.protocol_major, 1);
        assert_eq!(parsed.instance_id, "ab12cd34");
        assert!(parsed.pairing_required);
        assert_eq!(parsed.device_class, DeviceClass::Mobile);
        assert_eq!(parsed.connection_hint, "54321");
        assert!(ignored.is_empty(), "no foreign keys in a canonical record");
    }

    #[test]
    fn txt_receive_ignores_foreign_keys() {
        let mut pairs = canonical_pairs();
        pairs.push(("evil_key".into(), "never interpreted".into()));
        pairs.push(("user".into(), "alice".into()));
        let (parsed, ignored) = TxtRecord::from_pairs(&pairs).expect("foreign keys ignored");
        assert_eq!(parsed.instance_id, "ab12cd34");
        assert_eq!(ignored, vec!["evil_key", "user"]);
    }

    #[test]
    fn txt_receive_accepts_both_pr_encodings() {
        for v in ["1", "true"] {
            let mut pairs = canonical_pairs();
            pairs[2] = ("pr".into(), v.into());
            let (parsed, _) = TxtRecord::from_pairs(&pairs).expect(v);
            assert!(parsed.pairing_required, "pr={v} must parse as required");
        }
        for v in ["0", "false"] {
            let mut pairs = canonical_pairs();
            pairs[2] = ("pr".into(), v.into());
            let (parsed, _) = TxtRecord::from_pairs(&pairs).expect(v);
            assert!(
                !parsed.pairing_required,
                "pr={v} must parse as not required"
            );
        }
    }

    #[test]
    fn txt_missing_required_key_fails_closed() {
        for drop_key in TXT_KEYS {
            let pairs: Vec<_> = canonical_pairs()
                .into_iter()
                .filter(|(k, _)| k != drop_key)
                .collect();
            let err = TxtRecord::from_pairs(&pairs).expect_err("missing key must refuse");
            assert!(
                err.0.contains(drop_key),
                "error should name {drop_key}: {err}"
            );
        }
    }

    #[test]
    fn txt_malformed_values_fail_closed() {
        let cases: Vec<Vec<(String, String)>> = vec![
            vec![("pm".into(), "one".into())], // not a number
            vec![("id".into(), "".into())],    // empty instance id
            vec![("id".into(), "has space".into())],
            vec![("pr".into(), "maybe".into())],
            vec![("dc".into(), "iPhone 14".into())], // a model string, not a class
            vec![("ch".into(), "; rm -rf /".into())], // not a hint
            vec![("ch".into(), "0".into())],
            vec![("pm".into(), "1".into()), ("pm".into(), "2".into())], // duplicate
        ];
        // A record containing only the tested key must fail on it (the other
        // required keys are appended so the tested key is the only failure).
        for base_key_only in cases {
            let mut pairs = canonical_pairs();
            let (k, v) = base_key_only[0].clone();
            let idx = TXT_KEYS
                .iter()
                .position(|key| *key == k)
                .expect("known key");
            pairs[idx] = (k.clone(), v.clone());
            if base_key_only.len() > 1 {
                // duplicate-key case: append the second occurrence
                pairs.push(base_key_only[1].clone());
            }
            TxtRecord::from_pairs(&pairs).expect_err(&format!("{k}={v:?} must refuse"));
        }
    }

    #[test]
    fn txt_oversized_pair_fails_closed() {
        let pairs = vec![("id".into(), "x".repeat(MAX_TXT_PAIR_BYTES))];
        TxtRecord::from_pairs(&pairs).expect_err("over-255-byte pair must refuse");
    }

    #[test]
    fn txt_minimality_on_construction() {
        // The construction side can only ever emit the five §18.3 keys.
        let blob = format!("{:?}", canonical_pairs());
        assert!(blob.contains("pm") && blob.contains("ch"));
        for forbidden in ["user", "session", "sha256", "pubkey", "BEGIN", "title"] {
            assert!(!blob.contains(forbidden), "TXT leaked {forbidden}");
        }
    }
}
