//! Destination-side reconstruction of a carried PhotoCraft session from the
//! sealed cut's object digests — the shared logic behind the C ABI's
//! `carryon_photocraft_restore` and the stage-10 live integration harness.
//!
//! Engine-independent by design: the caller passes the cut's
//! `(object_id, generation, content_hash)` rows (the core already records them)
//! and a reader that returns an object's bytes from its own store. Nothing here
//! touches a network, an executable path, or the source machine.

use crate::model::{
    DocumentBundle, DocumentEncoding, PhotocraftMeta, SCHEMA_DOC, SCHEMA_DOC_MANIFEST, SCHEMA_META,
    SCHEMA_UNSAVED,
};
use std::collections::BTreeMap;

/// State re-materialized from a destination store.
pub struct CarriedState {
    /// Monolithic saved document (`.pcraft` ZIP), when that encoding was used.
    pub document: Option<Vec<u8>>,
    /// Split saved document (manifest + tile/blob parts), when used.
    pub bundle: Option<DocumentBundle>,
    /// Dirty-state snapshot ZIP (the exact current state), when the source was dirty.
    pub unsaved: Option<Vec<u8>>,
    pub meta: PhotocraftMeta,
    /// PhotoCraft objects found in the cut (the 3 authoritative + every part).
    pub objects: usize,
    /// Logical bytes re-materialized (document closure + unsaved).
    pub bytes: u64,
}

/// Build [`CarriedState`] from a sealed cut's digest rows.
///
/// Fail closed when a required object is missing from the cut or unreadable:
/// a restore never opens a partial document.
pub fn reconstruct_carried<F>(
    rows: &[(String, u64, String)],
    mut read: F,
) -> Result<CarriedState, String>
where
    F: FnMut(&str) -> Result<Vec<u8>, String>,
{
    if rows.is_empty() {
        return Err("the cut has no sealed objects (import did not complete?)".into());
    }
    let by_id: BTreeMap<&str, &str> = rows
        .iter()
        .map(|(id, _gen, hash)| (id.as_str(), hash.as_str()))
        .collect();
    let mut read_id = |id: &str| -> Result<Vec<u8>, String> {
        let hex = by_id
            .get(id)
            .ok_or_else(|| format!("carried object {id} is not in the sealed cut"))?;
        read(hex)
    };

    let meta: PhotocraftMeta = serde_json::from_slice(&read_id(SCHEMA_META)?)
        .map_err(|e| format!("decode photocraft meta: {e}"))?;
    let unsaved = read_id(SCHEMA_UNSAVED)?;
    let mut bytes = 0u64;
    let objects = rows
        .iter()
        .filter(|(id, _, _)| id.starts_with("photocraft."))
        .count();

    let (document, bundle) = match meta.document_encoding {
        DocumentEncoding::Split => {
            let manifest = read_id(SCHEMA_DOC_MANIFEST)?;
            bytes += manifest.len() as u64;
            let mut parts = BTreeMap::new();
            for (id, _gen, hex) in rows {
                let kind = if id.starts_with("photocraft.tile.") {
                    "tiles"
                } else if id.starts_with("photocraft.blob.") {
                    "blobs"
                } else {
                    continue;
                };
                let stem = id
                    .trim_start_matches("photocraft.")
                    .trim_start_matches("tile.")
                    .trim_start_matches("blob.")
                    .trim_end_matches(".v1");
                let part = read(hex)?;
                bytes += part.len() as u64;
                parts.insert(format!("{kind}/{stem}.zst"), part);
            }
            (None, Some(DocumentBundle { manifest, parts }))
        }
        DocumentEncoding::Monolithic => {
            let doc = read_id(SCHEMA_DOC)?;
            bytes += doc.len() as u64;
            (Some(doc), None)
        }
    };
    bytes += unsaved.len() as u64;
    Ok(CarriedState {
        document,
        bundle,
        unsaved: if unsaved.is_empty() {
            None
        } else {
            Some(unsaved)
        },
        meta,
        objects,
        bytes,
    })
}
