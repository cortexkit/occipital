//! Account handles: `occ_account_new` and `occ_account_free`.

use occipital_lang::SharedAccount;

use crate::boundary::{handle_entry, void_entry, Failure};

/// An opaque chat or app account handle (`OccAccount` in the header). The
/// account itself is shared: sessions clone it, so freeing the handle does not
/// end the account while a session still uses it.
pub struct OccAccount {
    pub(crate) account: SharedAccount,
}

/// `OCC_SCOPE_CHAT` in the header.
const SCOPE_CHAT: i32 = 0;
/// `OCC_SCOPE_APP` in the header.
const SCOPE_APP: i32 = 1;

/// A fresh account: `scope` 0 is a chat account ([`SharedAccount::chat`]) and
/// 1 an app account ([`SharedAccount::app`]). Any other scope, or a NULL
/// `out`, is `OCC_INVALID_ARGUMENT`.
///
/// # Safety
///
/// `out` must be NULL or valid for writing one pointer. A handle written to
/// `*out` must be freed exactly once with [`occ_account_free`].
#[no_mangle]
pub unsafe extern "C" fn occ_account_new(scope: i32, out: *mut *mut OccAccount) -> i32 {
    // SAFETY: the caller's guarantee about `out` is `handle_entry`'s.
    unsafe {
        handle_entry(out, || {
            let account = match scope {
                SCOPE_CHAT => SharedAccount::chat(),
                SCOPE_APP => SharedAccount::app(),
                _ => return Err(Failure::invalid()),
            };
            Ok(OccAccount { account })
        })
    }
}

/// Frees an account handle. NULL does nothing.
///
/// # Safety
///
/// `account` must be NULL or a handle from [`occ_account_new`] that has not
/// been freed, and no other call may be using it.
#[no_mangle]
pub unsafe extern "C" fn occ_account_free(account: *mut OccAccount) {
    void_entry(|| {
        if !account.is_null() {
            // SAFETY: the caller guarantees `account` came from
            // `occ_account_new` (a `Box::into_raw`) and is freed only once.
            drop(unsafe { Box::from_raw(account) });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OCC_INVALID_ARGUMENT, OCC_OK};
    use occipital_lang::AccountScope;
    use std::ptr;

    fn new_account(scope: i32) -> (i32, *mut OccAccount) {
        let mut out: *mut OccAccount = ptr::NonNull::dangling().as_ptr();
        // SAFETY: `out` is a valid local.
        let status = unsafe { occ_account_new(scope, &mut out) };
        (status, out)
    }

    #[test]
    fn scope_zero_is_chat_and_one_is_app() {
        for (scope, expected) in [(0, AccountScope::Chat), (1, AccountScope::App)] {
            let (status, account) = new_account(scope);
            assert_eq!(status, OCC_OK);
            assert!(!account.is_null());
            // SAFETY: `account` is a live handle from `occ_account_new`.
            assert_eq!(unsafe { &*account }.account.scope(), expected);
            // SAFETY: freed exactly once.
            unsafe { occ_account_free(account) };
        }
    }

    #[test]
    fn an_unknown_scope_is_an_invalid_argument_with_out_null() {
        for scope in [-1, 2, i32::MAX, i32::MIN] {
            let (status, account) = new_account(scope);
            assert_eq!(status, OCC_INVALID_ARGUMENT, "{scope}");
            assert!(account.is_null(), "{scope}");
        }
    }

    #[test]
    fn a_null_out_is_an_invalid_argument() {
        // SAFETY: NULL is allowed for `out`.
        let status = unsafe { occ_account_new(0, ptr::null_mut()) };
        assert_eq!(status, OCC_INVALID_ARGUMENT);
    }

    #[test]
    fn freeing_null_does_nothing() {
        // SAFETY: NULL is allowed.
        unsafe { occ_account_free(ptr::null_mut()) };
    }
}
