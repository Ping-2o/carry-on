//! Adapter consent (spec §10.6). Consent is specific to an adapter + target,
//! revocable, logged without storing sensitive content, and separate from
//! device pairing and authority transfer.

use crate::db::migrations::now_utc;
use crate::db::Db;
use crate::error::Result;
use carryon_adapter_api::{ConsentScope, ConsentToken};
use uuid::Uuid;

/// Issue a consent token for an adapter scope and persist a content-free record.
pub fn grant(db: &mut Db, scope: &ConsentScope) -> Result<ConsentToken> {
    let consent_id = Uuid::new_v4().to_string();
    let scope_json = serde_json::to_string(scope)?;
    db.with_tx(|tx| {
        tx.execute(
            "INSERT INTO adapter_consents (consent_id, adapter_id, scope_json, granted_utc) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![consent_id, scope.adapter_id, scope_json, now_utc()],
        )?;
        Ok(())
    })?;
    Ok(ConsentToken(consent_id))
}

/// Revoke a consent token.
pub fn revoke(db: &mut Db, token: &ConsentToken) -> Result<()> {
    db.with_tx(|tx| {
        tx.execute(
            "UPDATE adapter_consents SET revoked_utc=?2 WHERE consent_id=?1",
            rusqlite::params![token.0, now_utc()],
        )?;
        Ok(())
    })
}

/// Whether a token is currently valid (exists and not revoked) for an adapter.
pub fn is_live(db: &Db, adapter_id: &str, token: &ConsentToken) -> Result<bool> {
    let live: bool = db
        .conn()
        .query_row(
            "SELECT revoked_utc IS NULL FROM adapter_consents \
             WHERE consent_id=?1 AND adapter_id=?2",
            rusqlite::params![token.0, adapter_id],
            |r| r.get(0),
        )
        .unwrap_or(false);
    Ok(live)
}
