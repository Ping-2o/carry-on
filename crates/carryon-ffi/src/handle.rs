//! Opaque handle types for the C ABI. Each is a thin typed pointer over a boxed
//! Rust value. C sees only `*mut CarryonX`; it can never read the inner layout.
//!
//! Lifetime rule: a constructor returns `*mut T` (heap); the caller MUST pass it to
//! the matching `*_free`. Inputs are borrowed for the call only.

use carryon_core::carryon_net::{DeviceIdentity, Session, TrustStore};
use carryon_core::Core;
use std::net::TcpListener;

/// Boxed `Core` behind an opaque pointer.
pub type CarryonCore = Core;
/// Boxed `DeviceIdentity`.
pub type CarryonIdentity = DeviceIdentity;
/// Boxed `TrustStore`.
pub type CarryonTrust = TrustStore;
/// Boxed transport `Session`.
pub type CarryonSession = Session;
/// Boxed `TcpListener` (the FFI binds it; `accept` consumes connections from it).
pub type CarryonListener = TcpListener;

/// Box a value and leak it to a raw pointer for C ownership.
pub fn to_handle<T>(value: T) -> *mut T {
    Box::into_raw(Box::new(value))
}

/// Borrow a handle as `&mut`. Returns `None` on null.
///
/// # Safety
/// `ptr` must be a live handle from this library, not aliased elsewhere for the
/// duration of the borrow.
pub unsafe fn as_mut<'a, T>(ptr: *mut T) -> Option<&'a mut T> {
    ptr.as_mut()
}

/// Borrow a handle as `&`. Returns `None` on null.
///
/// # Safety
/// `ptr` must be a live handle from this library.
pub unsafe fn as_ref<'a, T>(ptr: *const T) -> Option<&'a T> {
    ptr.as_ref()
}

/// Free a boxed handle. Null-tolerant.
///
/// # Safety
/// `ptr` must be a handle returned by this library (or null), freed at most once.
pub unsafe fn free<T>(ptr: *mut T) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}
