//! Authority invariants (§6.7/§21.3): read-only never writable (AUTH-005),
//! ambiguous interruption blocks writes (AUTH-004), epochs monotonic (AUTH-001).

use carryon_core::ids::Epoch;
use carryon_core::model::{AuthorityMode, AuthorityState};

#[test]
fn read_only_never_mutates() {
    let st = AuthorityState {
        mode: AuthorityMode::ReadOnlyReplica,
        epoch: Epoch(0),
        owner_device: "local".into(),
        ambiguous: false,
    };
    assert!(
        !st.may_mutate(),
        "AUTH-005: read-only replica must never permit mutation"
    );
}

#[test]
fn single_writer_may_mutate_unless_ambiguous() {
    let ok = AuthorityState {
        mode: AuthorityMode::SingleWriter,
        epoch: Epoch(1),
        owner_device: "local".into(),
        ambiguous: false,
    };
    assert!(ok.may_mutate());

    let ambiguous = AuthorityState {
        ambiguous: true,
        ..ok.clone()
    };
    assert!(
        !ambiguous.may_mutate(),
        "AUTH-004: ambiguous interruption must block writes until recovery"
    );
}

#[test]
fn epochs_are_ordered() {
    // AUTH-001: epochs are monotonic and comparable.
    assert!(Epoch(0) < Epoch(1));
    assert!(Epoch(5) > Epoch(4));
}
