//! Content-addressed object store (spec §18.6/§19.1). Immutable payloads live in
//! a fan-out directory outside the SQL rows; the two-phase publish discipline
//! guarantees an object becomes visible only after its bytes are staged,
//! verified against the declared digest, and atomically renamed into place.

pub mod paths;
pub mod publish;

pub use publish::{
    publish_object, publish_object_resumable, ChunkSource, PublishOutcome, PublishProgress,
};

use crate::error::{CoreError, ObjectCode, Result};
use crate::ids::{Digest, TransferId};
use paths::{fsync_dir, StoreLayout};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;

/// A content-addressed object store rooted at a data directory.
pub struct Store {
    layout: StoreLayout,
}

/// A staging file being written before verification + publish.
pub struct StagedObject {
    pub transfer: TransferId,
    pub path: PathBuf,
    file: File,
}

impl StagedObject {
    /// Append bytes at the current position.
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.file.write_all(bytes)?;
        Ok(())
    }

    /// Flush + fsync the staging file.
    pub fn sync(&mut self) -> Result<()> {
        self.file.flush()?;
        self.file.sync_all()?;
        Ok(())
    }
}

/// A published, verified object's final location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedObject {
    pub digest: Digest,
    pub rel_path: String,
}

impl Store {
    /// Create the store under `root`, ensuring its directories exist.
    pub fn open(root: impl Into<PathBuf>) -> Result<Store> {
        let layout = StoreLayout::new(root);
        layout.ensure_dirs()?;
        Ok(Store { layout })
    }

    /// The store layout (for recovery enumeration).
    pub fn layout(&self) -> &StoreLayout {
        &self.layout
    }

    /// Open a fresh staging file for a transfer.
    pub fn stage_writer(&self, transfer: TransferId) -> Result<StagedObject> {
        let path = self.layout.staging_dir().join(format!("{transfer}.tmp"));
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;
        Ok(StagedObject {
            transfer,
            path,
            file,
        })
    }

    /// Reopen an EXISTING staging file for a suspended transfer, positioned at its
    /// end so writes append (§11.2 resume). Fails if the staging file is gone — the
    /// caller then treats the transfer as unresumable.
    pub fn reopen_stage_writer(
        &self,
        transfer: TransferId,
        staging_path: &std::path::Path,
    ) -> Result<StagedObject> {
        let mut file = OpenOptions::new()
            .write(true)
            .read(true)
            .open(staging_path)
            .map_err(|_| {
                CoreError::object(
                    ObjectCode::Missing,
                    format!("suspended staging file missing: {}", staging_path.display()),
                )
            })?;
        file.seek(SeekFrom::End(0))?;
        Ok(StagedObject {
            transfer,
            path: staging_path.to_path_buf(),
            file,
        })
    }

    /// Whether an object with this digest is already published.
    pub fn exists(&self, digest: &Digest) -> bool {
        self.layout.object_path(&digest.rel_path()).exists()
    }

    /// Open a published object for reading.
    pub fn open_object(&self, digest: &Digest) -> Result<File> {
        let p = self.layout.object_path(&digest.rel_path());
        File::open(&p).map_err(|_| {
            CoreError::object(ObjectCode::Missing, format!("object {digest} not present"))
        })
    }

    /// Verify a staged file's whole-content digest, then atomically publish it to
    /// its content-addressed path. On digest mismatch the staged bytes are moved
    /// to quarantine and `OBJECT_DigestMismatch` is returned — nothing is
    /// published (CORE-004).
    pub fn verify_and_publish(
        &self,
        mut staged: StagedObject,
        expected: Digest,
    ) -> Result<PublishedObject> {
        staged.sync()?;
        let actual = Digest::of(&std::fs::read(&staged.path)?);
        if actual != expected {
            self.quarantine(staged, "digest mismatch");
            return Err(CoreError::object(
                ObjectCode::DigestMismatch,
                format!("expected {expected}, computed {actual}"),
            ));
        }

        // Idempotent: if already present, drop the staging copy.
        if self.exists(&expected) {
            let _ = std::fs::remove_file(&staged.path);
            return Ok(PublishedObject {
                digest: expected,
                rel_path: expected.rel_path(),
            });
        }

        let rel = expected.rel_path();
        let final_path = self.layout.object_path(&rel);
        let parent = final_path.parent().expect("object path has a parent");
        std::fs::create_dir_all(parent)?;
        // Atomic publish: fsync staged, rename, fsync parent dir.
        staged.sync()?;
        std::fs::rename(&staged.path, &final_path)?;
        let _ = fsync_dir(parent);
        Ok(PublishedObject {
            digest: expected,
            rel_path: rel,
        })
    }

    /// Move a staged file to quarantine, preserving failure evidence (§7.3).
    pub fn quarantine(&self, staged: StagedObject, _reason: &str) {
        let dst = self
            .layout
            .quarantine_dir()
            .join(format!("{}.bad", staged.transfer));
        let _ = std::fs::rename(&staged.path, &dst);
    }
}
