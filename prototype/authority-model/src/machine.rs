//! stateright model of the §21.2 authority transfer: exhaustive interleavings
//! of two devices under crashes, message loss, and duplication.
//!
//! This is the systems half of the model-level evidence for the L4
//! single-writer protocol. `stateright` explores **every reachable state** of
//! the interleaving within [`Model::within_boundary`] and reports a
//! counterexample for any violated `always` property and a witness for every
//! `sometimes` property. It runs as an ordinary `cargo test`
//! (`tests/interleaving.rs`), so it is part of the green gate.
//!
//! # Faithfulness and documented divergences
//!
//! * One transfer direction (source owns the epoch; destination is the
//!   imported mirror), up to two proposal rounds and one manual recovery —
//!   the boundary in `within_boundary`.
//! * The adapter's `accept/commit` calls always succeed; adapter refusals
//!   are covered by the `Abort`/loss actions, which exercise the same
//!   authority-unchanged outcome (AUTH-002).
//! * Stale messages after a crash cannot be consumed by the public API in the
//!   engine (`serve/request_authority_transfer` expect the protocol's next
//!   message and error out on anything else, leaving authority unchanged);
//!   the model represents that by having no consuming action — the message
//!   just sits in flight. Same safety outcome, smaller state space.
//! * The volatile protocol positions (`SourcePending` on the source, the
//!   in-flight `request_authority_transfer` on the destination) are modeled by
//!   the stage enums; a crash action resets a stage to `Idle` while all
//!   durable state (epoch rows, receipts) persists — exactly the engine's
//!   transactional durability.
//!
//! Claim label: MODEL-level evidence about the protocol, not a proof of the
//! rusqlite/TLS implementation.

#![cfg(feature = "interleaving")]

use crate::protocol;
use crate::state::{AuthorityMode, AuthorityState};
use stateright::{Model, Property};

/// A device's durable authority state plus its persisted receipts.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Device {
    /// Mirror of the highest open `authority_epochs` row.
    pub state: AuthorityState,
    /// The destination acceptance receipt is durably recorded (AUTH-003).
    pub has_dest_receipt: bool,
    /// The source relinquishment receipt is durably recorded (AUTH-003).
    pub has_source_receipt: bool,
}

/// Volatile source position: `Some(SourcePending)` between
/// `propose_authority_transfer` and the commit steps.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum SrcStage {
    Idle,
    Proposed,
}

/// Volatile destination position: inside `request_authority_transfer`
/// awaiting the source's commit/abort (step 8).
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum DstStage {
    Idle,
    Pending,
}

/// Wire messages of §21.2 (receipts are opaque strings in the engine; only
/// presence matters, and `Abort` reasons do not affect authority).
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Msg {
    /// `AuthorityProposal` at `new_epoch`.
    Proposal(u64),
    /// `AuthorityAccept { dest_receipt }`.
    Accept,
    /// `AuthorityCommit { source_receipt }`.
    Commit,
    /// `AuthorityAbort`.
    Abort,
}

/// The full interleaving state.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TransferState {
    pub src: Device,
    pub dst: Device,
    pub src_stage: SrcStage,
    pub dst_stage: DstStage,
    /// The pending epoch row exists durably (opened by step 3, deleted on
    /// discard, cleared on recovery).
    pub dst_pending_open: bool,
    /// The epoch the pending epoch row replaced (for discard).
    pub dst_prev_epoch: u64,
    /// Epoch of the current proposal round (0 = none started).
    pub proposal_epoch: u64,
    /// In-flight messages (a multiset; index-addressed for determinism).
    pub net: Vec<Msg>,
    /// Manual recoveries performed (§21.3 conflict-risk flag).
    pub recoveries: u8,
    /// Proposals started (boundary for re-proposals).
    pub rounds: u8,
}

/// Every action the environment can take. Mirrors the engine's step sequence
/// plus the fault actions the engine can suffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Source step 1: propose (requires a live local single-writer epoch).
    SrcPropose,
    /// Source: consume `Accept`, durably record the dest receipt.
    SrcRecvAccept(usize),
    /// Source: consume `Abort` — keep authority (AUTH-002).
    SrcRecvAbort(usize),
    /// Source: give up on an answer — keep authority (AUTH-002).
    SrcGiveUp,
    /// Source step 8a: durably record the source receipt.
    SrcPersistSourceReceipt,
    /// Source step 8b: close the owned epoch to read-only.
    SrcCloseEpoch,
    /// Source step 8c: send `Commit` and finish.
    SrcSendCommit,
    /// Destination: consume `Proposal`, validate epoch (§21.3), durably record
    /// the dest receipt, or answer `Abort` (fail closed).
    DstRecvProposal(usize),
    /// Destination step 3: open the pending ambiguous epoch, send `Accept`.
    DstOpenPending,
    /// Destination: consume `Commit`, durably record the source receipt.
    DstRecvCommit(usize),
    /// Destination: consume `Abort` after opening the pending epoch — discard
    /// it and stay read-only (AUTH-002/005).
    DstRecvAbort(usize),
    /// Destination step 9: finalize ownership with both receipts (AUTH-003).
    DstFinalize,
    /// Source crash: lose the volatile `SourcePending`, keep durable state.
    CrashSrc,
    /// Destination crash: lose the volatile protocol position, keep durable
    /// state (a pending epoch row SURVIVES and stays ambiguous — AUTH-004).
    CrashDst,
    /// §21.3 manual recovery: the only exit from ambiguity (flagged).
    RecoverDst,
    /// Network: drop an in-flight message.
    NetLoss(usize),
    /// Network: duplicate an in-flight message.
    NetDup(usize),
}

/// The stateright model.
#[derive(Clone, Debug, Default)]
pub struct AuthorityTransferModel;

impl AuthorityTransferModel {
    pub fn new() -> Self {
        Self
    }

    /// Initial state: the source owns epoch 1 as a live single writer
    /// (post-`create_session`), the destination is the imported read-only
    /// mirror with no open epoch.
    fn initial() -> TransferState {
        TransferState {
            src: Device {
                state: AuthorityState {
                    epoch: 1,
                    mode: AuthorityMode::SingleWriter,
                    owner_local: true,
                    ambiguous: false,
                },
                has_dest_receipt: false,
                has_source_receipt: false,
            },
            dst: Device {
                state: AuthorityState {
                    epoch: 0,
                    mode: AuthorityMode::ReadOnlyReplica,
                    owner_local: false,
                    ambiguous: false,
                },
                has_dest_receipt: false,
                has_source_receipt: false,
            },
            src_stage: SrcStage::Idle,
            dst_stage: DstStage::Idle,
            dst_pending_open: false,
            dst_prev_epoch: 0,
            proposal_epoch: 0,
            net: Vec::new(),
            recoveries: 0,
            rounds: 0,
        }
    }
}

impl Model for AuthorityTransferModel {
    type State = TransferState;
    type Action = Action;

    fn init_states(&self) -> Vec<Self::State> {
        vec![Self::initial()]
    }

    fn actions(&self, s: &Self::State, actions: &mut Vec<Self::Action>) {
        // --- source -----------------------------------------------------------------
        if s.src_stage == SrcStage::Idle
            && protocol::guard_owned_single_writer(&s.src.state).is_ok()
        {
            actions.push(Action::SrcPropose);
        }
        if s.src_stage == SrcStage::Proposed {
            for i in 0..s.net.len() {
                match s.net[i] {
                    Msg::Accept => actions.push(Action::SrcRecvAccept(i)),
                    Msg::Abort => actions.push(Action::SrcRecvAbort(i)),
                    _ => {}
                }
            }
            actions.push(Action::SrcGiveUp);
            if s.src.has_dest_receipt {
                actions.push(Action::SrcPersistSourceReceipt);
            }
            if s.src.has_source_receipt {
                actions.push(Action::SrcCloseEpoch);
            }
            if s.src.has_source_receipt && s.src.state.mode == AuthorityMode::ReadOnlyReplica {
                actions.push(Action::SrcSendCommit);
            }
        }
        actions.push(Action::CrashSrc);

        // --- destination ------------------------------------------------------------
        if s.dst_stage == DstStage::Idle {
            for i in 0..s.net.len() {
                if let Msg::Proposal(_) = s.net[i] {
                    actions.push(Action::DstRecvProposal(i));
                }
            }
        }
        if s.dst_stage == DstStage::Pending {
            if s.dst.has_dest_receipt && !s.dst_pending_open {
                actions.push(Action::DstOpenPending);
            }
            for i in 0..s.net.len() {
                match s.net[i] {
                    Msg::Commit => actions.push(Action::DstRecvCommit(i)),
                    Msg::Abort if s.dst_pending_open => actions.push(Action::DstRecvAbort(i)),
                    _ => {}
                }
            }
            if s.dst_pending_open && s.dst.has_dest_receipt && s.dst.has_source_receipt {
                actions.push(Action::DstFinalize);
            }
        }
        actions.push(Action::CrashDst);
        if s.dst.state.ambiguous {
            actions.push(Action::RecoverDst);
        }

        // --- network ----------------------------------------------------------------
        for i in 0..s.net.len() {
            actions.push(Action::NetLoss(i));
            actions.push(Action::NetDup(i));
        }
    }

    fn next_state(&self, s: &Self::State, a: Self::Action) -> Option<Self::State> {
        let mut n = s.clone();
        match a {
            Action::SrcPropose => {
                let e = n.src.state.epoch.checked_add(1)?;
                n.proposal_epoch = e;
                n.src_stage = SrcStage::Proposed;
                n.net.push(Msg::Proposal(e));
                n.rounds += 1;
            }
            Action::SrcRecvAccept(i) => {
                if n.src_stage != SrcStage::Proposed || n.net.get(i) != Some(&Msg::Accept) {
                    return None;
                }
                n.net.remove(i);
                n.src.has_dest_receipt = true;
            }
            Action::SrcRecvAbort(i) => {
                if n.src_stage != SrcStage::Proposed || n.net.get(i) != Some(&Msg::Abort) {
                    return None;
                }
                n.net.remove(i);
                n.src_stage = SrcStage::Idle;
            }
            Action::SrcGiveUp => {
                if n.src_stage != SrcStage::Proposed {
                    return None;
                }
                n.src_stage = SrcStage::Idle;
            }
            Action::SrcPersistSourceReceipt => {
                if n.src_stage != SrcStage::Proposed || !n.src.has_dest_receipt {
                    return None;
                }
                n.src.has_source_receipt = true;
            }
            Action::SrcCloseEpoch => {
                if n.src_stage != SrcStage::Proposed || !n.src.has_source_receipt {
                    return None;
                }
                let owned = n.src.state.epoch;
                protocol::close_epoch_to_readonly(&mut n.src.state, owned).ok()?;
            }
            Action::SrcSendCommit => {
                if n.src_stage != SrcStage::Proposed
                    || !n.src.has_source_receipt
                    || n.src.state.mode != AuthorityMode::ReadOnlyReplica
                {
                    return None;
                }
                n.net.push(Msg::Commit);
                n.src_stage = SrcStage::Idle;
            }
            Action::DstRecvProposal(i) => {
                if n.dst_stage != DstStage::Idle {
                    return None;
                }
                let e = match n.net.get(i)? {
                    Msg::Proposal(e) => *e,
                    _ => return None,
                };
                n.net.remove(i);
                if protocol::validate_proposed_epoch(e, n.dst.state.epoch) {
                    // Step 3 (durable): the dest acceptance receipt.
                    n.dst.has_dest_receipt = true;
                    n.proposal_epoch = e;
                    n.dst_stage = DstStage::Pending;
                } else {
                    // §21.3 monotonicity refuses; fail closed, nothing durable.
                    n.net.push(Msg::Abort);
                }
            }
            Action::DstOpenPending => {
                if n.dst_stage != DstStage::Pending || !n.dst.has_dest_receipt || n.dst_pending_open
                {
                    return None;
                }
                n.dst_prev_epoch = n.dst.state.epoch;
                protocol::open_pending_epoch(&mut n.dst.state, n.proposal_epoch);
                n.dst_pending_open = true;
                n.net.push(Msg::Accept);
            }
            Action::DstRecvCommit(i) => {
                if n.dst_stage != DstStage::Pending || n.net.get(i) != Some(&Msg::Commit) {
                    return None;
                }
                n.net.remove(i);
                n.dst.has_source_receipt = true;
            }
            Action::DstRecvAbort(i) => {
                if n.dst_stage != DstStage::Pending
                    || !n.dst_pending_open
                    || n.net.get(i) != Some(&Msg::Abort)
                {
                    return None;
                }
                n.net.remove(i);
                // discard_pending_epoch: delete the pending row, prior state back.
                n.dst.state.epoch = n.dst_prev_epoch;
                n.dst.state.mode = AuthorityMode::ReadOnlyReplica;
                n.dst.state.ambiguous = false;
                n.dst_pending_open = false;
                n.dst_stage = DstStage::Idle;
            }
            Action::DstFinalize => {
                if n.dst_stage != DstStage::Pending
                    || !n.dst_pending_open
                    || !n.dst.has_dest_receipt
                    || !n.dst.has_source_receipt
                {
                    return None;
                }
                protocol::finalize_owned_epoch(
                    &mut n.dst.state,
                    n.dst.has_dest_receipt,
                    n.dst.has_source_receipt,
                )
                .ok()?;
                n.dst_stage = DstStage::Idle;
                n.dst_pending_open = false;
            }
            Action::CrashSrc => {
                n.src_stage = SrcStage::Idle;
            }
            Action::CrashDst => {
                n.dst_stage = DstStage::Idle;
            }
            Action::RecoverDst => {
                let recovered = protocol::recover_authority(&n.dst.state)?;
                n.dst.state = recovered;
                n.dst_pending_open = false;
                n.recoveries += 1;
            }
            Action::NetLoss(i) => {
                if i >= n.net.len() {
                    return None;
                }
                n.net.remove(i);
            }
            Action::NetDup(i) => {
                let m = n.net.get(i)?.clone();
                n.net.push(m);
            }
        }
        Some(n)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            // P1: two live writers can only exist after a FLAGGED manual
            // recovery (§21.3 conflict risk). Without recovery, never.
            Property::always(
                "single_writer_or_flagged_recovery",
                |_, s: &TransferState| {
                    !(protocol::may_mutate(&s.src.state) && protocol::may_mutate(&s.dst.state))
                        || s.recoveries > 0
                },
            ),
            // P2 (AUTH-004): no ambiguous state ever permits a write.
            Property::always("ambiguous_blocks_writes", |_, s: &TransferState| {
                (!protocol::may_mutate(&s.src.state) || !s.src.state.ambiguous)
                    && (!protocol::may_mutate(&s.dst.state) || !s.dst.state.ambiguous)
            }),
            // P3: authority on the destination implies the source relinquished
            // (it closed its epoch to read-only) — again excepting flagged
            // recovery.
            Property::always("relinquished_before_handover", |_, s: &TransferState| {
                !protocol::may_mutate(&s.dst.state)
                    || s.src.state.mode != AuthorityMode::SingleWriter
                    || s.recoveries > 0
            }),
            // P4 (AUTH-003): the destination writes with the full durable
            // receipt set in hand — excepting flagged manual recovery, which
            // by design (§21.3) opens a live writer from ambiguity alone and
            // records the conflict risk.
            Property::always(
                "destination_writes_need_both_receipts",
                |_, s: &TransferState| {
                    !protocol::may_mutate(&s.dst.state)
                        || (s.dst.has_dest_receipt && s.dst.has_source_receipt)
                        || s.recoveries > 0
                },
            ),
            // P5 (AUTH-002 witness): a round that never completed leaves the
            // source owning and the destination read-only.
            Property::sometimes("loss_keeps_source_authority", |_, s: &TransferState| {
                s.rounds > 0
                    && s.src_stage == SrcStage::Idle
                    && protocol::may_mutate(&s.src.state)
                    && s.dst.state.mode == AuthorityMode::ReadOnlyReplica
                    && s.recoveries == 0
            }),
            // P6 (§19.3.6 witness): the ambiguous window is reachable.
            Property::sometimes("ambiguous_window_reachable", |_, s: &TransferState| {
                s.dst.state.ambiguous
            }),
            // P7 (happy path witness): the transfer completes with the
            // destination the only writer and the source read-only.
            Property::sometimes("handoff_completes", |_, s: &TransferState| {
                protocol::may_mutate(&s.dst.state)
                    && s.src.state.mode == AuthorityMode::ReadOnlyReplica
                    && s.recoveries == 0
            }),
        ]
    }

    fn within_boundary(&self, s: &Self::State) -> bool {
        s.src.state.epoch <= 4
            && s.dst.state.epoch <= 4
            && s.net.len() <= 3
            && s.rounds <= 2
            && s.recoveries <= 1
    }
}
