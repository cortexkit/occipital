//! The panic and argument boundary every entry point runs through.
//!
//! The rules of `include/occipital.h` that are the same for every entry: an
//! entry with an `out` sets `*out` to NULL before any other work, a
//! status-returning entry turns a panic into `OCC_PANIC` (a `void` entry
//! discards it), only REFUSED and REJECTED hand error JSON back, and a JSON
//! parameter that is NULL, not UTF-8, malformed or not the expected envelope
//! is `OCC_INVALID_ARGUMENT`.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::{ptr, slice};

use serde::de::DeserializeOwned;

use crate::status::{carries_payload, OCC_INTERNAL, OCC_INVALID_ARGUMENT, OCC_OK, OCC_PANIC};

/// Why an entry point did not succeed: a non-OK status and, for REFUSED and
/// REJECTED only, the error JSON handed back through a `char **out`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    /// The non-OK status to return.
    pub(crate) status: i32,
    /// Error JSON for `*out`. It is dropped, and `*out` stays NULL, unless the
    /// status is REFUSED or REJECTED.
    pub(crate) payload: Option<String>,
}

impl Failure {
    /// A failure that leaves `*out` NULL.
    pub(crate) fn bare(status: i32) -> Self {
        Self {
            status,
            payload: None,
        }
    }

    /// `OCC_INVALID_ARGUMENT`.
    pub(crate) fn invalid() -> Self {
        Self::bare(OCC_INVALID_ARGUMENT)
    }

    /// `OCC_INTERNAL`.
    pub(crate) fn internal() -> Self {
        Self::bare(OCC_INTERNAL)
    }
}

/// Runs an entry's whole body inside `catch_unwind` and returns its status,
/// or `OCC_PANIC` if it panicked.
///
/// `AssertUnwindSafe` is sound here because a panicking body hands nothing
/// back: the caller gets `OCC_PANIC`, and the header tells it to stop using a
/// session that panicked rather than observe half-updated state.
pub(crate) fn status_entry(body: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(OCC_PANIC)
}

/// Runs the body of a `void` entry (the `_free` functions) inside
/// `catch_unwind`. There is no status to report a panic with, so it is
/// dropped; letting it unwind into C would be undefined behaviour.
pub(crate) fn void_entry(body: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(body));
}

/// Runs an entry that hands a JSON string back through `out`.
///
/// `out` NULL is `OCC_INVALID_ARGUMENT`. Otherwise `*out` is set to NULL
/// first, then `body` runs. `Ok(json)` puts `json` in `*out` and returns
/// `OCC_OK`; a failure returns its status and puts its payload in `*out` only
/// for REFUSED and REJECTED.
///
/// # Safety
///
/// `out` must be NULL or valid for writing one pointer.
pub(crate) unsafe fn string_entry(
    out: *mut *mut c_char,
    body: impl FnOnce() -> Result<String, Failure>,
) -> i32 {
    status_entry(|| {
        if out.is_null() {
            return OCC_INVALID_ARGUMENT;
        }
        // SAFETY: `out` is not NULL, and the caller guarantees it is valid
        // for writing one pointer.
        unsafe { out.write(ptr::null_mut()) };
        let (status, json) = match body() {
            Ok(json) => (OCC_OK, Some(json)),
            Err(failure) => {
                let payload = failure.payload.filter(|_| carries_payload(failure.status));
                (failure.status, payload)
            }
        };
        let Some(json) = json else {
            return status;
        };
        // JSON text never holds a raw NUL (serde_json escapes it), so this
        // only fails on a broken encoder.
        match CString::new(json) {
            Ok(text) => {
                // SAFETY: as above; ownership of the string passes to the
                // caller, who frees it with `occ_string_free`.
                unsafe { out.write(text.into_raw()) };
                status
            }
            Err(_) => OCC_INTERNAL,
        }
    })
}

/// Runs an entry that hands a new handle back through `out`.
///
/// `out` NULL is `OCC_INVALID_ARGUMENT`. Otherwise `*out` is set to NULL
/// first, then `body` runs. `Ok(value)` boxes `value` into `*out` and returns
/// `OCC_OK`. A handle cannot carry error JSON, so every failure, REFUSED
/// included, leaves `*out` NULL.
///
/// # Safety
///
/// `out` must be NULL or valid for writing one pointer.
pub(crate) unsafe fn handle_entry<T>(
    out: *mut *mut T,
    body: impl FnOnce() -> Result<T, Failure>,
) -> i32 {
    status_entry(|| {
        if out.is_null() {
            return OCC_INVALID_ARGUMENT;
        }
        // SAFETY: `out` is not NULL, and the caller guarantees it is valid
        // for writing one pointer.
        unsafe { out.write(ptr::null_mut()) };
        match body() {
            Ok(value) => {
                // SAFETY: as above; ownership of the box passes to the
                // caller, who frees it with the handle's `_free` function.
                unsafe { out.write(Box::into_raw(Box::new(value))) };
                OCC_OK
            }
            Err(failure) => failure.status,
        }
    })
}

/// The bytes of a `(pointer, length)` buffer. Length 0 is the empty buffer
/// whatever the pointer, so NULL is allowed then; a NULL pointer with a
/// non-zero length is `OCC_INVALID_ARGUMENT`.
///
/// # Safety
///
/// When `len` is not 0 and `bytes` is not NULL, `bytes` must be valid for
/// reading `len` bytes for the lifetime `'a`.
pub(crate) unsafe fn buffer<'a>(bytes: *const u8, len: usize) -> Result<&'a [u8], Failure> {
    if len == 0 {
        return Ok(&[]);
    }
    if bytes.is_null() {
        return Err(Failure::invalid());
    }
    // SAFETY: `bytes` is not NULL and the caller guarantees it is valid for
    // `len` bytes.
    Ok(unsafe { slice::from_raw_parts(bytes, len) })
}

/// Decodes a JSON parameter into `T`, normally a closed
/// (`deny_unknown_fields`) envelope. NULL, invalid UTF-8, malformed JSON and
/// JSON that is not a valid `T` are all `OCC_INVALID_ARGUMENT`.
///
/// # Safety
///
/// `text` must be NULL or point to a NUL-terminated string that stays valid
/// and unchanged for the duration of the call.
pub(crate) unsafe fn json_arg<T: DeserializeOwned>(text: *const c_char) -> Result<T, Failure> {
    if text.is_null() {
        return Err(Failure::invalid());
    }
    // SAFETY: `text` is not NULL, and the caller guarantees it is a
    // NUL-terminated string valid for the duration of the call.
    let bytes = unsafe { CStr::from_ptr(text) };
    let text = bytes.to_str().map_err(|_| Failure::invalid())?;
    serde_json::from_str(text).map_err(|_| Failure::invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{OCC_REFUSED, OCC_REJECTED, OCC_SESSION_STATE};
    use std::ffi::CStr;

    /// A non-NULL value that is not a valid string, so a test can tell
    /// whether an entry overwrote `*out`.
    fn sentinel() -> *mut c_char {
        ptr::NonNull::<c_char>::dangling().as_ptr()
    }

    /// Takes the string an entry returned and frees it the way C callers do.
    fn take(out: *mut c_char) -> String {
        assert!(!out.is_null());
        // SAFETY: `out` came from `string_entry`, which returns a valid
        // NUL-terminated string owned by the caller.
        let text = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_owned();
        // SAFETY: as above; it is freed exactly once.
        unsafe { crate::occ_string_free(out) };
        text
    }

    #[test]
    fn string_entry_success_puts_the_json_in_out() {
        let mut out = sentinel();
        // SAFETY: `out` is a valid local.
        let status = unsafe { string_entry(&mut out, || Ok("{\"a\":1}".to_owned())) };
        assert_eq!(status, OCC_OK);
        assert_eq!(take(out), "{\"a\":1}");
    }

    #[test]
    fn refused_and_rejected_hand_back_their_error_json() {
        for status in [OCC_REFUSED, OCC_REJECTED] {
            let mut out = sentinel();
            // SAFETY: `out` is a valid local.
            let returned = unsafe {
                string_entry(&mut out, || {
                    Err(Failure {
                        status,
                        payload: Some("{\"error\":true}".to_owned()),
                    })
                })
            };
            assert_eq!(returned, status);
            assert_eq!(take(out), "{\"error\":true}");
        }
    }

    #[test]
    fn other_failures_leave_out_null_even_with_a_payload() {
        let mut out = sentinel();
        // SAFETY: `out` is a valid local.
        let status = unsafe {
            string_entry(&mut out, || {
                Err(Failure {
                    status: OCC_SESSION_STATE,
                    payload: Some("{\"error\":true}".to_owned()),
                })
            })
        };
        assert_eq!(status, OCC_SESSION_STATE);
        assert!(out.is_null());
    }

    #[test]
    fn a_panicking_string_body_returns_panic_with_out_null() {
        let mut out = sentinel();
        // SAFETY: `out` is a valid local.
        let status = unsafe { string_entry(&mut out, || panic!("boom")) };
        assert_eq!(status, OCC_PANIC);
        assert!(out.is_null());
    }

    #[test]
    fn a_panicking_handle_body_returns_panic_with_out_null() {
        let mut out: *mut u8 = ptr::NonNull::dangling().as_ptr();
        // SAFETY: `out` is a valid local.
        let status = unsafe { handle_entry::<u8>(&mut out, || panic!("boom")) };
        assert_eq!(status, OCC_PANIC);
        assert!(out.is_null());
    }

    #[test]
    fn a_null_out_is_an_invalid_argument_and_the_body_does_not_run() {
        let mut ran = false;
        // SAFETY: NULL is allowed for `out`.
        let status = unsafe {
            string_entry(ptr::null_mut(), || {
                ran = true;
                Ok(String::new())
            })
        };
        assert_eq!(status, OCC_INVALID_ARGUMENT);
        // SAFETY: NULL is allowed for `out`.
        let handle = unsafe {
            handle_entry::<u8>(ptr::null_mut(), || {
                ran = true;
                Ok(0)
            })
        };
        assert_eq!(handle, OCC_INVALID_ARGUMENT);
        assert!(!ran);
    }

    #[test]
    fn void_entry_swallows_a_panic() {
        void_entry(|| panic!("boom"));
    }
}
