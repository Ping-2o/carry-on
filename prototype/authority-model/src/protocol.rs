//! Pure transition functions of the authority protocol (spec §21.2/§21.3).
//!
//! Every function here is a line-for-line mirror of a code path in
//! `crates/carryon-core/src/authority_xfer.rs`, reduced to the fields the
//! protocol logic actually reads (epoch, mode, owner, ambiguity, receipt
//! presence). Two intentional divergences, both toward **failing closed**:
//!
//! * `close_epoch_to_readonly` and `recover_authority` use `checked_add` and
//!   refuse on epoch overflow; the engine code panics (`Invariant` error at
//!   the proposal site) or would wrap in a release build at `u64::MAX`.
//! * the model has no I/O: durable writes are ordinary assignments to
//!   `AuthorityState` fields, and the adapter's
//!   `prepare/accept/commit_authority_transfer` calls are represented by
//!   receipt-presence booleans (adapter failures map to the abort paths, which
//!   the interleaving model exercises directly).
//!
//! Claim label: these functions are exhaustively verified by Kani
//! (`src/verification.rs`, `cargo kani --no-default-features`); they contain
//! no loops, so the verification is **complete over all inputs** — a proof of
//! the model functions, not of the rusqlite implementation.

use crate::state::{AuthorityMode, AuthorityState, Refusal, TransitionError};

/// Whether an authoritative mutation is permitted right now.
///
/// Mirror of `AuthorityState::may_mutate` (AUTH-004/005): a single writer only,
/// never while ambiguous.
pub fn may_mutate(s: &AuthorityState) -> bool {
    matches!(s.mode, AuthorityMode::SingleWriter) && !s.ambiguous
}

/// Guard an authoritative mutation. Mirror of `Core::guard_mutation` with its
/// exact refusal precedence: no open epoch, then ambiguity, then read-only.
pub fn guard_mutation(s: Option<&AuthorityState>) -> Result<(), Refusal> {
    match s {
        None => Err(Refusal::NoOpenEpoch),
        Some(s) if s.ambiguous => Err(Refusal::Ambiguous),
        Some(s) if s.mode == AuthorityMode::ReadOnlyReplica => Err(Refusal::ReadOnly),
        Some(_) => Ok(()),
    }
}

/// Only a live local single-writer owner may offer a transfer (AUTH-001/005).
///
/// Mirror of `Core::guard_owned_single_writer`: ambiguity, then mode, then
/// ownership.
pub fn guard_owned_single_writer(s: &AuthorityState) -> Result<(), Refusal> {
    if s.ambiguous {
        return Err(Refusal::Ambiguous);
    }
    if s.mode != AuthorityMode::SingleWriter {
        return Err(Refusal::ReadOnly);
    }
    if !s.owner_local {
        return Err(Refusal::NotOwner);
    }
    Ok(())
}

/// §21.3 monotonicity: a proposed epoch must be strictly above the highest
/// open epoch of the accepting side — a pending ambiguous epoch counts as
/// open, so a lost-round retry at the same number is refused.
///
/// Mirror of the `new_epoch <= current` check in
/// `Core::request_authority_transfer`.
pub fn validate_proposed_epoch(new_epoch: u64, current: u64) -> bool {
    new_epoch > current
}

/// Destination step 3: open the proposed epoch single-writer but **ambiguous**
/// — the §19.3.6 window. A crash here must block writes, never enable them
/// (AUTH-004).
///
/// Mirror of `Core::open_pending_epoch`.
pub fn open_pending_epoch(dst: &mut AuthorityState, epoch: u64) {
    dst.epoch = epoch;
    dst.mode = AuthorityMode::SingleWriter;
    dst.owner_local = true;
    dst.ambiguous = true;
}

/// Destination step 9: finalize ownership — only with the full receipt set in
/// hand (AUTH-003: durable receipts BEFORE the epoch moves).
///
/// Mirror of `Core::finalize_owned_epoch` (both receipts precede the call in
/// the engine; the model makes the requirement explicit).
pub fn finalize_owned_epoch(
    dst: &mut AuthorityState,
    has_dest_receipt: bool,
    has_source_receipt: bool,
) -> Result<(), TransitionError> {
    match (has_dest_receipt, has_source_receipt) {
        (true, true) => {
            dst.ambiguous = false;
            Ok(())
        }
        _ => Err(TransitionError::MissingReceipt),
    }
}

/// Source step 8: close the owned epoch and record a fresh read-only replica
/// epoch at `owned_epoch + 1` — the same number the destination now owns.
///
/// Mirror of `Core::close_epoch_to_readonly`. Fails closed on epoch overflow
/// (see module docs).
pub fn close_epoch_to_readonly(
    src: &mut AuthorityState,
    owned_epoch: u64,
) -> Result<(), TransitionError> {
    let next = owned_epoch
        .checked_add(1)
        .ok_or(TransitionError::EpochOverflow)?;
    src.epoch = next;
    src.mode = AuthorityMode::ReadOnlyReplica;
    src.owner_local = false;
    src.ambiguous = false;
    Ok(())
}

/// §21.3 manual recovery: the ONLY exit from ambiguity. Opens a **fresh**
/// epoch (never the ambiguous one) and, in the engine, records a visible
/// conflict-risk journal entry — modeled here by the caller keeping a
/// `recoveries` counter.
///
/// Mirror of `Core::recover_authority`.
pub fn recover_authority(s: &AuthorityState) -> Option<AuthorityState> {
    if !s.ambiguous {
        return None;
    }
    let next = s.epoch.checked_add(1)?;
    Some(AuthorityState {
        epoch: next,
        mode: AuthorityMode::SingleWriter,
        owner_local: true,
        ambiguous: false,
    })
}
