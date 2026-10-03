//! Shared test helpers for carryon-core integration tests.

use carryon_core::ids::Digest;
use carryon_core::ids::ObjectId;
use carryon_core::model::object::{Object, ObjectKind, Retention, Sensitivity};
use carryon_core::store::ChunkSource;
use carryon_core::Result;

/// An in-memory byte source for publish tests.
pub struct Bytes(pub Vec<u8>);

impl ChunkSource for Bytes {
    fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>> {
        let start = (offset as usize).min(self.0.len());
        let end = (start + length as usize).min(self.0.len());
        Ok(self.0[start..end].to_vec())
    }
}

/// A byte source that lies: it serves `served` but the object declares the hash
/// of `declared` — used to prove digest-mismatch fails closed.
pub struct LyingBytes {
    pub served: Vec<u8>,
}

impl ChunkSource for LyingBytes {
    fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>> {
        let start = (offset as usize).min(self.served.len());
        let end = (start + length as usize).min(self.served.len());
        Ok(self.served[start..end].to_vec())
    }
}

/// Build an authoritative object declaring the digest of `bytes`.
pub fn object_of(id: &str, bytes: &[u8]) -> Object {
    Object {
        id: ObjectId(id.into()),
        kind: ObjectKind::Authoritative,
        generation: 1,
        schema_id: "test.v1".into(),
        content_hash: Digest::of(bytes),
        wire_hash: None,
        logical_size: bytes.len() as u64,
        wire_size: None,
        parents: vec![],
        recipe_id: None,
        portable: true,
        sensitivity: Sensitivity::Public,
        retention: Retention::Session,
    }
}
