//! Test-only entry points. They are compiled only for this crate's own tests
//! and under the non-default `test-hooks` feature, so the shipped archive
//! never carries them. The header declares them behind `OCC_TEST_HOOKS`.

use std::ffi::c_char;

use crate::boundary::{json_arg, status_entry, string_entry, Failure};
use crate::envelope::LayoutEnvelope;

/// Panics inside the boundary and returns `OCC_PANIC`, proving a panic is
/// caught and the process keeps running.
#[no_mangle]
// Panicking is this hook's whole job.
#[allow(clippy::panic)]
pub extern "C" fn occ_test_force_panic() -> i32 {
    status_entry(|| panic!("occ_test_force_panic: forced panic"))
}

/// Decodes `layout_json` the way every `layout_json` parameter is decoded and
/// returns the decoded layout re-encoded as canonical JSON. It exercises the
/// shared argument rules: NULL, invalid UTF-8, malformed JSON and unknown
/// envelope members are all `OCC_INVALID_ARGUMENT` with `*out` NULL.
///
/// # Safety
///
/// `layout_json` must be NULL or a NUL-terminated string valid for the call,
/// and `out` NULL or valid for writing one pointer. A string written to
/// `*out` must be freed with [`occ_string_free`](crate::occ_string_free).
#[no_mangle]
pub unsafe extern "C" fn occ_test_decode_layout(
    layout_json: *const c_char,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantee about `layout_json` is `json_arg`'s.
        let envelope: LayoutEnvelope = unsafe { json_arg(layout_json) }?;
        let layout = envelope.into_context()?;
        serde_json::to_string(&LayoutEnvelope::from_context(&layout))
            .map_err(|_| Failure::internal())
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{occ_string_free, occ_version, OCC_INVALID_ARGUMENT, OCC_OK, OCC_PANIC};
    use std::ffi::{CStr, CString};
    use std::ptr;

    const LAYOUT: &str =
        r#"{"width":"390","content_size_category":"large","network":"yes","placement":"row"}"#;

    /// Calls the hook on raw bytes (a NUL is appended) and returns the status
    /// and the returned string, if any, freeing it.
    fn decode(bytes: &[u8]) -> (i32, Option<String>) {
        let input = CString::new(bytes).unwrap();
        decode_ptr(input.as_ptr())
    }

    fn decode_ptr(input: *const c_char) -> (i32, Option<String>) {
        let mut out: *mut c_char = ptr::NonNull::dangling().as_ptr();
        // SAFETY: `input` is NULL or a live NUL-terminated string, and `out`
        // is a valid local.
        let status = unsafe { occ_test_decode_layout(input, &mut out) };
        if out.is_null() {
            return (status, None);
        }
        // SAFETY: a non-NULL `out` is a string the hook returned.
        let text = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_owned();
        // SAFETY: freed exactly once.
        unsafe { occ_string_free(out) };
        (status, Some(text))
    }

    #[test]
    fn force_panic_returns_panic_and_the_process_keeps_running() {
        assert_eq!(occ_test_force_panic(), OCC_PANIC);
        // The library still works after the caught panic.
        assert_eq!(occ_test_force_panic(), OCC_PANIC);
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a valid local.
        assert_eq!(unsafe { occ_version(&mut out) }, OCC_OK);
        // SAFETY: freed exactly once.
        unsafe { occ_string_free(out) };
    }

    #[test]
    fn a_valid_layout_round_trips() {
        assert_eq!(decode(LAYOUT.as_bytes()), (OCC_OK, Some(LAYOUT.to_owned())));
    }

    #[test]
    fn a_null_json_parameter_is_an_invalid_argument_with_out_null() {
        assert_eq!(decode_ptr(ptr::null()), (OCC_INVALID_ARGUMENT, None));
    }

    #[test]
    fn a_null_out_is_an_invalid_argument() {
        let input = CString::new(LAYOUT).unwrap();
        // SAFETY: `input` is live; NULL is allowed for `out`.
        let status = unsafe { occ_test_decode_layout(input.as_ptr(), ptr::null_mut()) };
        assert_eq!(status, OCC_INVALID_ARGUMENT);
    }

    #[test]
    fn invalid_utf8_is_an_invalid_argument_with_out_null() {
        // A valid layout with one byte inside a string replaced by 0xFF, and
        // one with a lone UTF-8 lead byte (0xC3) in place of that byte.
        let at = LAYOUT.find("large").unwrap();
        let with = |byte: u8| {
            [
                &LAYOUT.as_bytes()[..at],
                &[byte],
                &LAYOUT.as_bytes()[at + 1..],
            ]
            .concat()
        };
        for bytes in [with(0xFF), with(0xC3)] {
            assert_eq!(decode(&bytes), (OCC_INVALID_ARGUMENT, None));
        }
    }

    #[test]
    fn malformed_json_is_an_invalid_argument_with_out_null() {
        for json in [
            "",
            "{",
            "{\"width\":\"390\",}",
            "null",
            "[]",
            &format!("{LAYOUT} {{}}"),
        ] {
            assert_eq!(
                decode(json.as_bytes()),
                (OCC_INVALID_ARGUMENT, None),
                "{json}"
            );
        }
    }

    #[test]
    fn unknown_envelope_members_are_an_invalid_argument_with_out_null() {
        let extra = LAYOUT.replace('}', r#","extra":true}"#);
        let renamed = LAYOUT.replace("content_size_category", "contentSizeCategory");
        for json in [extra, renamed] {
            assert_eq!(
                decode(json.as_bytes()),
                (OCC_INVALID_ARGUMENT, None),
                "{json}"
            );
        }
    }
}
