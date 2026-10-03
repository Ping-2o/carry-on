//! Logical session (spec §6.1) and its lifecycle state (spec §18.8 local subset).

use crate::ids::{Digest, Epoch, SessionId};
use crate::model::object::Sensitivity;
use serde::{Deserialize, Serialize};

/// Session lifecycle state. The UI MUST distinguish exact readiness (§16.2);
/// "Ready" alone is forbidden. Phase 1 uses this local subset of §18.8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Idle,
    Preparing,
    CutProposed,
    CutSealed,
    Importing,
    ActionReady,
    ReadOnlyActive,
    /// A terminal failure with evidence retained.
    Failed,
    /// Interrupted; outcome unknown. NEVER relabeled success (§19.3.9).
    Inconclusive,
    Aborted,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Idle => "Idle",
            SessionState::Preparing => "Preparing",
            SessionState::CutProposed => "CutProposed",
            SessionState::CutSealed => "CutSealed",
            SessionState::Importing => "Importing",
            SessionState::ActionReady => "ActionReady",
            SessionState::ReadOnlyActive => "ReadOnlyActive",
            SessionState::Failed => "Failed",
            SessionState::Inconclusive => "Inconclusive",
            SessionState::Aborted => "Aborted",
        }
    }

    /// Parse from the stored DB tag. Named `parse_tag` (not `from_str`) to avoid
    /// clashing with the `FromStr` trait convention.
    pub fn parse_tag(s: &str) -> Option<Self> {
        Some(match s {
            "Idle" => SessionState::Idle,
            "Preparing" => SessionState::Preparing,
            "CutProposed" => SessionState::CutProposed,
            "CutSealed" => SessionState::CutSealed,
            "Importing" => SessionState::Importing,
            "ActionReady" => SessionState::ActionReady,
            "ReadOnlyActive" => SessionState::ReadOnlyActive,
            "Failed" => SessionState::Failed,
            "Inconclusive" => SessionState::Inconclusive,
            "Aborted" => SessionState::Aborted,
            _ => return None,
        })
    }
}

/// One coherent body of user work (spec §6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub adapter_id: String,
    pub adapter_version: String,
    pub schema_version: u32,
    pub title: String,
    pub creation_device: String,
    pub created_utc: String,
    pub authority_epoch: Epoch,
    pub latest_cut: Option<u64>,
    pub privacy: Sensitivity,
    pub devices: Vec<String>,
    pub manifest_root: Option<Digest>,
    pub state: SessionState,
    /// Monotonic generation; bumped on every committed authoritative change.
    pub generation: u64,
}
