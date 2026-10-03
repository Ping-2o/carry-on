//! Journal event records (spec §23.1). Each event is journaled before a
//! user-visible success (CORE-005).

use serde::{Deserialize, Serialize};

/// Event type. Keep names stable — they appear in evidence bundles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    SessionCreated,
    TransferBegin,
    TransferComplete,
    TransferInterrupted,
    ObjectIncompleteHidden,
    CutSealed,
    ActionExecuted,
    AuthorityOpened,
    AuthorityTransferCommitted,
    AuthorityAmbiguous,
    AdapterRegistered,
    AdapterCrashed,
    ConsentGranted,
    ConsentRevoked,
    EvidenceExported,
    RecoveryReport,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::SessionCreated => "SESSION_CREATED",
            EventType::TransferBegin => "TRANSFER_BEGIN",
            EventType::TransferComplete => "TRANSFER_COMPLETE",
            EventType::TransferInterrupted => "TRANSFER_INTERRUPTED",
            EventType::ObjectIncompleteHidden => "OBJECT_INCOMPLETE_HIDDEN",
            EventType::CutSealed => "CUT_SEALED",
            EventType::ActionExecuted => "ACTION_EXECUTED",
            EventType::AuthorityOpened => "AUTHORITY_OPENED",
            EventType::AuthorityTransferCommitted => "AUTHORITY_TRANSFER_COMMITTED",
            EventType::AuthorityAmbiguous => "AUTHORITY_AMBIGUOUS",
            EventType::AdapterRegistered => "ADAPTER_REGISTERED",
            EventType::AdapterCrashed => "ADAPTER_CRASHED",
            EventType::ConsentGranted => "CONSENT_GRANTED",
            EventType::ConsentRevoked => "CONSENT_REVOKED",
            EventType::EvidenceExported => "EVIDENCE_EXPORTED",
            EventType::RecoveryReport => "RECOVERY_REPORT",
        }
    }
}

/// One journal event. `metadata` must carry no secret payloads (§23.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEvent {
    pub event_uuid: String,
    pub monotonic_ns: String,
    pub utc: String,
    pub device: String,
    pub process: String,
    pub session_id: Option<String>,
    pub cut_number: Option<u64>,
    pub transfer_id: Option<String>,
    pub action_id: Option<String>,
    pub event_type: EventType,
    pub schema_version: u32,
    pub result_code: String,
    pub metadata: serde_json::Value,
}

impl JournalEvent {
    /// A minimal event with just type + result code.
    pub fn new(event_type: EventType, result_code: impl Into<String>) -> Self {
        use crate::db::migrations::{monotonic_ns, now_utc};
        use uuid::Uuid;
        JournalEvent {
            event_uuid: Uuid::new_v4().to_string(),
            monotonic_ns: monotonic_ns().to_string(),
            utc: now_utc(),
            device: "local".into(),
            process: "carryon-core".into(),
            session_id: None,
            cut_number: None,
            transfer_id: None,
            action_id: None,
            event_type,
            schema_version: 1,
            result_code: result_code.into(),
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_session(mut self, s: impl Into<String>) -> Self {
        self.session_id = Some(s.into());
        self
    }
    pub fn with_transfer(mut self, t: impl Into<String>) -> Self {
        self.transfer_id = Some(t.into());
        self
    }
    pub fn with_cut(mut self, n: u64) -> Self {
        self.cut_number = Some(n);
        self
    }
    pub fn with_metadata(mut self, m: serde_json::Value) -> Self {
        self.metadata = m;
        self
    }
}
