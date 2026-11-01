//! `occ_string_free`, which frees every string returned through `char **out`.

use std::ffi::{c_char, CString};

use crate::boundary::void_entry;

/// Frees a string returned through a `char **out`. NULL does nothing.
///
/// # Safety
///
/// `s` must be NULL or a string returned by this library that has not been
/// freed.
#[no_mangle]
pub unsafe extern "C" fn occ_string_free(s: *mut c_char) {
    void_entry(|| {
        if !s.is_null() {
            // SAFETY: the caller guarantees `s` came from `CString::into_raw`
            // in an entry point and is freed only once.
            drop(unsafe { CString::from_raw(s) });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn freeing_null_does_nothing() {
        // SAFETY: NULL is allowed.
        unsafe { occ_string_free(ptr::null_mut()) };
    }
}
