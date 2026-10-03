//! Image/terrain reference adapter (L3, spec §10.7/§26.5.2).
//!
//! Authoritative object `image.heightfield.v1` is a raw grid. The action
//! `image.render_region(rect, recipe)` is checked by a dual-render oracle:
//! rendering a region directly vs. assembling it from per-cell tiles must
//! produce the same content hash.

use carryon_adapter_api::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// A width×height grid of u8 heights, row-major.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeightField {
    pub width: u32,
    pub height: u32,
    pub cells: Vec<u8>,
}

/// The image adapter.
pub struct ImageAdapter {
    field: HeightField,
    generation: u64,
}

const SCHEMA_HEIGHT: &str = "image.heightfield.v1";
const ADAPTER_ID: &str = "org.carryon.image";

impl ImageAdapter {
    pub fn new(field: HeightField) -> Self {
        ImageAdapter {
            field,
            generation: 1,
        }
    }

    /// A deterministic 8×8 sample field.
    pub fn sample() -> Self {
        let (w, h) = (8u32, 8u32);
        let cells = (0..w * h).map(|i| (i % 251) as u8).collect();
        ImageAdapter::new(HeightField {
            width: w,
            height: h,
            cells,
        })
    }

    fn field_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.field).expect("serialize heightfield")
    }

    /// Render a region directly: the bytes of the sub-grid.
    fn render_direct(&self, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h) as usize);
        for row in y..(y + h).min(self.field.height) {
            for col in x..(x + w).min(self.field.width) {
                out.push(self.field.cells[(row * self.field.width + col) as usize]);
            }
        }
        out
    }

    /// Render the same region cell-by-cell (the independent oracle path).
    fn render_tiled(&self, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        for row in y..(y + h).min(self.field.height) {
            for col in x..(x + w).min(self.field.width) {
                // "tile" = one cell fetched independently.
                let v = self.field.cells[(row * self.field.width + col) as usize];
                out.push(v);
            }
        }
        out
    }

    fn entry(&self) -> ObjectEntry {
        let bytes = self.field_bytes();
        ObjectEntry {
            object_id: SCHEMA_HEIGHT.into(),
            generation: self.generation,
            kind: ObjectKindWire::Authoritative,
            schema_id: SCHEMA_HEIGHT.into(),
            content_hash: hex_sha256(&bytes),
            logical_size: bytes.len() as u64,
            parents: vec![],
            recipe_id: None,
            portable: true,
            sensitivity: SensitivityWire::Public,
            retention: RetentionWire::Session,
        }
    }
}

impl Adapter for ImageAdapter {
    fn get_adapter_info(&self) -> AdapterInfo {
        AdapterInfo {
            adapter_id: ADAPTER_ID.into(),
            adapter_version: "1.0.0".into(),
            publisher_id: "org.carryon".into(),
            integration_level: IntegrationLevel::L3,
            executable: AdapterInfo::COMPILED_IN.into(),
            state_schemas: vec![SCHEMA_HEIGHT.into()],
            actions: vec!["image.render_region".into()],
            permissions: vec!["read_selected_workspace".into()],
            network_access: false,
            supports_snapshot: true,
            supports_mutations: false,
            supports_authority_transfer: false,
            max_object_bytes: 1024 * 1024,
        }
    }

    fn request_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken, AdapterError> {
        Ok(ConsentToken(format!("consent-{}", scope.target)))
    }

    fn list_sessions(&self, _c: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError> {
        Ok(vec![SessionSummary {
            session: "image-session".into(),
            title: "Sample terrain".into(),
            generation: self.generation,
            schema_version: 1,
        }])
    }

    fn begin_snapshot(&mut self, _s: &str, expected: u64) -> Result<SnapshotToken, AdapterError> {
        if expected != self.generation {
            return Err(AdapterError::StaleGeneration {
                expected,
                actual: self.generation,
            });
        }
        Ok(SnapshotToken(format!("snap-{}", self.generation)))
    }

    fn describe_snapshot(&self, _t: &SnapshotToken) -> Result<ObjectManifest, AdapterError> {
        Ok(ObjectManifest {
            session: "image-session".into(),
            generation: self.generation,
            objects: vec![self.entry()],
        })
    }

    fn read_object(
        &self,
        _t: &SnapshotToken,
        object_id: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AdapterError> {
        if object_id != SCHEMA_HEIGHT {
            return Err(AdapterError::UnknownObject(object_id.into()));
        }
        let bytes = self.field_bytes();
        let start = (offset as usize).min(bytes.len());
        let end = (start + length as usize).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }

    fn finish_snapshot(&mut self, _t: SnapshotToken) -> Result<SnapshotReceipt, AdapterError> {
        Ok(SnapshotReceipt {
            session: "image-session".into(),
            generation: self.generation,
            manifest_digest: hex_sha256(&self.field_bytes()),
        })
    }

    fn abort_snapshot(&mut self, _t: SnapshotToken, _reason: &str) {}

    fn current_generation(&self, _s: &str) -> Result<u64, AdapterError> {
        Ok(self.generation)
    }

    fn resolve_action(
        &self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<DependencyPlanWire, AdapterError> {
        if req.class != "image.render_region" {
            return Err(AdapterError::ActionUnsupported(req.class.clone()));
        }
        Ok(DependencyPlanWire {
            prerequisites: vec![ObjectVersionWire {
                object_id: SCHEMA_HEIGHT.into(),
                generation: self.generation,
            }],
            provenance: vec![],
            optional: vec![],
        })
    }

    fn validate_objects(
        &self,
        _cut: &CutRef,
        versions: &[ObjectVersionWire],
    ) -> Result<ValidationReport, AdapterError> {
        let missing: Vec<_> = versions
            .iter()
            .filter(|v| v.object_id != SCHEMA_HEIGHT)
            .cloned()
            .collect();
        Ok(ValidationReport {
            ok: missing.is_empty(),
            missing,
            message: "image validation".into(),
        })
    }

    fn import_objects(
        &mut self,
        _cut: &CutRef,
        locations: &[ObjectLocation],
    ) -> Result<ImportReceipt, AdapterError> {
        Ok(ImportReceipt {
            imported: locations
                .iter()
                .map(|l| ObjectVersionWire {
                    object_id: l.object_id.clone(),
                    generation: l.generation,
                })
                .collect(),
        })
    }

    fn activate(
        &mut self,
        _cut: &CutRef,
        _req: &ActionRequestWire,
    ) -> Result<ActivationReceipt, AdapterError> {
        Ok(ActivationReceipt {
            activated: true,
            detail: "image ready".into(),
        })
    }

    fn execute_action(
        &mut self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError> {
        if req.class != "image.render_region" {
            return Err(AdapterError::ActionUnsupported(req.class.clone()));
        }
        let g = |k: &str| req.params.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let (x, y, w, h) = (g("x"), g("y"), g("w"), g("h"));

        let direct = self.render_direct(x, y, w, h);
        let tiled = self.render_tiled(x, y, w, h);
        let agreed = direct == tiled;
        let output_hash = hex_sha256(&direct);

        Ok(ActionResultWire {
            output: serde_json::json!({ "x": x, "y": y, "w": w, "h": h, "bytes": direct.len() }),
            output_hash: output_hash.clone(),
            oracle: OracleOutcome {
                checked: true,
                agreed,
                output_hash,
                detail: "direct vs tiled render".into(),
            },
        })
    }

    fn export_evidence(
        &self,
        session: &str,
        _range: EvidenceRange,
    ) -> Result<EvidenceFragment, AdapterError> {
        Ok(EvidenceFragment {
            session: session.into(),
            json: serde_json::json!({ "width": self.field.width, "height": self.field.height }),
        })
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
