//! L4 single-writer authority transfer (spec §21.2) and the local authority-state
//! accessors that enforce split-brain prevention (§21.3, AUTH-001..005).
//!
//! # What moves, and when
//!
//! Authority transfer runs **after** the destination has already imported and
//! verified the cut (see [`crate::transfer`]); the destination therefore holds the
//! authoritative state before it is offered ownership. The ten steps of §21.2
//! collapse onto four wire messages (see [`carryon_net::wire::Message`]):
//!
//! 1. source `prepare_authority_transfer` (adapter) → `AuthorityProposal`
//! 2. destination validates the proposal locally (monotonic epoch, matching cut)
//! 3. destination durably writes its acceptance receipt and opens the proposed
//!    epoch **pending** → `AuthorityAccept { dest_receipt }`
//! 4. source durably writes the dest receipt, `commit_authority_transfer`
//!    (adapter) → its own relinquishment receipt, closes its epoch, drops to
//!    read-only → `AuthorityCommit { source_receipt }`
//! 5. destination durably writes the source receipt and finalizes ownership.
//!
//! # The invariant that survives a crash (AUTH-002/003/004)
//!
//! Receipts are durable **before** the epoch moves. Authority never moves on
//! network loss alone (AUTH-002): an `AuthorityAbort`, a dropped connection, or a
//! crash at any step before the destination has *both* receipts leaves the source
//! still owning its (never-closed) epoch — the destination's pending epoch is
//! discarded or marked ambiguous, never silently authoritative. A crash on the
//! destination *after* it opened the pending epoch but *before* the source receipt
//! landed is the one genuinely ambiguous window (§19.3.6): recovery marks that
//! epoch `ambiguous=1`, which blocks writes until manual recovery opens a fresh
//! epoch (AUTH-004).
//!
//! # Evidence honesty
//!
//! Driving both cores in one process over loopback is LOCAL evidence (§2/§30), not
//! physical cross-device evidence.

use crate::core::Core;
use crate::db::migrations::now_utc;
use crate::error::{AuthCode, CoreError, InternalCode, Result};
use crate::ids::{Epoch, SessionId};
use crate::journal::{EventType, JournalEvent};
use crate::model::{AuthorityMode, AuthorityState};
use carryon_net::wire::Message;
use carryon_net::Session;

/// The role of a persisted authority receipt (§21.2 / AUTH-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiptRole {
    Source,
    Destination,
}

impl ReceiptRole {
    fn as_str(self) -> &'static str {
        match self {
            ReceiptRole::Source => "source",
            ReceiptRole::Destination => "destination",
        }
    }
}

/// Source-side state between `propose_authority_transfer` (steps 1–7) and
/// `commit_relinquishment` (step 8). Authority has not moved while this is held.
pub struct SourcePending {
    session: SessionId,
    cut_number: u64,
    owned_epoch: Epoch,
    new_epoch: u64,
    proposal_id: String,
    dest_receipt: String,
    adapter_id: String,
}

/// Outcome of a completed authority transfer, from the perspective of either peer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthorityReceiptSet {
    pub proposal_id: String,
    pub new_epoch: u64,
    /// The destination's durable acceptance receipt (AUTH-003).
    pub dest_receipt: String,
    /// The source's durable relinquishment receipt (AUTH-003).
    pub source_receipt: String,
}

impl Core {
    // --- Local authority state (§6.7 / §21.3) ----------------------------------

    /// The current open authority state of `session` (the highest epoch whose row
    /// is not yet closed). `None` if the session has no open epoch.
    pub fn authority_state(&self, session: SessionId) -> Option<AuthorityState> {
        self.db_shared()
            .conn()
            .query_row(
                "SELECT epoch, owner_device, mode, ambiguous FROM authority_epochs \
                 WHERE session_id=?1 AND closed_utc IS NULL \
                 ORDER BY epoch DESC LIMIT 1",
                rusqlite::params![session.to_string()],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)? as u64,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)? != 0,
                    ))
                },
            )
            .ok()
            .map(|(epoch, owner_device, mode, ambiguous)| AuthorityState {
                mode: AuthorityMode::parse_tag(&mode).unwrap_or(AuthorityMode::ReadOnlyReplica),
                epoch: Epoch(epoch),
                owner_device,
                ambiguous,
            })
    }

    /// Whether this device may perform an authoritative mutation on `session`
    /// right now (AUTH-004/005). A missing or read-only or ambiguous state → `false`.
    pub fn may_mutate(&self, session: SessionId) -> bool {
        self.authority_state(session)
            .map(|s| s.may_mutate())
            .unwrap_or(false)
    }

    /// Guard an authoritative mutation. Returns the specific `AUTH_*` reason it is
    /// refused, so callers fail closed with an honest code rather than a bool.
    pub fn guard_mutation(&self, session: SessionId) -> Result<()> {
        match self.authority_state(session) {
            None => Err(CoreError::auth(
                AuthCode::WrongEpoch,
                "session has no open authority epoch",
            )),
            Some(s) if s.ambiguous => Err(CoreError::auth(
                AuthCode::Ambiguous,
                "authority is ambiguous; manual recovery required (§21.3)",
            )),
            Some(s) if s.mode == AuthorityMode::ReadOnlyReplica => Err(CoreError::auth(
                AuthCode::ReadOnly,
                "session is a read-only replica (AUTH-005)",
            )),
            Some(_) => Ok(()),
        }
    }

    // --- Source side of §21.2 --------------------------------------------------

    /// Offer single-writer authority for `cut_number` of `session` to the peer on
    /// `net`, and relinquish it on success. Runs steps 1, 4–5, 8 of §21.2.
    ///
    /// Precondition: this device currently owns a non-ambiguous single-writer epoch
    /// for `session` (else `AUTH_*`, fail closed). On success this device's epoch is
    /// closed and it becomes a read-only replica; authority has moved.
    pub fn serve_authority_transfer(
        &mut self,
        net: &mut Session,
        session: SessionId,
        cut_number: u64,
    ) -> Result<AuthorityReceiptSet> {
        let pending = self.propose_authority_transfer(net, session, cut_number)?;
        self.commit_relinquishment(net, pending)
    }

    /// Source steps 1–7: produce + send the proposal, await and durably record the
    /// destination's acceptance. Returns a [`SourcePending`] the caller feeds to
    /// [`Core::commit_relinquishment`]. Authority has **not** moved yet: if the
    /// caller never commits (crash, power loss), this device still owns its epoch
    /// (AUTH-002), and the destination's pending epoch is the ambiguous one.
    pub fn propose_authority_transfer(
        &mut self,
        net: &mut Session,
        session: SessionId,
        cut_number: u64,
    ) -> Result<SourcePending> {
        // Only a live single-writer owner may propose a transfer (AUTH-001/005).
        let state = self.guard_owned_single_writer(session)?;
        let new_epoch = state.epoch.0.checked_add(1).ok_or_else(|| {
            CoreError::internal(InternalCode::Invariant, "authority epoch overflow")
        })?;
        let proposal_id = uuid::Uuid::new_v4().to_string();

        let adapter_id = self.session_adapter_id(session)?;
        let cut_ref = carryon_adapter_api::CutRef {
            session: session.to_string(),
            cut_number,
        };
        // Step 1: adapter produces the opaque, cut-bound proposal payload.
        let proposal = self.host().call(&adapter_id, |a| {
            a.prepare_authority_transfer(&cut_ref)
                .map_err(CoreError::from)
        })?;

        net.send(Message::AuthorityProposal {
            session: session.to_string(),
            cut_number,
            new_epoch,
            proposal_id: proposal_id.clone(),
            proposal,
        })?;

        // Step 6–7: await the destination's durable acceptance.
        let dest_receipt = match net.recv()? {
            Message::AuthorityAccept {
                proposal_id: pid,
                dest_receipt,
            } if pid == proposal_id => dest_receipt,
            Message::AuthorityAbort { reason, .. } => {
                // AUTH-002: aborted or lost — we keep authority, nothing changes.
                return Err(CoreError::auth(AuthCode::WrongEpoch, reason));
            }
            other => {
                net.send(Message::AuthorityAbort {
                    proposal_id,
                    reason: format!("expected AuthorityAccept, got {other:?}"),
                })?;
                return Err(CoreError::internal(
                    InternalCode::Invariant,
                    "unexpected message during authority transfer",
                ));
            }
        };

        // Durable: record the dest receipt BEFORE committing relinquishment (AUTH-003).
        self.persist_receipt(
            &proposal_id,
            session,
            cut_number,
            new_epoch,
            ReceiptRole::Destination,
            &dest_receipt,
        )?;

        Ok(SourcePending {
            session,
            cut_number,
            owned_epoch: state.epoch,
            new_epoch,
            proposal_id,
            dest_receipt,
            adapter_id,
        })
    }

    /// Source step 8: commit relinquishment. Produces the source receipt, closes
    /// this device's epoch (→ read-only replica), and hands the destination the
    /// receipt so it can finalize. After this, authority has moved.
    pub fn commit_relinquishment(
        &mut self,
        net: &mut Session,
        pending: SourcePending,
    ) -> Result<AuthorityReceiptSet> {
        let SourcePending {
            session,
            cut_number,
            owned_epoch,
            new_epoch,
            proposal_id,
            dest_receipt,
            adapter_id,
        } = pending;

        // Step 8: adapter commits relinquishment; produce our source receipt.
        let source_receipt = self
            .host()
            .call(&adapter_id, |a| {
                a.commit_authority_transfer(&proposal_id, &dest_receipt)
                    .map_err(CoreError::from)
            })
            .inspect_err(|_| {
                // Adapter refused to relinquish: abort, keep authority (AUTH-002).
                let _ = net.send(Message::AuthorityAbort {
                    proposal_id: proposal_id.clone(),
                    reason: "source adapter refused relinquishment".into(),
                });
            })?;

        // Durable source receipt + close our epoch + drop to read-only.
        self.persist_receipt(
            &proposal_id,
            session,
            cut_number,
            new_epoch,
            ReceiptRole::Source,
            &source_receipt,
        )?;
        self.close_epoch_to_readonly(session, owned_epoch)?;

        // Step 8 wire: hand the destination our receipt so it can finalize (step 9).
        net.send(Message::AuthorityCommit {
            proposal_id: proposal_id.clone(),
            source_receipt: source_receipt.clone(),
        })?;

        self.journal().append(
            &JournalEvent::new(EventType::AuthorityTransferCommitted, "OK")
                .with_session(session.to_string())
                .with_cut(cut_number)
                .with_metadata(serde_json::json!({
                    "role": "source",
                    "proposal_id": proposal_id,
                    "new_epoch": new_epoch,
                    "relinquished": true,
                })),
        )?;

        Ok(AuthorityReceiptSet {
            proposal_id,
            new_epoch,
            dest_receipt,
            source_receipt,
        })
    }

    // --- Destination side of §21.2 ---------------------------------------------

    /// Accept an offered authority transfer onto the local mirror `session`. Runs
    /// steps 2–3, 6–7, 9 of §21.2.
    ///
    /// `session` is the destination's local (mirror) session id — the one carrying
    /// the imported cut. The proposed epoch must be strictly greater than the
    /// mirror's current epoch (monotonic, §21.3); otherwise fail closed.
    pub fn request_authority_transfer(
        &mut self,
        net: &mut Session,
        session: SessionId,
        adapter_id: &str,
    ) -> Result<AuthorityReceiptSet> {
        // Step 1 (recv): the source's proposal.
        let (proposal_id, cut_number, new_epoch, proposal) = match net.recv()? {
            Message::AuthorityProposal {
                proposal_id,
                cut_number,
                new_epoch,
                proposal,
                ..
            } => (proposal_id, cut_number, new_epoch, proposal),
            Message::AuthorityAbort { reason, .. } => {
                return Err(CoreError::auth(AuthCode::WrongEpoch, reason))
            }
            other => {
                return Err(CoreError::internal(
                    InternalCode::Invariant,
                    format!("expected AuthorityProposal, got {other:?}"),
                ))
            }
        };

        // Step 2: validate. Epoch MUST be strictly monotonic (§21.3). A replica
        // with no epoch is epoch 0; the proposed epoch must exceed what we hold.
        let current = self
            .authority_state(session)
            .map(|s| s.epoch.0)
            .unwrap_or(0);
        if new_epoch <= current {
            let reason = format!("proposed epoch {new_epoch} not above current {current}");
            net.send(Message::AuthorityAbort {
                proposal_id,
                reason: reason.clone(),
            })?;
            return Err(CoreError::auth(AuthCode::WrongEpoch, reason));
        }

        let cut_ref = carryon_adapter_api::CutRef {
            session: session.to_string(),
            cut_number,
        };
        // Step 2 (adapter): validate the proposal is coherent with imported state,
        // and produce the durable acceptance receipt (step 3).
        let dest_receipt = self
            .host()
            .call(adapter_id, |a| {
                a.accept_authority_transfer(&cut_ref, &proposal)
                    .map_err(CoreError::from)
            })
            .inspect_err(|_| {
                let _ = net.send(Message::AuthorityAbort {
                    proposal_id: proposal_id.clone(),
                    reason: "destination rejected proposal".into(),
                });
            })?;

        // Step 3 (durable): write the acceptance receipt and OPEN the proposed
        // epoch as single-writer but AMBIGUOUS — until the source receipt arrives
        // we cannot know ownership actually moved (§19.3.6 / AUTH-004). This is the
        // genuinely ambiguous window: a crash here must block writes, not enable them.
        self.persist_receipt(
            &proposal_id,
            session,
            cut_number,
            new_epoch,
            ReceiptRole::Destination,
            &dest_receipt,
        )?;
        self.open_pending_epoch(session, Epoch(new_epoch))?;

        net.send(Message::AuthorityAccept {
            proposal_id: proposal_id.clone(),
            dest_receipt: dest_receipt.clone(),
        })?;

        // Step 8 (recv): the source's relinquishment receipt completes the set.
        let source_receipt = match net.recv()? {
            Message::AuthorityCommit {
                proposal_id: pid,
                source_receipt,
            } if pid == proposal_id => source_receipt,
            Message::AuthorityAbort { reason, .. } => {
                // Source backed out after we opened the pending epoch. We never
                // finalize: discard the pending epoch, stay read-only (AUTH-002/005).
                self.discard_pending_epoch(session, Epoch(new_epoch))?;
                return Err(CoreError::auth(AuthCode::WrongEpoch, reason));
            }
            other => {
                return Err(CoreError::internal(
                    InternalCode::Invariant,
                    format!("expected AuthorityCommit, got {other:?}"),
                ))
            }
        };

        // Step 9 (durable): both receipts are in hand — finalize ownership. Clear
        // ambiguity and become the authoritative single writer.
        self.persist_receipt(
            &proposal_id,
            session,
            cut_number,
            new_epoch,
            ReceiptRole::Source,
            &source_receipt,
        )?;
        self.finalize_owned_epoch(session, Epoch(new_epoch))?;

        self.journal().append(
            &JournalEvent::new(EventType::AuthorityTransferCommitted, "OK")
                .with_session(session.to_string())
                .with_cut(cut_number)
                .with_metadata(serde_json::json!({
                    "role": "destination",
                    "proposal_id": proposal_id,
                    "new_epoch": new_epoch,
                    "now_authoritative": true,
                })),
        )?;

        Ok(AuthorityReceiptSet {
            proposal_id,
            new_epoch,
            dest_receipt,
            source_receipt,
        })
    }

    // --- Manual emergency recovery (§21.3) -------------------------------------

    /// Manually resolve an ambiguous session by opening a fresh epoch owned by this
    /// device and recording a visible conflict-risk record (§21.3). This is the
    /// ONLY path out of an ambiguous state; it never silently reuses the ambiguous
    /// epoch. Use with care: if the other device also committed, both now claim the
    /// session and the conflict-risk record flags it.
    pub fn recover_authority(&mut self, session: SessionId) -> Result<Epoch> {
        let state = self.authority_state(session).ok_or_else(|| {
            CoreError::auth(AuthCode::WrongEpoch, "no open authority epoch to recover")
        })?;
        if !state.ambiguous {
            return Err(CoreError::auth(
                AuthCode::WrongEpoch,
                "session is not ambiguous; nothing to recover",
            ));
        }
        let new_epoch = state.epoch.0.checked_add(1).ok_or_else(|| {
            CoreError::internal(InternalCode::Invariant, "authority epoch overflow")
        })?;
        self.db().with_tx(|tx| {
            // Close the ambiguous epoch and open a fresh, clean, owned one.
            tx.execute(
                "UPDATE authority_epochs SET closed_utc=?3 \
                 WHERE session_id=?1 AND epoch=?2",
                rusqlite::params![session.to_string(), state.epoch.0 as i64, now_utc()],
            )?;
            tx.execute(
                "INSERT INTO authority_epochs \
                 (session_id, epoch, owner_device, mode, opened_utc, ambiguous) \
                 VALUES (?1, ?2, 'local', 'single_writer', ?3, 0)",
                rusqlite::params![session.to_string(), new_epoch as i64, now_utc()],
            )?;
            tx.execute(
                "UPDATE sessions SET authority_epoch=?2 WHERE session_id=?1",
                rusqlite::params![session.to_string(), new_epoch as i64],
            )?;
            Ok(())
        })?;
        // A visible conflict-risk record: manual recovery can mask a real split.
        self.journal().append(
            &JournalEvent::new(EventType::AuthorityOpened, "CONFLICT_RISK")
                .with_session(session.to_string())
                .with_metadata(serde_json::json!({
                    "manual_recovery": true,
                    "closed_ambiguous_epoch": state.epoch.0,
                    "new_epoch": new_epoch,
                    "conflict_risk": "other device may also claim authority (§21.3)",
                })),
        )?;
        Ok(Epoch(new_epoch))
    }

    // --- internal helpers ------------------------------------------------------

    /// Fail closed unless this device owns a live, non-ambiguous single-writer epoch.
    fn guard_owned_single_writer(&self, session: SessionId) -> Result<AuthorityState> {
        let state = self
            .authority_state(session)
            .ok_or_else(|| CoreError::auth(AuthCode::WrongEpoch, "no open authority epoch"))?;
        if state.ambiguous {
            return Err(CoreError::auth(
                AuthCode::Ambiguous,
                "authority is ambiguous; recover before transferring",
            ));
        }
        if state.mode != AuthorityMode::SingleWriter {
            return Err(CoreError::auth(
                AuthCode::ReadOnly,
                "only a single-writer owner may transfer authority",
            ));
        }
        if state.owner_device != "local" {
            return Err(CoreError::auth(
                AuthCode::WrongEpoch,
                "this device does not own the current epoch",
            ));
        }
        Ok(state)
    }

    fn session_adapter_id(&self, session: SessionId) -> Result<String> {
        self.get_session(session)
            .map(|s| s.adapter_id)
            .ok_or_else(|| CoreError::internal(InternalCode::Invariant, "no such session"))
    }

    fn persist_receipt(
        &mut self,
        proposal_id: &str,
        session: SessionId,
        cut_number: u64,
        new_epoch: u64,
        role: ReceiptRole,
        receipt: &str,
    ) -> Result<()> {
        self.db().with_tx(|tx| {
            tx.execute(
                "INSERT OR REPLACE INTO authority_receipts \
                 (proposal_id, session_id, cut_number, new_epoch, role, receipt, created_utc) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![
                    proposal_id,
                    session.to_string(),
                    cut_number as i64,
                    new_epoch as i64,
                    role.as_str(),
                    receipt,
                    now_utc(),
                ],
            )?;
            Ok(())
        })
    }

    /// Source: close our owned epoch and record a fresh read-only replica epoch at
    /// the new number, so a subsequent read still resolves to a (non-writable) state.
    fn close_epoch_to_readonly(&mut self, session: SessionId, owned: Epoch) -> Result<()> {
        self.db().with_tx(|tx| {
            tx.execute(
                "UPDATE authority_epochs SET closed_utc=?3 \
                 WHERE session_id=?1 AND epoch=?2",
                rusqlite::params![session.to_string(), owned.0 as i64, now_utc()],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO authority_epochs \
                 (session_id, epoch, owner_device, mode, opened_utc, ambiguous) \
                 VALUES (?1, ?2, 'remote', 'read_only_replica', ?3, 0)",
                rusqlite::params![session.to_string(), (owned.0 + 1) as i64, now_utc()],
            )?;
            Ok(())
        })
    }

    /// Destination: open the proposed epoch as single-writer but ambiguous until the
    /// source receipt lands. A crash in this window keeps writes blocked (AUTH-004).
    fn open_pending_epoch(&mut self, session: SessionId, epoch: Epoch) -> Result<()> {
        self.db().with_tx(|tx| {
            tx.execute(
                "INSERT OR REPLACE INTO authority_epochs \
                 (session_id, epoch, owner_device, mode, opened_utc, ambiguous) \
                 VALUES (?1, ?2, 'local', 'single_writer', ?3, 1)",
                rusqlite::params![session.to_string(), epoch.0 as i64, now_utc()],
            )?;
            Ok(())
        })
    }

    /// Destination: both receipts present — clear ambiguity, finalize ownership.
    fn finalize_owned_epoch(&mut self, session: SessionId, epoch: Epoch) -> Result<()> {
        self.db().with_tx(|tx| {
            tx.execute(
                "UPDATE authority_epochs SET ambiguous=0 \
                 WHERE session_id=?1 AND epoch=?2",
                rusqlite::params![session.to_string(), epoch.0 as i64],
            )?;
            tx.execute(
                "UPDATE sessions SET authority_epoch=?2 WHERE session_id=?1",
                rusqlite::params![session.to_string(), epoch.0 as i64],
            )?;
            Ok(())
        })
    }

    /// Destination: source aborted after we opened the pending epoch — discard it,
    /// leaving the prior read-only state intact (authority never moved, AUTH-002).
    fn discard_pending_epoch(&mut self, session: SessionId, epoch: Epoch) -> Result<()> {
        self.db().with_tx(|tx| {
            tx.execute(
                "DELETE FROM authority_epochs WHERE session_id=?1 AND epoch=?2",
                rusqlite::params![session.to_string(), epoch.0 as i64],
            )?;
            Ok(())
        })
    }
}
