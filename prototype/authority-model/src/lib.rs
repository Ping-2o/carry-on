//! Machine-checked protocol model of the L4 single-writer authority transfer
//! (spec §21.2/§21.3, AUTH-001..005).
//!
//! # What this is — and what it is NOT
//!
//! This crate verifies the **protocol**, in two complementary ways:
//!
//! * [`verification`] — Kani proof harnesses over the pure transition
//!   functions in [`protocol`], which mirror
//!   `crates/carryon-core/src/authority_xfer.rs` function by function. The
//!   harnesses are loop-free, so Kani's verification is exhaustive over the
//!   entire input space: a proof of the model functions.
//! * [`machine`] — a stateright model of the two-device interleaving,
//!   including the crash windows of §19.3.6, message loss and duplication.
//!   It runs as an ordinary test in the green gate and explores every
//!   reachable state within its boundary.
//!
//! Claim labels: this is **MODEL-level evidence**. It is not a proof of the
//! `rusqlite` implementation, of the TLS transport, or of the adapter host —
//! those are covered by the engine's integration tests, fault injection, and
//! physical-device evidence (see `docs/TARGETS.md`). The mirrors are kept in
//! the same order and with the same refusal precedence as the engine code;
//! the documented divergences (all toward failing closed) are listed in
//! [`protocol`].
//!
//! # Running
//!
//! ```sh
//! cargo test -p carryon-authority-model            # stateright interleavings + unit tests
//! cargo kani -p carryon-authority-model --no-default-features   # Kani proofs
//! ```
//!
//! Recommendation provenance: the choice of Verus/Kani for checking this
//! protocol follows R. van Renesse's suggestion (outreach dossier, 5 Oct
//! 2026); he has not reviewed or endorsed this work.

pub mod machine;
pub mod protocol;
pub mod state;
#[cfg(kani)]
mod verification;

#[cfg(feature = "interleaving")]
pub use machine::{Action, AuthorityTransferModel, Device, DstStage, Msg, SrcStage, TransferState};
pub use protocol::{
    close_epoch_to_readonly, finalize_owned_epoch, guard_mutation, guard_owned_single_writer,
    may_mutate, open_pending_epoch, recover_authority, validate_proposed_epoch,
};
pub use state::{AuthorityMode, AuthorityState, Refusal, TransitionError};
