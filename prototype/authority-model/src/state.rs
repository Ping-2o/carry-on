//! Mirror types of the engine's authority state (spec §6.7/§21.3).
//!
//! These are **mirrors**, kept deliberately dependency-free so the model can be
//! compiled by Kani's symbolic backend (`crates/carryon-core` pulls in
//! `rusqlite`, TLS and the adapter host — none of which can or should be
//! symbolically executed). Each type cites the engine original it mirrors; the
//! mapping is one-to-one on the fields that the protocol logic reads.

use std::fmt;

/// Mirror of `carryon_core::model::AuthorityMode` (spec §6.7).
///
/// `ExplicitMerge` (L5) is out of scope, as in the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthorityMode {
    /// `read_only_replica` DB tag.
    ReadOnlyReplica,
    /// `single_writer` DB tag.
    SingleWriter,
}

/// Mirror of `carryon_core::model::AuthorityState`: the highest open
/// `authority_epochs` row of one session, as returned by
/// `Core::authority_state` (`crates/carryon-core/src/authority_xfer.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuthorityState {
    /// Highest open epoch. `0` means no open epoch (the engine's `None`).
    pub epoch: u64,
    pub mode: AuthorityMode,
    /// `owner_device == "local"` in the engine's row.
    pub owner_local: bool,
    /// Set when an interrupted commit left ownership unknown (§19.3.6,
    /// AUTH-004).
    pub ambiguous: bool,
}

/// Why an authoritative mutation or transfer proposal is refused. Mirrors the
/// `AuthCode` family of `CoreError::auth` (§24), one variant per refusal site
/// in `crates/carryon-core/src/authority_xfer.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `AuthCode::WrongEpoch` — "session has no open authority epoch".
    NoOpenEpoch,
    /// `AuthCode::Ambiguous` — writes blocked until recovery (§21.3).
    Ambiguous,
    /// `AuthCode::ReadOnly` — read-only replica (AUTH-005).
    ReadOnly,
    /// `AuthCode::WrongEpoch` — "this device does not own the current epoch".
    NotOwner,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Refusal::NoOpenEpoch => "no open authority epoch",
            Refusal::Ambiguous => "authority is ambiguous (AUTH-004)",
            Refusal::ReadOnly => "read-only replica (AUTH-005)",
            Refusal::NotOwner => "device does not own the current epoch",
        };
        f.write_str(s)
    }
}

/// Failure modes of the two epoch-moving transitions (both fail closed; the
/// Kani harnesses prove these are the only failure paths).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// AUTH-003: finalization without the full receipt set.
    MissingReceipt,
    /// Epoch counter overflow (`u64::MAX`); the model refuses where the
    /// engine would panic (see `protocol` module docs).
    EpochOverflow,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TransitionError::MissingReceipt => "receipt set incomplete (AUTH-003)",
            TransitionError::EpochOverflow => "epoch counter overflow",
        };
        f.write_str(s)
    }
}
