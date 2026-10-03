//! Numeric error codes for the C ABI (spec §24 requires machine-readable codes at
//! the boundary). `CoreError` carries a `family()` string and a Debug code name but
//! no integer; the mapping lives HERE so the core stays clean.
//!
//! Scheme: `i32 = family_base + ordinal`. Families are 100 apart; the ordinal is the
//! position of the inner code enum. Negative codes are FFI-local failures. `0` = OK.
//! The `code_of` match is exhaustive, so adding a `CoreError` variant forces a
//! compile error here — the table cannot silently drift.

use carryon_core::CoreError;
use std::cell::RefCell;
use std::ffi::CString;

pub const CARRYON_OK: i32 = 0;

// FFI-local failures (negative).
pub const CARRYON_ERR_PANIC: i32 = -1;
pub const CARRYON_ERR_NULL_ARG: i32 = -2;
pub const CARRYON_ERR_BAD_UTF8: i32 = -3;
pub const CARRYON_ERR_BAD_JSON: i32 = -4;
pub const CARRYON_ERR_BAD_HANDLE: i32 = -5;
pub const CARRYON_ERR_BUFFER_TOO_SMALL: i32 = -6;

// Family bases (positive).
const ADAPTER_BASE: i32 = 100;
const SCHEMA_BASE: i32 = 200;
const OBJECT_BASE: i32 = 300;
const TRANSFER_BASE: i32 = 400;
const BUDGET_BASE: i32 = 500;
const ACTION_BASE: i32 = 600;
const AUTH_BASE: i32 = 700;
const MATH_BASE: i32 = 800;
const INTERNAL_BASE: i32 = 900;
const PROTO_BASE: i32 = 1000;
const PLATFORM_BASE: i32 = 1100;

/// Map a `CoreError` to a stable i32 code. Exhaustive on every family + inner code
/// (append-only within an ABI major).
pub fn code_of(e: &CoreError) -> i32 {
    use carryon_core::error::*;
    match e {
        CoreError::Adapter { code, .. } => {
            ADAPTER_BASE
                + match code {
                    AdapterCode::Missing => 0,
                    AdapterCode::Incompatible => 1,
                    AdapterCode::Crashed => 2,
                    AdapterCode::Timeout => 3,
                    AdapterCode::Malformed => 4,
                    AdapterCode::ConsentRequired => 5,
                    AdapterCode::StaleGeneration => 6,
                }
        }
        CoreError::Schema { code, .. } => {
            SCHEMA_BASE
                + match code {
                    SchemaCode::Unsupported => 0,
                    SchemaCode::Invalid => 1,
                }
        }
        CoreError::Object { code, .. } => {
            OBJECT_BASE
                + match code {
                    ObjectCode::Missing => 0,
                    ObjectCode::Corrupt => 1,
                    ObjectCode::Stale => 2,
                    ObjectCode::Oversized => 3,
                    ObjectCode::Invalid => 4,
                    ObjectCode::DigestMismatch => 5,
                    ObjectCode::IncompleteStaged => 6,
                    ObjectCode::SecretExcluded => 7,
                }
        }
        CoreError::Transfer { code, .. } => {
            TRANSFER_BASE
                + match code {
                    TransferCode::Timeout => 0,
                    TransferCode::Cancelled => 1,
                    TransferCode::DigestMismatch => 2,
                    TransferCode::ConflictingChunk => 3,
                    TransferCode::Quota => 4,
                    TransferCode::ResumeFailure => 5,
                }
        }
        CoreError::Budget { code, .. } => {
            BUDGET_BASE
                + match code {
                    BudgetCode::Network => 0,
                    BudgetCode::Cpu => 1,
                    BudgetCode::Memory => 2,
                    BudgetCode::Storage => 3,
                    BudgetCode::Time => 4,
                }
        }
        CoreError::Action { code, .. } => {
            ACTION_BASE
                + match code {
                    ActionCode::Unsupported => 0,
                    ActionCode::DependencyFailure => 1,
                    ActionCode::ExecutionFailure => 2,
                    ActionCode::OracleFailure => 3,
                }
        }
        CoreError::Auth { code, .. } => {
            AUTH_BASE
                + match code {
                    AuthCode::Permission => 0,
                    AuthCode::WrongEpoch => 1,
                    AuthCode::Ambiguous => 2,
                    AuthCode::ReadOnly => 3,
                }
        }
        CoreError::Math(_) => MATH_BASE,
        CoreError::Internal { code, .. } => {
            INTERNAL_BASE
                + match code {
                    InternalCode::Invariant => 0,
                    InternalCode::DbCorrupt => 1,
                    InternalCode::Io => 2,
                    InternalCode::Serialization => 3,
                    InternalCode::Idempotency => 4,
                }
        }
        CoreError::Proto { code, .. } => {
            PROTO_BASE
                + match code {
                    ProtoCode::Version => 0,
                    ProtoCode::Framing => 1,
                    ProtoCode::Sequence => 2,
                    ProtoCode::Replay => 3,
                    ProtoCode::State => 4,
                }
        }
        CoreError::Platform { code, .. } => {
            PLATFORM_BASE
                + match code {
                    PlatformCode::Permission => 0,
                    PlatformCode::Activation => 1,
                    PlatformCode::Background => 2,
                    PlatformCode::SecureStorage => 3,
                    PlatformCode::Package => 4,
                }
        }
    }
}

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Store a last-error message for retrieval via `carryon_last_error`.
pub fn set_last_error(msg: impl Into<String>) {
    let cleaned = msg.into().replace('\0', " ");
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(cleaned).ok();
    });
}

/// Record a `CoreError` as the last error and return its code.
pub fn record(e: &CoreError) -> i32 {
    set_last_error(e.user_message());
    code_of(e)
}

/// Copy the last-error string into a caller buffer. Returns `CARRYON_OK` and writes
/// the length (including NUL) into `*len`; if the buffer is too small, returns
/// `CARRYON_ERR_BUFFER_TOO_SMALL` and sets `*len` to the required length.
/// Internal helper; `carryon_last_error` is the `unsafe` boundary.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn copy_last_error(buf: *mut u8, len: *mut usize) -> i32 {
    if len.is_null() {
        return CARRYON_ERR_NULL_ARG;
    }
    LAST_ERROR.with(|e| {
        let borrow = e.borrow();
        let bytes = borrow
            .as_ref()
            .map(|c| c.as_bytes_with_nul())
            .unwrap_or(b"\0");
        let required = bytes.len();
        let cap = unsafe { *len };
        unsafe { *len = required };
        if buf.is_null() || cap < required {
            return CARRYON_ERR_BUFFER_TOO_SMALL;
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, required) };
        CARRYON_OK
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use carryon_core::error::{BudgetCode, TransferCode};

    #[test]
    fn stable_codes() {
        assert_eq!(
            code_of(&CoreError::budget(BudgetCode::Network, "x")),
            BUDGET_BASE
        );
        assert_eq!(
            code_of(&CoreError::transfer(TransferCode::ResumeFailure, "x")),
            TRANSFER_BASE + 5
        );
    }
}
