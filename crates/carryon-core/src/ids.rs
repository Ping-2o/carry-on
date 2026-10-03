//! Typed identifiers. Newtypes prevent mixing a session id with a cut number,
//! an object id with a digest, and so on.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::fmt;
use uuid::Uuid;

/// A logical session identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        SessionId(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A cut reference: session + monotonically increasing cut number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CutId {
    pub session: SessionId,
    pub number: u64,
}

/// Adapter-namespaced logical object identifier (e.g. `"graph.nodes.v1"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectId(pub String);

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Object identity by (id, generation).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectVersion {
    pub id: ObjectId,
    pub generation: u64,
}

/// Monotonic logical session generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Generation(pub u64);

/// Monotonic authority epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Epoch(pub u64);

/// A transfer identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransferId(pub Uuid);

impl TransferId {
    pub fn new() -> Self {
        TransferId(Uuid::new_v4())
    }
}

impl Default for TransferId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TransferId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A SHA-256 content digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    /// Hash arbitrary bytes.
    pub fn of(bytes: &[u8]) -> Self {
        let mut h = Sha256::new();
        h.update(bytes);
        let out = h.finalize();
        let mut d = [0u8; 32];
        d.copy_from_slice(&out);
        Digest(d)
    }

    /// Lowercase hex (64 chars).
    pub fn to_hex(self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// Parse from lowercase/uppercase hex; `None` if not 64 hex chars.
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut d = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hi = (chunk[0] as char).to_digit(16)?;
            let lo = (chunk[1] as char).to_digit(16)?;
            d[i] = (hi * 16 + lo) as u8;
        }
        Some(Digest(d))
    }

    /// Content-addressed relative path fan-out: `objects/sha256/ab/cd/<full>`.
    pub fn rel_path(self) -> String {
        let hex = self.to_hex();
        format!("objects/sha256/{}/{}/{}", &hex[0..2], &hex[2..4], hex)
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// A request/idempotency identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub Uuid);

impl RequestId {
    pub fn new() -> Self {
        RequestId(Uuid::new_v4())
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_hex_roundtrip() {
        let d = Digest::of(b"carry-on");
        let hex = d.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(Digest::from_hex(&hex), Some(d));
    }

    #[test]
    fn rel_path_fanout() {
        let d = Digest::of(b"x");
        let p = d.rel_path();
        let hex = d.to_hex();
        assert_eq!(
            p,
            format!("objects/sha256/{}/{}/{}", &hex[0..2], &hex[2..4], hex)
        );
    }
}
