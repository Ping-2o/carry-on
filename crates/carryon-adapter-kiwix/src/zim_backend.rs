//! Safe wrapper over the czim C ABI (the C++ shim over libzim).
//!
//! Fail-closed: every fallible operation returns `Err(ZimBackendError)` —
//! never fabricated data. This module exists only with the `zim-backend`
//! feature; without it, `KiwixAdapter::from_zim` fails closed.
//!
//! Labels: IMPLEMENTED on the macOS host (build + tests against real ZIMs,
//! `tests/zim_backend.rs`); cross-target build status per
//! `docs/kiwix-bridge/BUILD-EVIDENCE.md`.

use std::ffi::{c_char, c_uint, CStr, CString};
use std::fmt;
use std::os::raw::{c_int, c_longlong, c_uchar, c_ulonglong};

#[repr(C)]
struct CzimArchiveOpaque {
    _private: [u8; 0],
}
#[repr(C)]
struct CzimItemOpaque {
    _private: [u8; 0],
}

extern "C" {
    fn czim_open(utf8_path: *const c_char) -> *mut CzimArchiveOpaque;
    fn czim_open_noxapian(utf8_path: *const c_char) -> *mut CzimArchiveOpaque;
    fn czim_close(arch: *mut CzimArchiveOpaque);
    fn czim_uuid(arch: *mut CzimArchiveOpaque, buf: *mut c_char, buf_len: c_uint) -> c_int;
    fn czim_title(arch: *mut CzimArchiveOpaque, buf: *mut c_char, buf_len: c_uint) -> c_int;
    fn czim_filesize(arch: *mut CzimArchiveOpaque) -> c_ulonglong;
    fn czim_entry_count(arch: *mut CzimArchiveOpaque) -> c_ulonglong;
    fn czim_main_path(arch: *mut CzimArchiveOpaque, buf: *mut c_char, buf_len: c_uint) -> c_int;
    fn czim_item_open(
        arch: *mut CzimArchiveOpaque,
        utf8_path: *const c_char,
    ) -> *mut CzimItemOpaque;
    fn czim_item_close(item: *mut CzimItemOpaque);
    fn czim_item_size(item: *mut CzimItemOpaque) -> c_ulonglong;
    fn czim_item_mimetype(item: *mut CzimItemOpaque, buf: *mut c_char, buf_len: c_uint) -> c_int;
    fn czim_item_path(item: *mut CzimItemOpaque, buf: *mut c_char, buf_len: c_uint) -> c_int;
    fn czim_item_title(item: *mut CzimItemOpaque, buf: *mut c_char, buf_len: c_uint) -> c_int;
    fn czim_item_read(
        item: *mut CzimItemOpaque,
        offset: c_ulonglong,
        len: c_ulonglong,
        buf: *mut c_uchar,
    ) -> c_longlong;
    fn czim_item_direct_access(
        item: *mut CzimItemOpaque,
        path_buf: *mut c_char,
        path_len: c_uint,
        offset_out: *mut c_ulonglong,
    ) -> c_int;
}

/// Errors from the libzim backend. All fail closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZimBackendError {
    /// The path was not valid UTF-8 / contained a NUL.
    BadPath(String),
    /// Opening the archive failed (missing file, bad format, ...).
    OpenFailed(String),
    /// The entry was not found in the archive.
    EntryNotFound(String),
    /// A string/buffer query failed on the C side.
    QueryFailed(&'static str),
    /// A read failed on the C side.
    ReadFailed,
}

impl fmt::Display for ZimBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZimBackendError::BadPath(p) => write!(f, "bad path (not utf8/nul-free): {p}"),
            ZimBackendError::OpenFailed(p) => write!(f, "zim open failed: {p}"),
            ZimBackendError::EntryNotFound(p) => write!(f, "zim entry not found: {p}"),
            ZimBackendError::QueryFailed(what) => write!(f, "zim query failed: {what}"),
            ZimBackendError::ReadFailed => write!(f, "zim read failed"),
        }
    }
}

impl std::error::Error for ZimBackendError {}

/// Read one of the C string functions that return the needed length.
/// Returns <0 => error; else the string.
fn call_str(
    what: &'static str,
    f: impl Fn(*mut c_char, c_uint) -> c_int,
) -> Result<String, ZimBackendError> {
    let mut buf = [0u8; 128];
    let need = f(buf.as_mut_ptr() as *mut c_char, buf.len() as c_uint);
    if need < 0 {
        return Err(ZimBackendError::QueryFailed(what));
    }
    if (need as usize) <= buf.len() {
        // SAFETY: the C side NUL-terminates within buf_len.
        let cstr = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        return Ok(cstr.to_string_lossy().into_owned());
    }
    // Re-call with the needed length.
    let mut big = vec![0u8; need as usize + 1];
    let need2 = f(big.as_mut_ptr() as *mut c_char, big.len() as c_uint);
    if need2 < 0 {
        return Err(ZimBackendError::QueryFailed(what));
    }
    // SAFETY: the C side NUL-terminates within buf_len.
    let cstr = unsafe { CStr::from_ptr(big.as_ptr() as *const c_char) };
    Ok(cstr.to_string_lossy().into_owned())
}

/// An opened ZIM archive. Cheap handle; the underlying archive is closed on
/// Drop (items keep it alive via shared_ptr on the C side).
pub struct ZimArchive {
    raw: *mut CzimArchiveOpaque,
}

// SAFETY: libzim's Archive methods are threadsafe (include/zim/archive.h
// "all methods of `Archive` are threadsafe") and our handle is read-only
// after open; the raw pointer is never freed twice (Drop + Send-only move).
unsafe impl Send for ZimArchive {}
unsafe impl Sync for ZimArchive {}

impl Drop for ZimArchive {
    fn drop(&mut self) {
        // SAFETY: we own the archive returned by czim_open.
        if !self.raw.is_null() {
            unsafe { czim_close(self.raw) };
        }
    }
}

/// Identity info of an archive (no bytes of the content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZimInfo {
    pub uuid: String,
    pub title: Option<String>,
    pub filesize: u64,
    pub entry_count: u64,
    pub main_path: Option<String>,
}

/// One resolved item in an archive.
pub struct ZimItem {
    raw: *mut CzimItemOpaque,
}

unsafe impl Send for ZimItem {}

impl Drop for ZimItem {
    fn drop(&mut self) {
        // SAFETY: we own the item returned by czim_item_open.
        if !self.raw.is_null() {
            unsafe { czim_item_close(self.raw) };
        }
    }
}

impl ZimItem {
    pub fn size(&self) -> u64 {
        // SAFETY: self.raw is a live item handle.
        unsafe { czim_item_size(self.raw) }
    }

    pub fn mimetype(&self) -> Result<String, ZimBackendError> {
        // SAFETY: self.raw is a live item handle.
        call_str("mimetype", |buf, len| unsafe {
            czim_item_mimetype(self.raw, buf, len)
        })
    }

    pub fn path(&self) -> Result<String, ZimBackendError> {
        // SAFETY: self.raw is a live item handle.
        call_str("path", |buf, len| unsafe {
            czim_item_path(self.raw, buf, len)
        })
    }

    pub fn title(&self) -> Result<String, ZimBackendError> {
        // SAFETY: self.raw is a live item handle.
        call_str("title", |buf, len| unsafe {
            czim_item_title(self.raw, buf, len)
        })
    }

    /// Read `len` bytes at `offset` of the item's decompressed data.
    /// Returns 0 bytes at/after end. This is the chunked-transfer primitive
    /// (`zim::Item::getData(offset, size)`).
    pub fn read_range(&self, offset: u64, len: u64) -> Result<Vec<u8>, ZimBackendError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let mut buf = vec![0u8; len as usize];
        // SAFETY: self.raw is a live item handle; buf is valid for `len`.
        let got = unsafe { czim_item_read(self.raw, offset, len, buf.as_mut_ptr()) };
        if got < 0 {
            return Err(ZimBackendError::ReadFailed);
        }
        buf.truncate(got as usize);
        Ok(buf)
    }

    /// Direct access info (raw bytes inside the zim file; uncompressed
    /// items only). Fail closed: None when unavailable.
    pub fn direct_access(&self) -> Result<Option<(String, u64)>, ZimBackendError> {
        let mut path_buf = [0u8; 4096];
        let mut offset: c_ulonglong = 0;
        // SAFETY: self.raw live; buffers valid.
        let r = unsafe {
            czim_item_direct_access(
                self.raw,
                path_buf.as_mut_ptr() as *mut c_char,
                path_buf.len() as c_uint,
                &mut offset,
            )
        };
        match r {
            1 => {
                // SAFETY: NUL-terminated within path_buf.
                let cstr = unsafe { CStr::from_ptr(path_buf.as_ptr() as *const c_char) };
                Ok(Some((cstr.to_string_lossy().into_owned(), offset)))
            }
            0 => Ok(None),
            _ => Err(ZimBackendError::QueryFailed("direct_access")),
        }
    }
}

impl ZimArchive {
    /// Open a ZIM file. Fail closed on any error.
    pub fn open(path: &str) -> Result<ZimArchive, ZimBackendError> {
        Self::open_inner(path, false)
    }

    /// Open WITHOUT eager Xapian preload — for partially-materialized
    /// archives: reading works; full-text search loads lazily (and fails
    /// closed while the file is sparse). Fail closed on any error.
    pub fn open_progressive(path: &str) -> Result<ZimArchive, ZimBackendError> {
        Self::open_inner(path, true)
    }

    fn open_inner(path: &str, no_xapian: bool) -> Result<ZimArchive, ZimBackendError> {
        let cpath = CString::new(path).map_err(|_| ZimBackendError::BadPath(path.to_string()))?;
        // SAFETY: cpath is a valid NUL-terminated string for the duration.
        let raw = if no_xapian {
            unsafe { czim_open_noxapian(cpath.as_ptr()) }
        } else {
            unsafe { czim_open(cpath.as_ptr()) }
        };
        if raw.is_null() {
            return Err(ZimBackendError::OpenFailed(path.to_string()));
        }
        Ok(ZimArchive { raw })
    }

    /// Identity info (uuid, title, size, counts, main path).
    pub fn info(&self) -> Result<ZimInfo, ZimBackendError> {
        // SAFETY: self.raw is a live archive handle.
        let uuid = call_str("uuid", |buf, len| unsafe { czim_uuid(self.raw, buf, len) })?;
        let title = call_str("title", |buf, len| unsafe {
            czim_title(self.raw, buf, len)
        })
        .ok();
        let filesize = unsafe { czim_filesize(self.raw) };
        let entry_count = unsafe { czim_entry_count(self.raw) };
        let main_path = call_str("main_path", |buf, len| unsafe {
            czim_main_path(self.raw, buf, len)
        })
        .ok();
        Ok(ZimInfo {
            uuid,
            title,
            filesize,
            entry_count,
            main_path,
        })
    }

    /// Resolve an entry by path (redirects followed). Fail closed if absent.
    pub fn item(&self, path: &str) -> Result<ZimItem, ZimBackendError> {
        let cpath = CString::new(path).map_err(|_| ZimBackendError::BadPath(path.to_string()))?;
        // SAFETY: cpath valid for the duration; self.raw live.
        let raw = unsafe { czim_item_open(self.raw, cpath.as_ptr()) };
        if raw.is_null() {
            return Err(ZimBackendError::EntryNotFound(path.to_string()));
        }
        Ok(ZimItem { raw })
    }

    /// Read the whole item (convenience for the article object).
    pub fn item_bytes(&self, path: &str) -> Result<Vec<u8>, ZimBackendError> {
        let item = self.item(path)?;
        let size = item.size();
        item.read_range(0, size)
    }
}
