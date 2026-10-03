//! String/byte marshalling across the C ABI. ONE allocator discipline: Rust
//! allocates every out-string, and the caller frees it with `carryon_string_free`.
//! Input `*const c_char` is borrowed for the call only.

use crate::errors::{CARRYON_ERR_BAD_UTF8, CARRYON_ERR_NULL_ARG};
use std::ffi::{c_char, CStr, CString};

/// Borrow a C string as `&str` for the duration of the call. `Err(code)` on null or
/// non-UTF-8. Internal helper; the public `extern "C"` entry points are the actual
/// `unsafe` boundary (they document the pointer contract).
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn borrow_str<'a>(ptr: *const c_char) -> Result<&'a str, i32> {
    if ptr.is_null() {
        return Err(CARRYON_ERR_NULL_ARG);
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| CARRYON_ERR_BAD_UTF8)
}

/// Heap-allocate a C string the caller must free with `carryon_string_free`.
/// Writes the pointer into `*out`. Returns `CARRYON_OK` / an error code. Internal
/// helper; the public `extern "C"` entry points are the `unsafe` boundary.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn write_out_string(out: *mut *mut c_char, s: String) -> i32 {
    if out.is_null() {
        return CARRYON_ERR_NULL_ARG;
    }
    let cleaned = s.replace('\0', " ");
    match CString::new(cleaned) {
        Ok(c) => {
            unsafe { *out = c.into_raw() };
            crate::errors::CARRYON_OK
        }
        Err(_) => CARRYON_ERR_BAD_UTF8,
    }
}

/// Free a string previously returned by any `carryon_*` function. Null-tolerant.
///
/// # Safety
/// `s` must be a pointer returned by this library (or null), freed at most once.
#[no_mangle]
pub unsafe extern "C" fn carryon_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}
