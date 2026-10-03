//! Object-store directory layout (spec §19.1).
//!
//! ```text
//! <root>/objects/sha256/ab/cd/<full-digest>   content-addressed payloads
//! <root>/staging/<transfer>.tmp               in-progress bytes
//! <root>/quarantine/<transfer>.bad            failed/suspect bytes (evidence)
//! ```

use std::path::{Path, PathBuf};

/// Known subdirectories under the data root.
pub struct StoreLayout {
    pub root: PathBuf,
}

impl StoreLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        StoreLayout { root: root.into() }
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.root.join("objects")
    }
    pub fn staging_dir(&self) -> PathBuf {
        self.root.join("staging")
    }
    pub fn quarantine_dir(&self) -> PathBuf {
        self.root.join("quarantine")
    }

    /// Absolute path for a content digest's fan-out location.
    pub fn object_path(&self, rel_path: &str) -> PathBuf {
        self.root.join(rel_path)
    }

    /// Create the base directories if absent.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.objects_dir())?;
        std::fs::create_dir_all(self.staging_dir())?;
        std::fs::create_dir_all(self.quarantine_dir())?;
        Ok(())
    }
}

/// Fsync a directory so a rename into it is durable.
pub fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    let f = std::fs::File::open(dir)?;
    f.sync_all()
}
