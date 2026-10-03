//! Authority modes and epoch bookkeeping (spec §6.7/§21.3).
//!
//! Phase 1 implements `ReadOnlyReplica` and `SingleWriter` only. The split-brain
//! rule (§21.3) is enforced locally: an ambiguous interruption blocks new
//! authoritative writes until a new epoch is opened; network loss alone never
//! implies relinquishment (there is no network in Phase 1, so "ambiguous" arises
//! only from an interrupted local commit).

use crate::ids::Epoch;
use serde::{Deserialize, Serialize};

/// Authority mode (spec §6.7). `ExplicitMerge` (L5) is out of Phase 1 scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorityMode {
    ReadOnlyReplica,
    SingleWriter,
}

impl AuthorityMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthorityMode::ReadOnlyReplica => "read_only_replica",
            AuthorityMode::SingleWriter => "single_writer",
        }
    }

    /// Parse from the stored DB tag. Named `parse_tag` to avoid clashing with the
    /// `FromStr` trait convention.
    pub fn parse_tag(s: &str) -> Option<Self> {
        match s {
            "read_only_replica" => Some(AuthorityMode::ReadOnlyReplica),
            "single_writer" => Some(AuthorityMode::SingleWriter),
            _ => None,
        }
    }
}

/// Current authority state of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityState {
    pub mode: AuthorityMode,
    pub epoch: Epoch,
    pub owner_device: String,
    /// Set when an interrupted commit left ownership unknown. Blocks writes
    /// until manual recovery opens a new epoch (§19.3.6, AUTH-004).
    pub ambiguous: bool,
}

impl AuthorityState {
    /// Whether an authoritative mutation is permitted right now.
    ///
    /// A read-only replica never permits mutation (AUTH-005); an ambiguous
    /// single-writer session blocks until recovery (AUTH-004).
    pub fn may_mutate(&self) -> bool {
        matches!(self.mode, AuthorityMode::SingleWriter) && !self.ambiguous
    }
}
