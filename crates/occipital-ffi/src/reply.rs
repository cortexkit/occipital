//! Reply sessions: `occ_reply_new`, `occ_reply_chunk`, `occ_reply_finish`,
//! `occ_reply_event`, `occ_reply_measure` and `occ_reply_free`.

use std::ffi::c_char;

use occipital_lang::{measure, Catalog, Profile, ReplyEvent, ReplySession};

use crate::account::OccAccount;
use crate::boundary::{buffer, handle_entry, json_arg, string_entry, void_entry, Failure};
use crate::envelope::{EventEnvelope, LayoutEnvelope, RecordsEnvelope, StampEnvelope};
use crate::metrics::{Adapter, OccMetrics};
use crate::output::{optional_revision, reply_failure, to_json, MeasurementJson, RevisionJson};
use crate::status::{OCC_MEASURE_ERROR, OCC_SESSION_STATE};

/// An opaque reply session handle (`OccReply` in the header). It owns its
/// own clones of the chat and app accounts it was created with.
pub struct OccReply {
    pub(crate) session: ReplySession,
}

/// The session behind a handle, or `OCC_INVALID_ARGUMENT` for NULL.
///
/// # Safety
///
/// `reply` must be NULL or a live handle from [`occ_reply_new`] that no
/// other call is using.
unsafe fn session<'a>(reply: *mut OccReply) -> Result<&'a mut ReplySession, Failure> {
    // SAFETY: the caller guarantees a non-NULL `reply` is live and not in
    // use elsewhere.
    unsafe { reply.as_mut() }
        .map(|reply| &mut reply.session)
        .ok_or_else(Failure::invalid)
}

/// The account behind a handle, or `OCC_INVALID_ARGUMENT` for NULL.
///
/// # Safety
///
/// `account` must be NULL or a live handle from `occ_account_new`.
unsafe fn account<'a>(account: *const OccAccount) -> Result<&'a OccAccount, Failure> {
    // SAFETY: the caller guarantees a non-NULL `account` is live.
    unsafe { account.as_ref() }.ok_or_else(Failure::invalid)
}

/// A session for one reply: `reply::Session::new` with the reply profile and
/// the embedded catalog. `stamp_json` is `{"dialect"}` and `layout_json` the
/// layout context. The session clones `chat` and `app`, so either handle may
/// be freed while the session lives. An account that refuses the session
/// (exhausted, or of the wrong scope) is `OCC_REFUSED` with `*out` NULL.
///
/// # Safety
///
/// `stamp_json` and `layout_json` must be NULL or NUL-terminated strings,
/// `chat` and `app` NULL or live account handles, and `out` NULL or valid
/// for writing one pointer, all for the duration of the call. A handle
/// written to `*out` must be freed exactly once with [`occ_reply_free`].
#[no_mangle]
pub unsafe extern "C" fn occ_reply_new(
    stamp_json: *const c_char,
    layout_json: *const c_char,
    chat: *const OccAccount,
    app: *const OccAccount,
    out: *mut *mut OccReply,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `json_arg` and
        // `account`.
        let (stamp, layout, chat, app) = unsafe {
            (
                json_arg::<StampEnvelope>(stamp_json)?,
                json_arg::<LayoutEnvelope>(layout_json)?,
                account(chat)?,
                account(app)?,
            )
        };
        let session = ReplySession::new(
            Profile::Reply,
            stamp.dialect,
            Catalog::v1(),
            layout.into_context()?,
            chat.account.clone(),
            app.account.clone(),
        )
        .map_err(reply_failure)?;
        Ok(OccReply { session })
    };
    // SAFETY: the caller's guarantee about `out` is `handle_entry`'s.
    unsafe { handle_entry(out, body) }
}

/// Feeds streamed reply bytes, unvalidated: a UTF-8 sequence may be split
/// across calls. `*out` is the provisional revision JSON, or JSON `null` when
/// nothing visible changed. After finishing this is `OCC_SESSION_STATE`.
///
/// # Safety
///
/// `reply` must be NULL or a live handle no other call is using, `bytes`
/// valid for `len` bytes (or anything when `len` is 0), and `out` NULL or
/// valid for writing one pointer.
#[no_mangle]
pub unsafe extern "C" fn occ_reply_chunk(
    reply: *mut OccReply,
    bytes: *const u8,
    len: usize,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `session` and `buffer`.
        let (session, bytes) = unsafe { (session(reply)?, buffer(bytes, len)?) };
        let revision = session.chunk(bytes).map_err(reply_failure)?;
        optional_revision(revision.as_ref())
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

/// Feeds the final bytes, unvalidated, and finishes the reply with the
/// records in `records_json` (`{"references":{N:record},"handles":[…]}`).
/// `*out` is the authoritative revision JSON. Finishing twice is
/// `OCC_SESSION_STATE`.
///
/// # Safety
///
/// As [`occ_reply_chunk`], and `records_json` must be NULL or a
/// NUL-terminated string valid for the call.
#[no_mangle]
pub unsafe extern "C" fn occ_reply_finish(
    reply: *mut OccReply,
    tail: *const u8,
    len: usize,
    records_json: *const c_char,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `session`, `buffer`
        // and `json_arg`.
        let (session, tail, records) = unsafe {
            (
                session(reply)?,
                buffer(tail, len)?,
                json_arg::<RecordsEnvelope>(records_json)?,
            )
        };
        let revision = session
            .finish(tail, records.references.0, records.handles)
            .map_err(reply_failure)?;
        to_json(&RevisionJson::new(&revision))
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

/// Delivers one late host event, tagged by `kind` (see
/// [`EventEnvelope`]). `MeasureComplete` is not accepted: only
/// [`occ_reply_measure`] delivers measurements. `*out` is the resulting
/// revision JSON, or JSON `null` when nothing changed. An event that needs a
/// finished reply before finishing is `OCC_SESSION_STATE`.
///
/// # Safety
///
/// `reply` must be NULL or a live handle no other call is using,
/// `event_json` NULL or a NUL-terminated string valid for the call, and
/// `out` NULL or valid for writing one pointer.
#[no_mangle]
pub unsafe extern "C" fn occ_reply_event(
    reply: *mut OccReply,
    event_json: *const c_char,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `session` and
        // `json_arg`.
        let (session, event) = unsafe { (session(reply)?, json_arg::<EventEnvelope>(event_json)?) };
        let revision = session.event(event.into_event()?).map_err(reply_failure)?;
        optional_revision(revision.as_ref())
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

/// Measures the session's current candidate revision under `layout_json`
/// with `metrics`, and only when that succeeds delivers the measurement to
/// the session as `MeasureComplete`. `*out` is the measurement JSON. A
/// session with no candidate yet is `OCC_SESSION_STATE`. A metrics callback
/// returning an unusable value, or a tree the library cannot measure, is
/// `OCC_MEASURE_ERROR`, and the session is left unchanged.
///
/// # Safety
///
/// `reply` must be NULL or a live handle no other call is using,
/// `layout_json` NULL or a NUL-terminated string, `metrics` NULL or a valid
/// table whose callbacks never call back into this library, and `out` NULL
/// or valid for writing one pointer, all for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn occ_reply_measure(
    reply: *mut OccReply,
    layout_json: *const c_char,
    metrics: *const OccMetrics,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `session`, `json_arg`
        // and `Adapter::new`.
        let (session, layout, metrics) = unsafe {
            (
                session(reply)?,
                json_arg::<LayoutEnvelope>(layout_json)?,
                Adapter::new(metrics)?,
            )
        };
        let layout = layout.into_context()?;
        let candidate = session
            .candidate
            .clone()
            .ok_or_else(|| Failure::bare(OCC_SESSION_STATE))?;
        let measured = measure(&candidate.measure_revision(), &layout, &metrics);
        // `measure` is pure, so discarding a measurement made with an
        // unusable callback value leaves nothing behind.
        let measured = match measured {
            Ok(measured) if !metrics.failed() => measured,
            _ => return Err(Failure::bare(OCC_MEASURE_ERROR)),
        };
        let json = to_json(&MeasurementJson::new(&measured))?;
        session
            .event(ReplyEvent::MeasureComplete(Box::new(measured)))
            .map_err(reply_failure)?;
        Ok(json)
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

/// Frees a reply session. NULL does nothing.
///
/// # Safety
///
/// `reply` must be NULL or a handle from [`occ_reply_new`] that has not been
/// freed, and no other call may be using it.
#[no_mangle]
pub unsafe extern "C" fn occ_reply_free(reply: *mut OccReply) {
    void_entry(|| {
        if !reply.is_null() {
            // SAFETY: the caller guarantees `reply` came from `occ_reply_new`
            // (a `Box::into_raw`) and is freed only once.
            drop(unsafe { Box::from_raw(reply) });
        }
    })
}

#[cfg(test)]
mod tests;
