//! Kani proof harnesses for the pure protocol functions.
//!
//! Run with `cargo kani --no-default-features` (the `interleaving` feature is
//! disabled so the symbolic backend does not have to compile `stateright`).
//!
//! Claim label: every harness below is loop-free, so Kani verifies it
//! **exhaustively over the entire input space** (symbolic `u64`/`bool` values
//! are not ranges or samples — they cover all values). These are proofs of the
//! *model functions* that mirror
//! `crates/carryon-core/src/authority_xfer.rs`, not of the rusqlite
//! implementation.

#![cfg(kani)]

use crate::protocol::{
    close_epoch_to_readonly, finalize_owned_epoch, guard_mutation, guard_owned_single_writer,
    may_mutate, open_pending_epoch, recover_authority, validate_proposed_epoch,
};
use crate::state::{AuthorityMode, AuthorityState, Refusal};

/// A fully symbolic authority state (all `2 * 2 * u64` combinations).
fn any_state() -> AuthorityState {
    AuthorityState {
        epoch: kani::any(),
        mode: if kani::any::<bool>() {
            AuthorityMode::SingleWriter
        } else {
            AuthorityMode::ReadOnlyReplica
        },
        owner_local: kani::any(),
        ambiguous: kani::any(),
    }
}

/// AUTH-004: an ambiguous state never permits a mutation, no matter the mode,
/// epoch, or owner.
#[kani::proof]
fn ambiguous_state_never_permits_writes() {
    let s = any_state();
    kani::assume(s.ambiguous);
    assert!(!may_mutate(&s));
}

/// AUTH-004: `guard_mutation` refuses EVERY ambiguous state with the exact
/// engine refusal (`AuthCode::Ambiguous`), preserving the engine's precedence
/// (ambiguity is checked before mode).
#[kani::proof]
fn guard_blocks_all_ambiguous_states() {
    let s = any_state();
    kani::assume(s.ambiguous);
    assert!(matches!(guard_mutation(Some(&s)), Err(Refusal::Ambiguous)));
}

/// A session with no open epoch is refused, never silently writable.
#[kani::proof]
fn guard_no_open_epoch_refuses() {
    assert!(matches!(guard_mutation(None), Err(Refusal::NoOpenEpoch)));
}

/// AUTH-005: a non-ambiguous read-only replica is always refused and never
/// writable.
#[kani::proof]
fn guard_read_only_never_writable() {
    let s = any_state();
    kani::assume(!s.ambiguous);
    kani::assume(s.mode == AuthorityMode::ReadOnlyReplica);
    assert!(matches!(guard_mutation(Some(&s)), Err(Refusal::ReadOnly)));
    assert!(!may_mutate(&s));
}

/// The guard permits a mutation only in a live single-writer state, and the
/// guard agrees with `may_mutate` everywhere (AUTH-001 local half).
#[kani::proof]
fn guard_permits_only_live_single_writer() {
    let s = any_state();
    if guard_mutation(Some(&s)).is_ok() {
        assert!(may_mutate(&s));
        assert!(!s.ambiguous);
        assert_eq!(s.mode, AuthorityMode::SingleWriter);
    }
}

/// AUTH-002/005 + §21.3: a transfer may only be offered by a live local
/// single-writer owner; ambiguous, read-only, and remote-owner states all
/// fail closed with the engine's refusal order.
#[kani::proof]
fn only_local_single_writer_may_propose() {
    let s = any_state();
    if guard_owned_single_writer(&s).is_ok() {
        assert!(may_mutate(&s));
        assert!(s.owner_local);
        assert!(!s.ambiguous);
        assert_eq!(s.mode, AuthorityMode::SingleWriter);
    }
    kani::assume(s.ambiguous);
    assert!(matches!(
        guard_owned_single_writer(&s),
        Err(Refusal::Ambiguous)
    ));
}

/// §21.3 monotonicity: a proposal must be strictly above the current epoch;
/// the standard `current + 1` proposal always passes while equal and lower
/// numbers never do (so a lost round retried at the same number is refused,
/// including across a pending ambiguous epoch).
#[kani::proof]
fn proposal_epoch_must_be_strictly_monotonic() {
    let current: u64 = kani::any();
    assert!(!validate_proposed_epoch(current, current));
    if current > 0 {
        assert!(!validate_proposed_epoch(current - 1, current));
    }
    if let Some(next) = current.checked_add(1) {
        assert!(validate_proposed_epoch(next, current));
    }
}

/// §19.3.6 / AUTH-004: opening the pending epoch lands the destination in the
/// genuinely ambiguous window — a crash there must find writes blocked.
#[kani::proof]
fn pending_epoch_blocks_writes() {
    let mut s = any_state();
    let epoch: u64 = kani::any();
    open_pending_epoch(&mut s, epoch);
    assert!(s.ambiguous);
    assert_eq!(s.mode, AuthorityMode::SingleWriter);
    assert_eq!(s.epoch, epoch);
    assert!(!may_mutate(&s));
    assert!(matches!(guard_mutation(Some(&s)), Err(Refusal::Ambiguous)));
}

/// AUTH-003: finalization succeeds only with the full receipt set, and exactly
/// then does the window open for writes.
#[kani::proof]
fn finalize_requires_both_receipts() {
    let mut s = any_state();
    open_pending_epoch(&mut s, kani::any());
    let has_dest: bool = kani::any();
    let has_source: bool = kani::any();
    match finalize_owned_epoch(&mut s, has_dest, has_source) {
        Ok(()) => {
            assert!(has_dest && has_source);
            assert!(!s.ambiguous);
            assert!(may_mutate(&s));
        }
        Err(_) => {
            assert!(!(has_dest && has_source));
        }
    }
}

/// AUTH-005 (relinquished source): after `close_epoch_to_readonly` the source
/// is a read-only replica at the epoch the destination now owns, never
/// writable, and the epoch it held is gone (AUTH-002: it cannot be re-owned
/// silently).
#[kani::proof]
fn relinquished_source_is_never_writable() {
    let mut s = any_state();
    let owned: u64 = kani::any();
    match close_epoch_to_readonly(&mut s, owned) {
        Ok(()) => {
            assert_eq!(s.epoch, owned.wrapping_add(1));
            assert_eq!(s.mode, AuthorityMode::ReadOnlyReplica);
            assert!(!s.owner_local);
            assert!(!s.ambiguous);
            assert!(!may_mutate(&s));
            assert!(matches!(guard_mutation(Some(&s)), Err(Refusal::ReadOnly)));
        }
        Err(_) => assert_eq!(owned, u64::MAX),
    }
}

/// §21.3: recovery exists only from ambiguity, always opens a strictly fresh
/// epoch (never reuses the ambiguous one), and always yields a live writer —
/// the conflict-risk flag lives in the interleaving model.
#[kani::proof]
fn recovery_only_from_ambiguous_and_always_fresh() {
    let s = any_state();
    match recover_authority(&s) {
        None => {
            assert!(!s.ambiguous || s.epoch == u64::MAX);
        }
        Some(r) => {
            assert!(s.ambiguous);
            assert_eq!(r.epoch, s.epoch.checked_add(1).unwrap());
            assert!(r.epoch > s.epoch);
            assert_ne!(r.epoch, s.epoch);
            assert!(!r.ambiguous);
            assert_eq!(r.mode, AuthorityMode::SingleWriter);
            assert!(may_mutate(&r));
        }
    }
}
