//! Append-only, hash-chained event journal (spec §23.1). The journal file is the
//! source of truth for the hash chain; a mirror row is also written to
//! `journal_entries` for queryable indexing.
//!
//! # Frame format
//!
//! Each record is framed as:
//!
//! ```text
//! [ u32 big-endian payload length ][ payload bytes ][ 32-byte chain hash ]
//! ```
//!
//! where `chain_hash = sha256(prev_chain_hash || payload)`. On recovery the
//! reader walks frames and recomputes the chain; at the first frame whose length
//! runs past EOF or whose chain hash does not verify, it truncates the file to
//! the end of the last good record (§19.3.7).

pub mod event;

pub use event::{EventType, JournalEvent};

use crate::error::{CoreError, Result};
use crate::ids::Digest;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The append-only journal.
pub struct Journal {
    path: PathBuf,
    file: File,
    last_hash: [u8; 32],
}

impl Journal {
    /// Open (creating if absent) and recover the journal at `path`, returning the
    /// journal, every valid event read back, and whether a torn tail was
    /// truncated (§19.3.7).
    pub fn open(path: &Path) -> Result<(Journal, Vec<JournalEvent>, bool)> {
        let (events, last_hash, valid_len) = Self::scan(path)?;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let on_disk = file.metadata()?.len();
        let truncated = valid_len < on_disk;
        // Truncate any torn tail to the last valid record.
        file.set_len(valid_len)?;
        let mut file = file;
        file.seek(SeekFrom::End(0))?;

        Ok((
            Journal {
                path: path.to_path_buf(),
                file,
                last_hash,
            },
            events,
            truncated,
        ))
    }

    /// Append an event, extending the hash chain and fsyncing.
    pub fn append(&mut self, ev: &JournalEvent) -> Result<()> {
        let payload = serde_json::to_vec(ev)?;
        let chain = Self::chain(&self.last_hash, &payload);

        let mut frame = Vec::with_capacity(4 + payload.len() + 32);
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload);
        frame.extend_from_slice(&chain);

        self.file.write_all(&frame)?;
        self.file.sync_all()?;
        self.last_hash = chain;
        Ok(())
    }

    /// The current chain head (hex), for evidence.
    pub fn head_hex(&self) -> String {
        Digest(self.last_hash).to_hex()
    }

    /// Read every valid event from a journal file without opening/truncating it
    /// (read-only; used by evidence export).
    pub fn read_all(path: &Path) -> Result<Vec<JournalEvent>> {
        let (events, _hash, _len) = Self::scan(path)?;
        Ok(events)
    }

    /// Path to the journal file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Chain step: `sha256(prev || payload)`.
    fn chain(prev: &[u8; 32], payload: &[u8]) -> [u8; 32] {
        let mut buf = Vec::with_capacity(32 + payload.len());
        buf.extend_from_slice(prev);
        buf.extend_from_slice(payload);
        Digest::of(&buf).0
    }

    /// Read every valid framed record. Returns (events, last_chain_hash,
    /// valid_byte_length). Stops at the first torn/forged frame.
    fn scan(path: &Path) -> Result<(Vec<JournalEvent>, [u8; 32], u64)> {
        let mut events = Vec::new();
        let mut last_hash = [0u8; 32];
        let mut valid_len: u64 = 0;

        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((events, last_hash, 0));
            }
            Err(e) => return Err(CoreError::from(e)),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        let mut off = 0usize;
        while off + 4 <= bytes.len() {
            let len =
                u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
                    as usize;
            let frame_end = off + 4 + len + 32;
            if frame_end > bytes.len() {
                break; // torn tail
            }
            let payload = &bytes[off + 4..off + 4 + len];
            let stored_chain = &bytes[off + 4 + len..frame_end];
            let expect = Self::chain(&last_hash, payload);
            if stored_chain != expect {
                break; // forged/corrupt chain — stop here
            }
            match serde_json::from_slice::<JournalEvent>(payload) {
                Ok(ev) => events.push(ev),
                Err(_) => break, // unparseable payload — treat as torn
            }
            last_hash = expect;
            off = frame_end;
            valid_len = off as u64;
        }

        Ok((events, last_hash, valid_len))
    }
}

impl std::fmt::Debug for Journal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Journal").field("path", &self.path).finish()
    }
}
