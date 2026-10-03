//! Versioned typed objects (spec §6.3/§6.4).

use crate::ids::{Digest, ObjectId, ObjectVersion};
use carryon_adapter_api::{ObjectKindWire, RetentionWire, SensitivityWire};
use serde::{Deserialize, Serialize};

/// Object kind (spec §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectKind {
    /// Acknowledged user state; MUST NOT be regenerated from an older version.
    Authoritative,
    /// Reproducible from pinned parents + recipe.
    Derived,
    /// Disposable acceleration data.
    Cache,
    /// Lower-fidelity user-visible representation.
    Preview,
    /// Never transferred or persisted.
    Ephemeral,
}

/// Sensitivity classification (spec §6.3/§22.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sensitivity {
    Public,
    Personal,
    Confidential,
    Secret,
    Prohibited,
}

impl Sensitivity {
    /// Whether the core excludes objects of this sensitivity by default (ADP-008).
    pub fn is_excluded_by_default(self) -> bool {
        matches!(self, Sensitivity::Secret | Sensitivity::Prohibited)
    }
}

/// Retention policy (spec §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Retention {
    Session,
    Bounded(u64),
    Persistent,
    NoCache,
}

/// A versioned state object (spec §6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Object {
    pub id: ObjectId,
    pub kind: ObjectKind,
    pub generation: u64,
    pub schema_id: String,
    pub content_hash: Digest,
    pub wire_hash: Option<Digest>,
    pub logical_size: u64,
    pub wire_size: Option<u64>,
    pub parents: Vec<ObjectVersion>,
    pub recipe_id: Option<String>,
    pub portable: bool,
    pub sensitivity: Sensitivity,
    pub retention: Retention,
}

impl Object {
    /// This object's (id, generation) identity.
    pub fn version(&self) -> ObjectVersion {
        ObjectVersion {
            id: self.id.clone(),
            generation: self.generation,
        }
    }
}

// --- wire <-> core conversions (used by the host when validating manifests) ---

impl From<ObjectKindWire> for ObjectKind {
    fn from(w: ObjectKindWire) -> Self {
        match w {
            ObjectKindWire::Authoritative => ObjectKind::Authoritative,
            ObjectKindWire::Derived => ObjectKind::Derived,
            ObjectKindWire::Cache => ObjectKind::Cache,
            ObjectKindWire::Preview => ObjectKind::Preview,
            ObjectKindWire::Ephemeral => ObjectKind::Ephemeral,
        }
    }
}

impl From<SensitivityWire> for Sensitivity {
    fn from(w: SensitivityWire) -> Self {
        match w {
            SensitivityWire::Public => Sensitivity::Public,
            SensitivityWire::Personal => Sensitivity::Personal,
            SensitivityWire::Confidential => Sensitivity::Confidential,
            SensitivityWire::Secret => Sensitivity::Secret,
            SensitivityWire::Prohibited => Sensitivity::Prohibited,
        }
    }
}

impl From<RetentionWire> for Retention {
    fn from(w: RetentionWire) -> Self {
        match w {
            RetentionWire::Session => Retention::Session,
            RetentionWire::Bounded => Retention::Bounded(0),
            RetentionWire::Persistent => Retention::Persistent,
            RetentionWire::NoCache => Retention::NoCache,
        }
    }
}

/// String tags for DB storage.
impl ObjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Authoritative => "authoritative",
            ObjectKind::Derived => "derived",
            ObjectKind::Cache => "cache",
            ObjectKind::Preview => "preview",
            ObjectKind::Ephemeral => "ephemeral",
        }
    }
}

impl Sensitivity {
    pub fn as_str(self) -> &'static str {
        match self {
            Sensitivity::Public => "public",
            Sensitivity::Personal => "personal",
            Sensitivity::Confidential => "confidential",
            Sensitivity::Secret => "secret",
            Sensitivity::Prohibited => "prohibited",
        }
    }
}
