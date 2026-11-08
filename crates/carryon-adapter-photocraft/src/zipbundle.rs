//! Split a PhotoCraft `.pcraft` ZIP into the adapter's **split document bundle**
//! (manifest object + parent-linked per-tile/per-blob objects,
//! `TILE-SPLITTING.md` §2), without walking through the PhotoCraft engine.
//!
//! Why a ZIP reader lives here: the desktop app's save path (`app.save` over the
//! control channel) writes `.pcraft` **ZIP files** only — it refuses to write a
//! directory bundle (`automation write refused: … is not a regular file`, pinned
//! 0.16.0). PhotoCraft's ZIP and its directory bundle carry the *same* entries
//! (RESEARCH §1.5: `manifest.json`, `tiles/<blake3>.zst`, `blobs/<blake3>.zst`,
//! plus previews), and every entry is **stored** (compression method 0, verified
//! against real files at the pin), so the bundle can be produced from the ZIP
//! bytes the app already gave us — the adapter does not need the pinned engine
//! for a split carry, and the app never forks PhotoCraft.
//!
//! What is carried: `manifest.json` + every `tiles/*.zst` / `blobs/*.zst` entry,
//! bytes exact. What is NOT carried: the caller-rendered previews
//! (`thumb.png`, `composite/preview.png`) — the destination re-renders them
//! (`TILE-SPLITTING.md` §2), exactly as the directory-bundle path does.
//!
//! Fail-closed limits (honest, tested):
//! - compressed (method ≠ 0) entries are refused: a full DEFLATE decoder is not
//!   part of this bridge; PhotoCraft at the pin never writes them.
//! - ZIP64 / multi-disk archives are refused.
//! - duplicate entries, path traversal, and a missing `manifest.json` are
//!   refused.

use crate::model::DocumentBundle;
use std::collections::BTreeMap;

const EOCD: &[u8; 4] = b"PK\x05\x06";
const EOCD64: &[u8; 4] = b"PK\x06\x06";
const CENTRAL: &[u8; 4] = b"PK\x01\x02";
const LOCAL: &[u8; 4] = b"PK\x03\x04";

/// Largest central directory this reader will accept (a `.pcraft` bundle's
/// manifest + entry names; far above any real document).
const MAX_ENTRIES: usize = 65_536;
/// Entry-name sanity cap (bundle paths are short).
const MAX_NAME: usize = 1024;

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Split `.pcraft` ZIP bytes into a [`DocumentBundle`].
///
/// Returns an error string suitable for a driver `Failed` message (no paths,
/// no bytes — safe for evidence).
pub fn split_pcraft_zip(zip: &[u8]) -> Result<DocumentBundle, String> {
    let entries = read_stored_entries(zip)?;
    let mut manifest: Option<Vec<u8>> = None;
    let mut parts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (name, bytes) in entries {
        if name == "manifest.json" {
            if manifest.is_some() {
                return Err("duplicate manifest.json in .pcraft ZIP".into());
            }
            manifest = Some(bytes);
        } else if is_bundle_part(&name) && parts.insert(name.clone(), bytes).is_some() {
            return Err(format!("duplicate bundle entry {name}"));
        }
        // Previews and any other entry are deliberately dropped (re-rendered on
        // the destination); an unexpected *content* entry is ignored exactly as
        // PhotoCraft ignores unreferenced bundle entries on open.
    }
    let manifest = manifest.ok_or_else(|| ".pcraft ZIP has no manifest.json".to_string())?;
    Ok(DocumentBundle { manifest, parts })
}

/// Whether a ZIP entry is a `tiles/<64-hex>.zst` / `blobs/<64-hex>.zst` part —
/// the only content entries a split carry takes.
fn is_bundle_part(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix("tiles/")
        .or_else(|| name.strip_prefix("blobs/"))
    else {
        return false;
    };
    let Some(stem) = rest.strip_suffix(".zst") else {
        return false;
    };
    stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Every entry as `(name, bytes)` in central-directory order, refusing anything
/// but stored (method 0) entries.
fn read_stored_entries(zip: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let eocd = find_eocd(zip)?;
    if zip.get(eocd..eocd + 4) == Some(EOCD64.as_slice()) {
        return Err("ZIP64 archives are not supported by the .pcraft splitter".into());
    }
    let disk = u16_at(zip, eocd + 4).ok_or("truncated EOCD")?;
    let cd_disk = u16_at(zip, eocd + 6).ok_or("truncated EOCD")?;
    if disk != 0 || cd_disk != 0 {
        return Err("multi-disk ZIP archives are not supported".into());
    }
    let count = u16_at(zip, eocd + 10).ok_or("truncated EOCD")? as usize;
    if count > MAX_ENTRIES {
        return Err(format!("too many ZIP entries ({count})"));
    }
    let cd_size = u32_at(zip, eocd + 12).ok_or("truncated EOCD")? as usize;
    let cd_off = u32_at(zip, eocd + 16).ok_or("truncated EOCD")? as usize;
    let cd_end = cd_off
        .checked_add(cd_size)
        .filter(|&e| e <= eocd)
        .ok_or("central directory out of bounds")?;

    let mut out = Vec::with_capacity(count);
    let mut pos = cd_off;
    for _ in 0..count {
        if zip.get(pos..pos + 4) != Some(CENTRAL.as_slice()) {
            return Err("bad central directory entry signature".into());
        }
        let method = u16_at(zip, pos + 10).ok_or("truncated central entry")?;
        let comp_size = u32_at(zip, pos + 20).ok_or("truncated central entry")? as usize;
        let uncomp_size = u32_at(zip, pos + 24).ok_or("truncated central entry")? as usize;
        let name_len = u16_at(zip, pos + 28).ok_or("truncated central entry")? as usize;
        let extra_len = u16_at(zip, pos + 30).ok_or("truncated central entry")? as usize;
        let comment_len = u16_at(zip, pos + 32).ok_or("truncated central entry")? as usize;
        let local_off = u32_at(zip, pos + 42).ok_or("truncated central entry")? as usize;
        if name_len == 0 || name_len > MAX_NAME {
            return Err(format!("bad ZIP entry name length {name_len}"));
        }
        let name_bytes = zip
            .get(pos + 46..pos + 46 + name_len)
            .ok_or("truncated ZIP entry name")?;
        let name = String::from_utf8(name_bytes.to_vec())
            .map_err(|_| "ZIP entry name is not UTF-8".to_string())?;
        if name.contains("..") || name.starts_with('/') || name.contains('\\') {
            return Err(format!("refusing unsafe ZIP entry name {name:?}"));
        }
        if method != 0 {
            return Err(format!(
                "ZIP entry {name} uses compression method {method}; \
                 the .pcraft splitter only accepts stored entries"
            ));
        }
        if comp_size != uncomp_size {
            return Err(format!("stored ZIP entry {name} has mismatched sizes"));
        }
        // Local header: name + extra lengths differ from the central copy.
        if zip.get(local_off..local_off + 4) != Some(LOCAL.as_slice()) {
            return Err(format!("bad local header for {name}"));
        }
        let ln = u16_at(zip, local_off + 26).ok_or("truncated local header")? as usize;
        let le = u16_at(zip, local_off + 28).ok_or("truncated local header")? as usize;
        let data_off = local_off + 30 + ln + le;
        let bytes = zip
            .get(data_off..data_off + comp_size)
            .ok_or_else(|| format!("truncated data for {name}"))?
            .to_vec();
        out.push((name, bytes));
        pos += 46 + name_len + extra_len + comment_len;
        if pos > cd_end {
            return Err("central directory entry overruns its bounds".into());
        }
    }
    Ok(out)
}

/// Locate the End Of Central Directory record (the comment may be non-empty;
/// ZIP comments are ≤ 65535 bytes).
fn find_eocd(zip: &[u8]) -> Result<usize, String> {
    if zip.len() < 22 {
        return Err("file too small to be a ZIP".into());
    }
    let lo = zip.len().saturating_sub(22 + 65_535);
    let mut found = None;
    let mut i = zip.len() - 22;
    loop {
        if zip.get(i..i + 4) == Some(EOCD.as_slice()) {
            found = Some(i);
            break;
        }
        if i == lo {
            break;
        }
        i -= 1;
    }
    found.ok_or_else(|| "ZIP end-of-central-directory record not found".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal **stored** ZIP writer for tests only (no CRC checking on read).
    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let off = out.len();
            let name_b = name.as_bytes();
            // Local header
            out.extend_from_slice(LOCAL);
            out.extend_from_slice(&20u16.to_le_bytes()); // version
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
            out.extend_from_slice(&0u16.to_le_bytes()); // time
            out.extend_from_slice(&0u16.to_le_bytes()); // date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc (ignored)
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra
            out.extend_from_slice(name_b);
            out.extend_from_slice(data);
            // Central entry
            central.extend_from_slice(CENTRAL);
            central.extend_from_slice(&20u16.to_le_bytes()); // made by
            central.extend_from_slice(&20u16.to_le_bytes()); // needed
            central.extend_from_slice(&0u16.to_le_bytes()); // flags
            central.extend_from_slice(&0u16.to_le_bytes()); // method
            central.extend_from_slice(&0u16.to_le_bytes()); // time
            central.extend_from_slice(&0u16.to_le_bytes()); // date
            central.extend_from_slice(&0u32.to_le_bytes()); // crc
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra
            central.extend_from_slice(&0u16.to_le_bytes()); // comment
            central.extend_from_slice(&0u16.to_le_bytes()); // disk
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&(off as u32).to_le_bytes());
            central.extend_from_slice(name_b);
        }
        let cd_off = out.len();
        out.extend_from_slice(&central);
        out.extend_from_slice(EOCD);
        out.extend_from_slice(&0u16.to_le_bytes()); // disk
        out.extend_from_slice(&0u16.to_le_bytes()); // cd disk
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&(cd_off as u32).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment
        out
    }

    fn hex64(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    #[test]
    fn splits_manifest_and_tiles_and_drops_previews() {
        let tile = format!("tiles/{}.zst", hex64('a'));
        let blob = format!("blobs/{}.zst", hex64('b'));
        let zip = make_zip(&[
            ("manifest.json", br#"{"version":1}"#),
            ("thumb.png", b"thumb"),
            ("composite/preview.png", b"preview"),
            (tile.as_str(), b"tile-bytes"),
            (blob.as_str(), b"blob-bytes"),
        ]);
        let b = split_pcraft_zip(&zip).unwrap();
        assert_eq!(b.manifest, br#"{"version":1}"#);
        assert_eq!(b.parts.len(), 2);
        assert_eq!(b.parts[tile.as_str()], b"tile-bytes");
        assert_eq!(b.parts[blob.as_str()], b"blob-bytes");
        assert!(b.part_entries().len() == 2);
    }

    #[test]
    fn fails_closed_without_manifest_or_on_unsafe_entries() {
        let tile = format!("tiles/{}.zst", hex64('c'));
        let no_manifest = make_zip(&[(tile.as_str(), b"x")]);
        assert!(split_pcraft_zip(&no_manifest)
            .unwrap_err()
            .contains("manifest"));
        let traversal = make_zip(&[("manifest.json", b"{}"), ("tiles/../../evil.zst", b"x")]);
        assert!(split_pcraft_zip(&traversal).is_err());
        let weird = make_zip(&[("manifest.json", b"{}"), ("tiles/notahash.zst", b"x")]);
        let b = split_pcraft_zip(&weird).unwrap();
        assert!(
            b.parts.is_empty(),
            "non-content-addressed names are ignored"
        );
    }

    #[test]
    fn compressed_entries_are_refused() {
        let mut zip = make_zip(&[
            ("manifest.json", b"{}"),
            (&format!("tiles/{}.zst", hex64('d')), b"data"),
        ]);
        // Flip the method to DEFLATE (8) in both the central and local headers.
        // The local header starts at 0; its method field is at offset 8.
        let cd = find_eocd(&zip).unwrap();
        let cd_off = u32_at(&zip, cd + 16).unwrap() as usize;
        zip[8..10].copy_from_slice(&8u16.to_le_bytes());
        zip[cd_off + 10..cd_off + 12].copy_from_slice(&8u16.to_le_bytes());
        assert!(split_pcraft_zip(&zip).unwrap_err().contains("compression"));
    }

    #[test]
    fn real_photo_zip_if_present() {
        // A real `.pcraft` from the repo evidence, when the checkout has it.
        let p = std::path::Path::new("../../docs/artcraft-bridge/evidence/l3-live/base.pcraft");
        if let Ok(bytes) = std::fs::read(p) {
            let b = split_pcraft_zip(&bytes).expect("real photo zip splits");
            assert!(!b.manifest.is_empty());
            assert!(b
                .parts
                .keys()
                .all(|k| k.starts_with("tiles/") || k.starts_with("blobs/")));
        }
    }
}
