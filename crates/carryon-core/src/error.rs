//! Core error taxonomy (spec §24). Every public failure is one stable
//! machine-readable family; the `Display` form prints `FAMILY_Code: message`.
//!
//! Phase-1-active families: `ADAPTER_/SCHEMA_/OBJECT_/TRANSFER_/BUDGET_/ACTION_/
//! AUTH_/MATH_/INTERNAL_`. `PROTO_`/`PLATFORM_` exist so the taxonomy is stable
//! but are mostly a Phase-2 surface.

use carryon_adapter_api::AdapterError;
use carryon_math_core::MathError;

/// Adapter-family codes (§24 `ADAPTER_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterCode {
    Missing,
    Incompatible,
    Crashed,
    Timeout,
    Malformed,
    ConsentRequired,
    StaleGeneration,
}

/// Schema-family codes (§24 `SCHEMA_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaCode {
    Unsupported,
    Invalid,
}

/// Object-family codes (§24 `OBJECT_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectCode {
    Missing,
    Corrupt,
    Stale,
    Oversized,
    Invalid,
    DigestMismatch,
    IncompleteStaged,
    SecretExcluded,
}

/// Transfer-family codes (§24 `TRANSFER_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferCode {
    Timeout,
    Cancelled,
    DigestMismatch,
    ConflictingChunk,
    Quota,
    ResumeFailure,
}

/// Budget-family codes (§24 `BUDGET_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetCode {
    Network,
    Cpu,
    Memory,
    Storage,
    Time,
}

/// Action-family codes (§24 `ACTION_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionCode {
    Unsupported,
    DependencyFailure,
    ExecutionFailure,
    OracleFailure,
}

/// Authority-family codes (§24 `AUTH_*`, local authority/epoch in Phase 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthCode {
    Permission,
    WrongEpoch,
    Ambiguous,
    ReadOnly,
}

/// Internal-family codes (§24 `INTERNAL_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalCode {
    Invariant,
    DbCorrupt,
    Io,
    Serialization,
    Idempotency,
}

/// Protocol-family codes (§24 `PROTO_*`, mostly Phase 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtoCode {
    Version,
    Framing,
    Sequence,
    Replay,
    State,
}

/// Platform-family codes (§24 `PLATFORM_*`, mostly Phase 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformCode {
    Permission,
    Activation,
    Background,
    SecureStorage,
    Package,
}

/// The core error type. Each variant maps to a spec §24 family.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CoreError {
    #[error("ADAPTER_{code:?}: {msg}")]
    Adapter { code: AdapterCode, msg: String },
    #[error("SCHEMA_{code:?}: {msg}")]
    Schema { code: SchemaCode, msg: String },
    #[error("OBJECT_{code:?}: {msg}")]
    Object { code: ObjectCode, msg: String },
    #[error("TRANSFER_{code:?}: {msg}")]
    Transfer { code: TransferCode, msg: String },
    #[error("BUDGET_{code:?}: {msg}")]
    Budget { code: BudgetCode, msg: String },
    #[error("ACTION_{code:?}: {msg}")]
    Action { code: ActionCode, msg: String },
    #[error("AUTH_{code:?}: {msg}")]
    Auth { code: AuthCode, msg: String },
    #[error("MATH_{0}")]
    Math(#[from] MathError),
    #[error("INTERNAL_{code:?}: {msg}")]
    Internal { code: InternalCode, msg: String },
    #[error("PROTO_{code:?}: {msg}")]
    Proto { code: ProtoCode, msg: String },
    #[error("PLATFORM_{code:?}: {msg}")]
    Platform { code: PlatformCode, msg: String },
}

impl CoreError {
    /// The stable family string (`"ADAPTER"`, `"OBJECT"`, …) for logs/metrics.
    pub fn family(&self) -> &'static str {
        match self {
            CoreError::Adapter { .. } => "ADAPTER",
            CoreError::Schema { .. } => "SCHEMA",
            CoreError::Object { .. } => "OBJECT",
            CoreError::Transfer { .. } => "TRANSFER",
            CoreError::Budget { .. } => "BUDGET",
            CoreError::Action { .. } => "ACTION",
            CoreError::Auth { .. } => "AUTH",
            CoreError::Math(_) => "MATH",
            CoreError::Internal { .. } => "INTERNAL",
            CoreError::Proto { .. } => "PROTO",
            CoreError::Platform { .. } => "PLATFORM",
        }
    }

    /// A safe, user-facing message (no internal debug chain).
    pub fn user_message(&self) -> String {
        self.to_string()
    }

    // --- constructors keep call sites terse and consistent ---

    pub fn object(code: ObjectCode, msg: impl Into<String>) -> Self {
        CoreError::Object {
            code,
            msg: msg.into(),
        }
    }
    pub fn transfer(code: TransferCode, msg: impl Into<String>) -> Self {
        CoreError::Transfer {
            code,
            msg: msg.into(),
        }
    }
    pub fn schema(code: SchemaCode, msg: impl Into<String>) -> Self {
        CoreError::Schema {
            code,
            msg: msg.into(),
        }
    }
    pub fn adapter(code: AdapterCode, msg: impl Into<String>) -> Self {
        CoreError::Adapter {
            code,
            msg: msg.into(),
        }
    }
    pub fn action(code: ActionCode, msg: impl Into<String>) -> Self {
        CoreError::Action {
            code,
            msg: msg.into(),
        }
    }
    pub fn auth(code: AuthCode, msg: impl Into<String>) -> Self {
        CoreError::Auth {
            code,
            msg: msg.into(),
        }
    }
    pub fn internal(code: InternalCode, msg: impl Into<String>) -> Self {
        CoreError::Internal {
            code,
            msg: msg.into(),
        }
    }
    pub fn budget(code: BudgetCode, msg: impl Into<String>) -> Self {
        CoreError::Budget {
            code,
            msg: msg.into(),
        }
    }
}

/// Map an adapter-reported error into the core taxonomy (§24).
impl From<AdapterError> for CoreError {
    fn from(e: AdapterError) -> Self {
        match e {
            AdapterError::Incompatible(m) => CoreError::adapter(AdapterCode::Incompatible, m),
            AdapterError::ActionUnsupported(m) => CoreError::action(ActionCode::Unsupported, m),
            AdapterError::ConsentRequired(m) => CoreError::adapter(AdapterCode::ConsentRequired, m),
            AdapterError::StaleGeneration { expected, actual } => CoreError::adapter(
                AdapterCode::StaleGeneration,
                format!("expected generation {expected}, adapter at {actual}"),
            ),
            AdapterError::UnknownObject(m) => CoreError::object(ObjectCode::Missing, m),
            AdapterError::OutOfRange(m) => CoreError::object(ObjectCode::Invalid, m),
            AdapterError::Internal(m) => CoreError::adapter(AdapterCode::Malformed, m),
        }
    }
}

/// rusqlite errors become `INTERNAL_Io`/`Serialization` unless a caller maps
/// them more specifically.
impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        CoreError::internal(InternalCode::Io, format!("sqlite: {e}"))
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::internal(InternalCode::Io, e.to_string())
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::internal(InternalCode::Serialization, e.to_string())
    }
}

/// Map a transport error (§24 `PROTO_*`/`AUTH_*`/`TRANSFER_*`) into the core
/// taxonomy, preserving the family so logs/metrics stay stable across the boundary.
impl From<carryon_net::NetError> for CoreError {
    fn from(e: carryon_net::NetError) -> Self {
        use carryon_net::NetError as N;
        match e {
            N::Proto { code, msg } => CoreError::Proto {
                code: match code {
                    carryon_net::ProtoCode::Version => ProtoCode::Version,
                    carryon_net::ProtoCode::Framing => ProtoCode::Framing,
                    carryon_net::ProtoCode::Sequence => ProtoCode::Sequence,
                    carryon_net::ProtoCode::Replay => ProtoCode::Replay,
                    carryon_net::ProtoCode::State => ProtoCode::State,
                },
                msg,
            },
            N::Auth { code, msg } => CoreError::auth(
                match code {
                    carryon_net::AuthCode::PinMismatch
                    | carryon_net::AuthCode::Unpaired
                    | carryon_net::AuthCode::Revoked => AuthCode::Permission,
                    carryon_net::AuthCode::Handshake => AuthCode::Permission,
                },
                msg,
            ),
            N::Transfer { code, msg } => CoreError::transfer(
                match code {
                    carryon_net::TransferCode::ConflictingChunk => TransferCode::ConflictingChunk,
                    carryon_net::TransferCode::DigestMismatch => TransferCode::DigestMismatch,
                    carryon_net::TransferCode::Unavailable => TransferCode::ResumeFailure,
                    carryon_net::TransferCode::Cancelled => TransferCode::Cancelled,
                },
                msg,
            ),
            N::Io(m) => CoreError::internal(InternalCode::Io, m),
            N::Serialize(m) => CoreError::internal(InternalCode::Serialization, m),
        }
    }
}

/// Crate result alias.
pub type Result<T> = std::result::Result<T, CoreError>;
