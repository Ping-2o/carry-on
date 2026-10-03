//! Transport error taxonomy. Maps onto spec §24 `PROTO_*`/`TRANSFER_*`/`AUTH_*`
//! families; the core re-wraps these into `CoreError` at the boundary.

/// A transport-layer failure.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    /// Protocol version, framing, sequence, replay, or state-machine violation (§24 `PROTO_*`).
    #[error("PROTO_{code:?}: {msg}")]
    Proto { code: ProtoCode, msg: String },
    /// Pairing, certificate pin, or revoked-device failure (§24 `AUTH_*`). Fails closed.
    #[error("AUTH_{code:?}: {msg}")]
    Auth { code: AuthCode, msg: String },
    /// Transfer timeout, cancellation, digest mismatch, or resume failure (§24 `TRANSFER_*`).
    #[error("TRANSFER_{code:?}: {msg}")]
    Transfer { code: TransferCode, msg: String },
    /// Underlying I/O error.
    #[error("IO: {0}")]
    Io(String),
    /// Serialization of a control message failed.
    #[error("SERIALIZE: {0}")]
    Serialize(String),
}

/// Protocol-family codes (§24 `PROTO_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtoCode {
    /// Major-version mismatch (§18.2 fail closed).
    Version,
    /// Frame length out of bounds, truncated, or unparseable.
    Framing,
    /// Sequence number non-monotonic or out of window.
    Sequence,
    /// A replayed message id/sequence was observed.
    Replay,
    /// Message arrived in a state the machine does not accept it in.
    State,
}

/// Authentication-family codes (§24 `AUTH_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthCode {
    /// Peer certificate pin did not match the trust record (§18.5 fail closed).
    PinMismatch,
    /// Peer device is unknown (not paired).
    Unpaired,
    /// Peer device credential was revoked (§18.4.9).
    Revoked,
    /// TLS handshake failed.
    Handshake,
}

/// Transfer-family codes (§24 `TRANSFER_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferCode {
    /// A chunk's declared digest differs from a prior write for the same index (NET-007).
    ConflictingChunk,
    /// Whole-object digest did not match after transfer (NET-005).
    DigestMismatch,
    /// Peer reported the object/chunk is unavailable.
    Unavailable,
    /// Transfer was cancelled.
    Cancelled,
}

impl NetError {
    pub fn proto(code: ProtoCode, msg: impl Into<String>) -> Self {
        NetError::Proto {
            code,
            msg: msg.into(),
        }
    }
    pub fn auth(code: AuthCode, msg: impl Into<String>) -> Self {
        NetError::Auth {
            code,
            msg: msg.into(),
        }
    }
    pub fn transfer(code: TransferCode, msg: impl Into<String>) -> Self {
        NetError::Transfer {
            code,
            msg: msg.into(),
        }
    }

    /// Stable family string for logs/metrics (`"PROTO"`, `"AUTH"`, …).
    pub fn family(&self) -> &'static str {
        match self {
            NetError::Proto { .. } => "PROTO",
            NetError::Auth { .. } => "AUTH",
            NetError::Transfer { .. } => "TRANSFER",
            NetError::Io(_) => "IO",
            NetError::Serialize(_) => "SERIALIZE",
        }
    }
}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for NetError {
    fn from(e: serde_json::Error) -> Self {
        NetError::Serialize(e.to_string())
    }
}

/// Transport result alias.
pub type Result<T> = std::result::Result<T, NetError>;
