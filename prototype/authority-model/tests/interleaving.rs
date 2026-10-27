//! Exhaustive stateright check of the §21.2 authority-transfer interleavings.
//!
//! Part of the green gate: `always` properties must have no counterexample and
//! every `sometimes` witness must be reachable, across ALL interleavings of
//! the protocol steps with crashes, message loss, and duplication, within
//! `AuthorityTransferModel::within_boundary`.

#![cfg(feature = "interleaving")]

use carryon_authority_model::AuthorityTransferModel;
use stateright::{Checker, Model};

#[test]
fn authority_interleavings_are_safe() {
    let checker = AuthorityTransferModel::new().checker().spawn_bfs().join();
    checker.assert_properties();
}
