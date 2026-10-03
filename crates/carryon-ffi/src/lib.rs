//! # Carry-On C ABI (`carryon-ffi`)
//!
//! A versioned, **panic-safe** C ABI that drives the full Carry-On import→carry→
//! relaunch lifecycle (spec §5.12/§8.1). Any system shell — a desktop binary now, an
//! iOS Swift or Android Kotlin shell later — links this and calls it; no shell is
//! compiled or "supported" here (§9/§53/PLAT-001). The green gate is a Rust-as-C
//! caller test.
//!
//! ## Safety contract
//! - Every entry point is wrapped in `catch_unwind` (`ffi_guard!`): a Rust panic
//!   becomes `CARRYON_ERR_PANIC`, never an unwind across the boundary (§8.1).
//! - Handles are opaque `*mut` over boxed Rust values; the caller frees each with the
//!   matching `*_free`. Out-strings are Rust-allocated, freed by `carryon_string_free`
//!   (one allocator discipline). Input strings/bytes are borrowed for the call only.
//! - serde types (manifests, budgets, results, tokens) cross as JSON strings.
//!
//! ## Honest boundary
//! The adapter factory ([`factory`]) maps a fixed id + JSON params to a compiled-in
//! adapter — C passes data, never behavior (§3.8 no remote code execution). No
//! foreign-binary execution, JIT, or arbitrary plugins exist on this surface.

pub mod abi_core;
pub mod abi_net;
pub mod abi_transfer;
pub mod errors;
pub mod factory;
pub mod handle;
pub mod strings;

/// C ABI version. Major changes break the header; minor are additive.
pub const CARRYON_ABI_MAJOR: u32 = 1;
pub const CARRYON_ABI_MINOR: u32 = 0;

/// Write the ABI version into the out-params. Lets a shell check compatibility at
/// load time against the header's `#define`s.
///
/// # Safety
/// `major`/`minor` must be valid writable pointers or null (null is ignored).
#[no_mangle]
pub unsafe extern "C" fn carryon_abi_version(major: *mut u32, minor: *mut u32) {
    if !major.is_null() {
        *major = CARRYON_ABI_MAJOR;
    }
    if !minor.is_null() {
        *minor = CARRYON_ABI_MINOR;
    }
}

/// Copy the last-error string into a caller-owned buffer (see [`errors::copy_last_error`]).
///
/// # Safety
/// `buf` points to at least `*len` bytes (or is null to query the length); `len` is a
/// valid pointer.
#[no_mangle]
pub unsafe extern "C" fn carryon_last_error(buf: *mut u8, len: *mut usize) -> i32 {
    ffi_guard!(errors::CARRYON_ERR_PANIC, {
        errors::copy_last_error(buf, len)
    })
}

/// Run an entry-point body under `catch_unwind`. A caught panic stores a last-error
/// and returns `$default`. Keeps panics from unwinding across the C boundary (§8.1).
#[macro_export]
macro_rules! ffi_guard {
    ($default:expr, $body:block) => {{
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $body)) {
            Ok(v) => v,
            Err(_) => {
                $crate::errors::set_last_error("panic caught at FFI boundary");
                $default
            }
        }
    }};
}

/// Test-only entry point that deliberately panics, to prove `ffi_guard!` returns an
/// error instead of aborting (§8.1). Compiled only under the `test-panic` feature.
#[cfg(feature = "test-panic")]
#[no_mangle]
pub extern "C" fn carryon_debug_panic() -> i32 {
    ffi_guard!(errors::CARRYON_ERR_PANIC, {
        panic!("deliberate test panic");
    })
}
