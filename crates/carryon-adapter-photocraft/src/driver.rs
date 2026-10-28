//! The seam to a real PhotoCraft instance (`docs/artcraft-bridge/ADAPTER-DESIGN.md`
//! §4–5).
//!
//! # Phase 1 status: UNSUPPORTED
//!
//! Only [`StubDriver`] exists and **every operation fails closed**. Phase 2
//! plugs in two real drivers behind this trait, without touching the adapter's
//! object model, hashing, or authority logic:
//!
//! - `ControlChannelDriver` (Phase 2a) — speaks the documented JSON-lines
//!   loopback control protocol of a running `photocraft --control` app
//!   (RESEARCH §2.3): `app.save {path}` for document/unsaved snapshots,
//!   `ui.inspect`/`document.inspect` for meta and workspace,
//!   `engine.execute` for step replay, `ui.set` for workspace restore.
//! - `InProcessDriver` (Phase 2b) — embeds `photocraft_engine::Session` in the
//!   Carry-On host process (RESEARCH §2.5): direct `Session::journal` slicing
//!   for journal-mode unsaved edits, `Session::execute` for replay, the
//!   `Session::authorize` hook for the fail-closed command policy.
//!
//! Both drivers are out of scope for this crate until they carry their own
//! evidence; nothing here may pretend they exist.

use crate::model::{PhotocraftMeta, Workspace};
use serde::{Deserialize, Serialize};

/// One journal/step entry: `[id, params]` — "a command id plus its parameters
/// (data, not code)" (RESEARCH §1.1, SmartFilter; §1.3, journal/actions shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub params: serde_json::Value,
}

/// Result of replaying steps (`document.edit`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditOutcome {
    /// Steps actually applied (PhotoCraft batches are not atomic and report
    /// partial application; RESEARCH §2.4).
    pub steps_ran: usize,
    pub new_revision: u64,
    /// SHA-256 hex of the dirty state after replay.
    pub unsaved_hash: String,
}

/// What the adapter needs from a live PhotoCraft. Every method takes `&mut`
/// because a driver may need to mutate its session (e.g. to open the carried
/// document) to answer.
pub trait PhotocraftDriver: Send {
    /// Single-file `.pcraft` bytes of the saved document (`app.save`/`doc.save`
    /// under the automation write root; RESEARCH §2.2).
    fn document_bytes(&mut self) -> Result<Vec<u8>, DriverError>;

    /// Bytes of the unsaved state, in the mode `meta.unsaved_mode` names:
    /// snapshot `.pcraft` of the dirty state, or journal steps from the last
    /// saved revision.
    fn unsaved_bytes(&mut self) -> Result<Vec<u8>, DriverError>;

    /// Session meta: revision, saved_revision, dirty, path, name.
    fn meta(&mut self) -> Result<PhotocraftMeta, DriverError>;

    /// `ui.inspect`-derived workspace JSON (RESEARCH §2.3).
    fn workspace_bytes(&mut self) -> Result<Vec<u8>, DriverError>;

    /// Replay `[[id, params], …]` steps against the active document
    /// (`document.edit`).
    fn apply_steps(&mut self, steps: &[Step]) -> Result<EditOutcome, DriverError>;

    /// Bring the destination session to the restored position
    /// (`session.restore`): activate the document, select layers, set the
    /// viewport. Only the live-app driver can fully do this (ADAPTER-DESIGN §4).
    fn restore(&mut self, workspace: &Workspace) -> Result<(), DriverError>;
}

/// Driver failures. `Unsupported` is the honest Phase-1 state; it never
/// masquerades as success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverError {
    /// No real driver exists yet (Phase 2, ADAPTER-DESIGN §5).
    Unsupported(String),
    /// The driver is real but the operation failed (transport, policy refusal,
    /// PhotoCraft error). Fail closed.
    Failed(String),
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverError::Unsupported(m) => {
                write!(f, "PHOTOCRAFT_DRIVER_UNSUPPORTED: {m}")
            }
            DriverError::Failed(m) => write!(f, "PHOTOCRAFT_DRIVER_FAILED: {m}"),
        }
    }
}

impl std::error::Error for DriverError {}

/// The Phase-1 stand-in: refuses everything, naming what is missing. A
/// `StubDriver` in the adapter is indistinguishable from "no PhotoCraft
/// reachable" — by design, so the fail-closed path is the tested default.
#[derive(Debug, Default, Clone, Copy)]
pub struct StubDriver;

macro_rules! stub {
    ($method:ident) => {
        fn $method(&mut self) -> Result<Vec<u8>, DriverError> {
            Err(DriverError::Unsupported(
                concat!(
                    stringify!($method),
                    ": no real PhotoCraft driver exists in Phase 1 (ADAPTER-DESIGN §5); \
                 this stub fails closed"
                )
                .into(),
            ))
        }
    };
}

impl PhotocraftDriver for StubDriver {
    stub!(document_bytes);
    stub!(unsaved_bytes);

    fn meta(&mut self) -> Result<PhotocraftMeta, DriverError> {
        Err(DriverError::Unsupported(
            "meta: no real PhotoCraft driver exists in Phase 1 (ADAPTER-DESIGN §5); \
             this stub fails closed"
                .into(),
        ))
    }

    fn workspace_bytes(&mut self) -> Result<Vec<u8>, DriverError> {
        Err(DriverError::Unsupported(
            "workspace_bytes: no real PhotoCraft driver exists in Phase 1 \
             (ADAPTER-DESIGN §5); this stub fails closed"
                .into(),
        ))
    }

    fn apply_steps(&mut self, _steps: &[Step]) -> Result<EditOutcome, DriverError> {
        Err(DriverError::Unsupported(
            "apply_steps: no real PhotoCraft driver exists in Phase 1 \
             (ADAPTER-DESIGN §5); this stub fails closed"
                .into(),
        ))
    }

    fn restore(&mut self, _workspace: &Workspace) -> Result<(), DriverError> {
        Err(DriverError::Unsupported(
            "restore: no real PhotoCraft driver exists in Phase 1 (ADAPTER-DESIGN §5); \
             this stub fails closed"
                .into(),
        ))
    }
}
