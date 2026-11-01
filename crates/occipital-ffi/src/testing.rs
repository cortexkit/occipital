//! Helpers shared by this crate's tests: calling entries the way a C caller
//! does, and freeing what they return.

use std::ffi::{c_char, CStr, CString};
use std::ptr;

use crate::account::{occ_account_new, OccAccount};
use crate::reply::{occ_reply_new, OccReply};

/// A layout context as `layout_json`.
pub(crate) const LAYOUT: &str =
    r#"{"width":"320","content_size_category":"normal","network":"yes","placement":"page"}"#;

/// The reply dialect as `stamp_json`.
pub(crate) const STAMP: &str = r#"{"dialect":"1.0"}"#;

/// Records with no references or handles, as `records_json`.
pub(crate) const NO_RECORDS: &str = r#"{"references":{},"handles":[]}"#;

/// Records with references 7 and 22 resolved, as `records_json`.
pub(crate) const RECORDS: &str = r#"{"references":{"7":{"state":"resolved","reason":null,"tool":"bash","bytes":20,"call_key":"a"},"22":{"state":"resolved","tool":"du","bytes":40,"call_key":"b"}},"handles":[]}"#;

/// A tagged reply of prose with two inline Badges, a Callout with Markdown
/// content and a Chart over reference 7. Its measurement is prose-deferred:
/// it starts with a paragraph, which the library does not measure yet.
pub(crate) const TAGGED: &str = "The test run finished with one failure. <Badge tone=\"danger\">1 failed</Badge> <Badge tone=\"success\">17 passed</Badge>\n\n<Callout tone=\"warning\" title=\"testPaging failed\">\nThe second page came back empty after its cursor expired. The fix is in `PageLoader.swift`: the cursor is now refreshed before the request.\n</Callout>\n\nHow long each suite took:\n\n<Chart kind=\"bar\" data={Parse($$tc(7), \"csv\", {columns: {seconds: \"number\"}})} x=\"suite\" y=\"seconds\" aspect=\"16:9\" />";

/// A tagged reply of block components only, so its whole height is
/// measured through the metrics callbacks.
pub(crate) const BLOCKS: &str = "<Callout tone=\"warning\" title=\"testPaging failed\" description=\"The second page came back empty after its cursor expired.\" />\n\n<Table data={Parse($$tc(22), \"json\", {columns: {path: \"text\", bytes: \"number\"}})} columns={[\"path\", \"bytes\"]} displayRows={6} />\n\n<Chart kind=\"bar\" data={Parse($$tc(7), \"csv\", {columns: {seconds: \"number\"}})} x=\"suite\" y=\"seconds\" aspect=\"16:9\" />";

/// A tagged reply with a Table component and a Markdown table holding
/// Badges.
pub(crate) const TABLE: &str = "Your three largest folders are below. <Styled tone=\"muted\">Sizes come from a scan two minutes ago.</Styled>\n\n<Table data={Parse($$tc(22), \"json\", {columns: {path: \"text\", bytes: \"number\"}})} columns={[\"path\", \"bytes\"]} displayRows={6} />\n\n| Folder | Safe to clear? |\n|---|---|\n| `~/Library/Caches` | <Badge tone=\"success\">Yes</Badge> |\n| `~/Work` | <Badge tone=\"danger\">No</Badge> |";

/// A tagged reply whose text holds two- and four-byte UTF-8 sequences in
/// props, Markdown and tag content. The first Callout is measured through
/// the callbacks before the prose defers the rest.
pub(crate) const MULTIBYTE: &str = "<Callout tone=\"info\" title=\"Café ☕ é🙂\" description=\"Crème brûlée 🙂 é\" />\n\nCafé ☕ is open. <Badge tone=\"info\">é🙂</Badge>\n\n<Callout tone=\"warning\" title=\"Ünïcode 🎉\">\nWeiter é 🙂\n</Callout>";

/// `text` as a C string.
pub(crate) fn c(text: &str) -> CString {
    CString::new(text).unwrap()
}

/// Calls an entry with a `char **out` that starts as a non-NULL sentinel,
/// and returns its status and the string it left in `*out`, freeing it.
pub(crate) fn call(entry: impl FnOnce(*mut *mut c_char) -> i32) -> (i32, Option<String>) {
    let mut out: *mut c_char = ptr::NonNull::dangling().as_ptr();
    let status = entry(&mut out);
    if out.is_null() {
        return (status, None);
    }
    // SAFETY: a non-NULL `out` is a string the entry returned.
    let text = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_owned();
    // SAFETY: freed exactly once.
    unsafe { crate::occ_string_free(out) };
    (status, Some(text))
}

/// A fresh account handle of `scope`.
pub(crate) fn account(scope: i32) -> *mut OccAccount {
    let mut out = ptr::null_mut();
    // SAFETY: `out` is a valid local.
    assert_eq!(unsafe { occ_account_new(scope, &mut out) }, crate::OCC_OK);
    out
}

/// Calls `occ_reply_new` with an out pointer that starts as a non-NULL
/// sentinel, and returns its status and handle.
pub(crate) fn new_reply_raw(
    stamp: *const c_char,
    layout: *const c_char,
    chat: *const OccAccount,
    app: *const OccAccount,
) -> (i32, *mut OccReply) {
    let mut out: *mut OccReply = ptr::NonNull::dangling().as_ptr();
    // SAFETY: every pointer is NULL or live, and `out` is a valid local.
    let status = unsafe { occ_reply_new(stamp, layout, chat, app, &mut out) };
    (status, out)
}

/// A new reply session over fresh accounts (whose handles are freed at
/// once: the session keeps its own clones).
pub(crate) fn new_reply() -> *mut OccReply {
    let (chat, app) = (account(0), account(1));
    let (stamp, layout) = (c(STAMP), c(LAYOUT));
    let (status, reply) = new_reply_raw(stamp.as_ptr(), layout.as_ptr(), chat, app);
    // SAFETY: both handles are live and freed exactly once.
    unsafe {
        crate::occ_account_free(chat);
        crate::occ_account_free(app);
    }
    assert_eq!(status, crate::OCC_OK);
    reply
}

/// `text` parsed as JSON.
pub(crate) fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}
