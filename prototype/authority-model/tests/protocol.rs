//! Deterministic unit tests of the pure protocol functions — small scenarios
//! mirroring the engine integration tests (`authority_handoff.rs`,
//! fault-injection classes).

use carryon_authority_model::protocol as p;
use carryon_authority_model::{AuthorityMode, AuthorityState, Refusal, TransitionError};

fn live_owner(epoch: u64) -> AuthorityState {
    AuthorityState {
        epoch,
        mode: AuthorityMode::SingleWriter,
        owner_local: true,
        ambiguous: false,
    }
}

fn mirror() -> AuthorityState {
    AuthorityState {
        epoch: 0,
        mode: AuthorityMode::ReadOnlyReplica,
        owner_local: false,
        ambiguous: false,
    }
}

#[test]
fn guard_refusal_precedence_matches_engine() {
    // None -> WrongEpoch("no open authority epoch").
    assert_eq!(p::guard_mutation(None), Err(Refusal::NoOpenEpoch));
    // Ambiguity is checked BEFORE mode (authority_xfer.rs order).
    let mut s = mirror();
    s.ambiguous = true;
    assert_eq!(p::guard_mutation(Some(&s)), Err(Refusal::Ambiguous));
    // Read-only is refused (AUTH-005).
    assert_eq!(p::guard_mutation(Some(&mirror())), Err(Refusal::ReadOnly));
    // A live single writer passes.
    assert_eq!(p::guard_mutation(Some(&live_owner(1))), Ok(()));
}

#[test]
fn may_mutate_requires_single_writer_and_no_ambiguity() {
    assert!(p::may_mutate(&live_owner(3)));
    let mut s = live_owner(3);
    s.ambiguous = true; // §19.3.6 window blocks writes (AUTH-004).
    assert!(!p::may_mutate(&s));
    assert!(!p::may_mutate(&mirror()));
}

#[test]
fn only_local_single_writer_may_propose() {
    let mut remote = live_owner(2);
    remote.owner_local = false;
    assert_eq!(
        p::guard_owned_single_writer(&remote),
        Err(Refusal::NotOwner)
    );
    assert_eq!(
        p::guard_owned_single_writer(&mirror()),
        Err(Refusal::ReadOnly)
    );
    let mut amb = live_owner(2);
    amb.ambiguous = true;
    assert_eq!(p::guard_owned_single_writer(&amb), Err(Refusal::Ambiguous));
    assert_eq!(p::guard_owned_single_writer(&live_owner(2)), Ok(()));
}

#[test]
fn proposed_epoch_must_be_strictly_monotonic() {
    // §21.3: equal and lower are refused — so a retried lost round at the same
    // number fails closed, including across a pending ambiguous epoch.
    assert!(p::validate_proposed_epoch(2, 1));
    assert!(!p::validate_proposed_epoch(2, 2));
    assert!(!p::validate_proposed_epoch(1, 2));
}

#[test]
fn pending_epoch_is_the_ambiguous_window() {
    // Destination step 3: the proposed epoch opens single-writer but
    // ambiguous; a crash here must find writes blocked (AUTH-004).
    let mut dst = mirror();
    p::open_pending_epoch(&mut dst, 2);
    assert_eq!(dst.epoch, 2);
    assert_eq!(dst.mode, AuthorityMode::SingleWriter);
    assert!(dst.ambiguous);
    assert!(!p::may_mutate(&dst));
    assert_eq!(p::guard_mutation(Some(&dst)), Err(Refusal::Ambiguous));
}

#[test]
fn finalize_requires_both_receipts_then_unblocks() {
    // AUTH-003: no finalization without the full receipt set.
    let mut dst = mirror();
    p::open_pending_epoch(&mut dst, 2);
    assert_eq!(
        p::finalize_owned_epoch(&mut dst, true, false),
        Err(TransitionError::MissingReceipt)
    );
    assert_eq!(
        p::finalize_owned_epoch(&mut dst, false, true),
        Err(TransitionError::MissingReceipt)
    );
    assert!(dst.ambiguous && !p::may_mutate(&dst));
    assert_eq!(p::finalize_owned_epoch(&mut dst, true, true), Ok(()));
    assert!(!dst.ambiguous && p::may_mutate(&dst));
}

#[test]
fn relinquished_source_becomes_read_only_at_destination_epoch() {
    // Source step 8: close e -> read-only replica at e+1, the number the
    // destination now owns; the source can never write again silently
    // (AUTH-005).
    let mut src = live_owner(1);
    assert_eq!(p::close_epoch_to_readonly(&mut src, 1), Ok(()));
    assert_eq!(src.epoch, 2);
    assert_eq!(src.mode, AuthorityMode::ReadOnlyReplica);
    assert!(!src.owner_local);
    assert!(!p::may_mutate(&src));
    assert_eq!(p::guard_mutation(Some(&src)), Err(Refusal::ReadOnly));
    // Epoch overflow fails closed (documented divergence).
    let mut hot = live_owner(u64::MAX);
    assert_eq!(
        p::close_epoch_to_readonly(&mut hot, u64::MAX),
        Err(TransitionError::EpochOverflow)
    );
}

#[test]
fn recovery_only_from_ambiguity_and_always_fresh_epoch() {
    // §21.3: recovery is the ONLY exit; it never reuses the ambiguous epoch.
    assert_eq!(p::recover_authority(&mirror()), None);
    let mut dst = mirror();
    p::open_pending_epoch(&mut dst, 2);
    let r = p::recover_authority(&dst).expect("ambiguous recovers");
    assert_eq!(r.epoch, 3);
    assert_ne!(r.epoch, dst.epoch);
    assert!(!r.ambiguous);
    assert!(p::may_mutate(&r));
    // Overflow fails closed.
    let mut hot = mirror();
    p::open_pending_epoch(&mut hot, u64::MAX);
    assert_eq!(p::recover_authority(&hot), None);
}
