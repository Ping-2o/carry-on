//! Logical cut (spec §6.2): an immutable statement of the versions defining a
//! handoff boundary. Correctness is defined against a cut, not "whatever was on
//! the source when a packet arrived" (§6.2).

use crate::ids::{Digest, Epoch, ObjectVersion, SessionId};
use serde::{Deserialize, Serialize};

/// A derived-object validity statement carried in a cut (§6.2 optional).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedValidity {
    pub object_id: String,
    pub generation: u64,
    pub valid: bool,
    pub reason: String,
}

/// An immutable cut (spec §6.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cut {
    pub session: SessionId,
    pub number: u64,
    pub epoch: Epoch,
    /// Ordered authoritative object-version vector.
    pub authoritative_versions: Vec<ObjectVersion>,
    pub adapter_schema_version: u32,
    pub recipe_versions: Vec<(String, String)>,
    pub derived_validity: Vec<DerivedValidity>,
    /// Source device signature. `None` in Phase 1 (no device keys yet).
    pub source_signature: Option<Vec<u8>>,
    pub created_utc: String,
    pub manifest_digest: Digest,
}

impl Cut {
    /// Deterministic canonical bytes for the cut's identity, independent of
    /// field order and of audit-only fields (`created_utc`, `source_signature`).
    ///
    /// Built as an explicitly-ordered JSON string so two runs with the same
    /// logical content produce byte-identical output (the digest is over these
    /// bytes). `created_utc` is excluded because it is audit-only (§6.2) and must
    /// not change the cut identity.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut s = String::new();
        s.push('{');
        s.push_str(&format!("\"session\":\"{}\",", self.session));
        s.push_str(&format!("\"number\":{},", self.number));
        s.push_str(&format!("\"epoch\":{},", self.epoch.0));
        s.push_str(&format!(
            "\"adapter_schema_version\":{},",
            self.adapter_schema_version
        ));

        s.push_str("\"authoritative_versions\":[");
        for (i, v) in self.authoritative_versions.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"id\":\"{}\",\"generation\":{}}}",
                v.id.0, v.generation
            ));
        }
        s.push_str("],");

        s.push_str("\"recipe_versions\":[");
        for (i, (k, v)) in self.recipe_versions.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("[\"{k}\",\"{v}\"]"));
        }
        s.push_str("],");

        s.push_str("\"derived_validity\":[");
        for (i, d) in self.derived_validity.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"object_id\":\"{}\",\"generation\":{},\"valid\":{}}}",
                d.object_id, d.generation, d.valid
            ));
        }
        s.push(']');
        s.push('}');
        s.into_bytes()
    }

    /// Compute the manifest digest from the canonical bytes.
    pub fn compute_digest(&self) -> Digest {
        Digest::of(&self.canonical_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ObjectId;

    fn version(id: &str, g: u64) -> ObjectVersion {
        ObjectVersion {
            id: ObjectId(id.into()),
            generation: g,
        }
    }

    #[test]
    fn canonical_bytes_ignore_audit_fields() {
        let base = Cut {
            session: SessionId::new(),
            number: 1,
            epoch: Epoch(0),
            authoritative_versions: vec![
                version("graph.nodes.v1", 1),
                version("graph.edges.v1", 1),
            ],
            adapter_schema_version: 2,
            recipe_versions: vec![("csr".into(), "1.0".into())],
            derived_validity: vec![],
            source_signature: None,
            created_utc: "2026-10-02T00:00:00Z".into(),
            manifest_digest: Digest([0; 32]),
        };
        let mut other = base.clone();
        other.created_utc = "2026-10-02T23:59:59Z".into();
        other.source_signature = Some(vec![1, 2, 3]);
        // Audit-only fields must not change the identity digest.
        assert_eq!(base.compute_digest(), other.compute_digest());
    }

    #[test]
    fn digest_changes_with_versions() {
        let mut c = Cut {
            session: SessionId::new(),
            number: 1,
            epoch: Epoch(0),
            authoritative_versions: vec![version("a", 1)],
            adapter_schema_version: 1,
            recipe_versions: vec![],
            derived_validity: vec![],
            source_signature: None,
            created_utc: "t".into(),
            manifest_digest: Digest([0; 32]),
        };
        let d1 = c.compute_digest();
        c.authoritative_versions[0].generation = 2;
        assert_ne!(d1, c.compute_digest());
    }
}
