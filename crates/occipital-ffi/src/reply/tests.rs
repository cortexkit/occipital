use std::ptr;

use occipital_lang::{AccountScope, RetainedAllocation};
use serde_json::Value as Json;

use super::*;
use crate::metrics::tests::{table, Script};
use crate::status::{OCC_INVALID_ARGUMENT, OCC_OK, OCC_REFUSED};
use crate::testing::{
    account, c, call, json, new_reply, new_reply_raw, BLOCKS, LAYOUT, MULTIBYTE, NO_RECORDS,
    RECORDS, STAMP, TAGGED,
};

fn chunk(reply: *mut OccReply, bytes: &[u8]) -> (i32, Option<String>) {
    // SAFETY: `reply` is live and `bytes` valid for its length.
    call(|out| unsafe { occ_reply_chunk(reply, bytes.as_ptr(), bytes.len(), out) })
}

fn finish(reply: *mut OccReply, tail: &[u8], records: &str) -> (i32, Option<String>) {
    let records = c(records);
    // A NULL tail pointer when the tail is empty, as a C caller may pass.
    let bytes = if tail.is_empty() {
        ptr::null()
    } else {
        tail.as_ptr()
    };
    // SAFETY: `reply` is live, `bytes` valid for `tail.len()` bytes and
    // `records` a live C string.
    call(|out| unsafe { occ_reply_finish(reply, bytes, tail.len(), records.as_ptr(), out) })
}

fn event(reply: *mut OccReply, event: &str) -> (i32, Option<String>) {
    let event = c(event);
    // SAFETY: `reply` is live and `event` a live C string.
    call(|out| unsafe { occ_reply_event(reply, event.as_ptr(), out) })
}

fn measure_with(reply: *mut OccReply, script: &mut Script) -> (i32, Option<String>) {
    let layout = c(LAYOUT);
    let metrics = table(script);
    // SAFETY: `reply` is live, `layout` a live C string and `metrics` a live
    // table over a live script.
    call(|out| unsafe { occ_reply_measure(reply, layout.as_ptr(), &metrics, out) })
}

fn measure_stub(reply: *mut OccReply) -> (i32, Option<String>) {
    measure_with(reply, &mut Script::default())
}

fn free(reply: *mut OccReply) {
    // SAFETY: `reply` is live and freed exactly once.
    unsafe { occ_reply_free(reply) };
}

fn ok(result: (i32, Option<String>)) -> Json {
    assert_eq!(result.0, OCC_OK, "{:?}", result.1);
    json(&result.1.unwrap())
}

fn session_of<'a>(reply: *mut OccReply) -> &'a ReplySession {
    // SAFETY: `reply` is a live handle and no entry is running on it.
    &unsafe { &*reply }.session
}

/// A revision's JSON text without its leading id member. A streamed reply
/// publishes provisional revisions first, so its final revision has a later
/// id than a one-shot one even when everything else is equal. The rest is
/// compared as text, so prop order in the tree counts too.
fn without_id(revision: &str) -> String {
    assert!(revision.starts_with("{\"id\":"), "{revision}");
    revision.split_once(',').unwrap().1.to_owned()
}

/// A measurement's JSON without the revision id in its layout key, for the
/// same reason.
fn without_revision(mut measurement: Json) -> Json {
    measurement["key"]
        .as_object_mut()
        .unwrap()
        .remove("revision");
    measurement
}

fn ok_text(result: (i32, Option<String>)) -> String {
    assert_eq!(result.0, OCC_OK, "{:?}", result.1);
    result.1.unwrap()
}

/// What a finished reply gave: its revision text without the id, the
/// measurement without the revision id, and what the stub metrics were
/// asked.
type Finished = (String, Json, Vec<String>);

/// Finishes `source` in one call and measures it with the stub metrics.
fn one_shot(source: &[u8], records: &str) -> Finished {
    let reply = new_reply();
    let revision = ok_text(finish(reply, source, records));
    let mut script = Script::default();
    let measurement = ok(measure_with(reply, &mut script));
    free(reply);
    (
        without_id(&revision),
        without_revision(measurement),
        script.texts,
    )
}

/// Feeds `chunks` in order, finishes with `tail`, and measures with the stub
/// metrics. Every chunk must succeed and give `null` or a provisional
/// revision, and the measurement must be committed.
fn streamed(chunks: &[&[u8]], tail: &[u8], records: &str) -> Finished {
    let reply = new_reply();
    for bytes in chunks {
        let result = ok(chunk(reply, bytes));
        assert!(
            result.is_null() || result["provisional"] == true,
            "{result}"
        );
    }
    let revision = ok_text(finish(reply, tail, records));
    assert_eq!(json(&revision)["provisional"], false);
    let mut script = Script::default();
    let measurement = ok(measure_with(reply, &mut script));
    assert!(session_of(reply).committed.is_some());
    free(reply);
    (
        without_id(&revision),
        without_revision(measurement),
        script.texts,
    )
}

#[test]
fn one_byte_chunks_finish_and_measure_like_one_shot() {
    for source in [TAGGED, BLOCKS] {
        let expected = one_shot(source.as_bytes(), RECORDS);
        assert_eq!(
            json(&format!("{{{}", expected.0))["diagnostics"],
            Json::Array(vec![])
        );
        let bytes: Vec<&[u8]> = source.as_bytes().chunks(1).collect();
        assert_eq!(streamed(&bytes, &[], RECORDS), expected);
    }
    let prose = one_shot(TAGGED.as_bytes(), RECORDS);
    assert!(json(&format!("{{{}", prose.0))["plain_text"]
        .as_str()
        .unwrap()
        .starts_with("The test run finished with one failure. [1 failed] [17 passed]"));
    // The block-only reply is measured in full, through the callbacks.
    let blocks = one_shot(BLOCKS.as_bytes(), RECORDS);
    assert!(blocks.1["height"]["known"].is_string(), "{}", blocks.1);
    assert!(
        !blocks.1["boxes"].as_array().unwrap().is_empty(),
        "{}",
        blocks.1
    );
    assert!(
        blocks.2.iter().any(|text| text == "testPaging failed"),
        "{:?}",
        blocks.2
    );
}

#[test]
fn multibyte_utf8_split_at_every_byte_matches_one_shot() {
    let source = MULTIBYTE.as_bytes();
    let expected = one_shot(source, NO_RECORDS);
    let revision = json(&format!("{{{}", expected.0));
    assert_eq!(revision["diagnostics"], Json::Array(vec![]));
    let plain = revision["plain_text"].as_str().unwrap();
    assert!(plain.contains("Café ☕ is open. [é🙂]"), "{plain}");
    assert!(plain.contains("Weiter é 🙂"), "{plain}");
    assert!(!plain.contains('\u{FFFD}'), "{plain}");
    // The first Callout's title and description reach the callbacks whole.
    assert!(
        expected.2.iter().any(|text| text == "Café ☕ é🙂"),
        "{:?}",
        expected.2
    );
    assert!(
        expected.2.iter().any(|text| text == "Crème brûlée 🙂 é"),
        "{:?}",
        expected.2
    );
    for split in 0..=source.len() {
        let (head, rest) = source.split_at(split);
        // The second part as the finish tail, and as a chunk before an
        // empty tail.
        assert_eq!(streamed(&[head], rest, NO_RECORDS), expected, "{split}");
        assert_eq!(
            streamed(&[head, rest], &[], NO_RECORDS),
            expected,
            "{split}"
        );
    }
    let bytes: Vec<&[u8]> = source.chunks(1).collect();
    assert_eq!(streamed(&bytes, &[], NO_RECORDS), expected);
}

#[test]
fn chunks_are_passed_unvalidated_and_an_empty_null_buffer_is_allowed() {
    let reply = new_reply();
    // SAFETY: `reply` is live; a NULL buffer is allowed with length 0.
    let empty = call(|out| unsafe { occ_reply_chunk(reply, ptr::null(), 0, out) });
    assert_eq!(empty, (OCC_OK, Some("null".to_owned())));
    // Bytes that are not UTF-8 are the library's to repair, not an invalid
    // argument.
    assert_eq!(chunk(reply, b"bad \xFF \xC3").0, OCC_OK);
    let revision = ok(finish(reply, b"\xA9 end", NO_RECORDS));
    assert!(revision["plain_text"].as_str().unwrap().ends_with("end"));
    free(reply);
}

#[test]
fn a_null_buffer_with_a_length_is_an_invalid_argument() {
    let reply = new_reply();
    // SAFETY: `reply` is live; the NULL buffer is refused before any read.
    let result = call(|out| unsafe { occ_reply_chunk(reply, ptr::null(), 3, out) });
    assert_eq!(result, (OCC_INVALID_ARGUMENT, None));
    let records = c(NO_RECORDS);
    // SAFETY: as above.
    let result =
        call(|out| unsafe { occ_reply_finish(reply, ptr::null(), 1, records.as_ptr(), out) });
    assert_eq!(result, (OCC_INVALID_ARGUMENT, None));
    free(reply);
}

#[test]
fn state_errors_are_session_state_with_out_null() {
    let reply = new_reply();
    // Nothing to measure before any revision exists.
    assert_eq!(measure_stub(reply), (OCC_SESSION_STATE, None));
    // Late events need the final call first.
    assert_eq!(
        event(reply, r#"{"kind":"account_release"}"#),
        (OCC_SESSION_STATE, None)
    );
    assert_eq!(
        event(
            reply,
            r#"{"kind":"snapshot","reference":7,"result":"failed"}"#
        ),
        (OCC_SESSION_STATE, None)
    );
    ok(finish(reply, b"Hello <Badge>x</Badge>", NO_RECORDS));
    // Finishing twice, and streaming after finishing.
    assert_eq!(finish(reply, b"", NO_RECORDS), (OCC_SESSION_STATE, None));
    assert_eq!(chunk(reply, b"more"), (OCC_SESSION_STATE, None));
    free(reply);
}

#[test]
fn events_after_finishing_return_a_revision_or_null() {
    let reply = new_reply();
    ok(finish(reply, TAGGED.as_bytes(), RECORDS));
    let released = ok(event(reply, r#"{"kind":"account_release"}"#));
    assert_eq!(released["provisional"], false);
    // An unknown handle record changes nothing.
    let unknown = ok(event(
        reply,
        r#"{"kind":"handle_availability","record_id":"nope","available":false}"#,
    ));
    assert!(unknown.is_null());
    // Delivered snapshot bytes (base64 of "suite,seconds\nunit,2\n").
    let delivered = ok(event(
        reply,
        r#"{"kind":"snapshot","reference":7,"result":{"ok":"c3VpdGUsc2Vjb25kcwp1bml0LDIK"}}"#,
    ));
    assert_eq!(delivered["provisional"], false);
    let layout = ok(event(
        reply,
        &format!(r#"{{"kind":"layout","layout":{LAYOUT}}}"#),
    ));
    assert_eq!(layout["id"], delivered["id"]);
    free(reply);
}

#[test]
fn measure_complete_is_not_an_event_a_host_sends() {
    let reply = new_reply();
    ok(finish(reply, TAGGED.as_bytes(), RECORDS));
    for kind in ["measure_complete", "MeasureComplete"] {
        let json = format!(r#"{{"kind":"{kind}"}}"#);
        assert_eq!(event(reply, &json), (OCC_INVALID_ARGUMENT, None), "{kind}");
    }
    assert!(session_of(reply).committed.is_none());
    free(reply);
}

#[test]
fn an_unusable_metric_is_a_measure_error_and_changes_nothing() {
    let reply = new_reply();
    ok(finish(reply, BLOCKS.as_bytes(), RECORDS));
    let bad = [
        f64::NAN,
        -1.0,
        -0.000_001,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e21,
    ];
    for value in bad {
        let mut script = Script {
            value: Some(value),
            ..Script::default()
        };
        assert_eq!(
            measure_with(reply, &mut script),
            (OCC_MEASURE_ERROR, None),
            "{value:e}"
        );
        assert!(!script.styles.is_empty(), "the callbacks ran for {value:e}");
        assert!(session_of(reply).committed.is_none(), "{value:e}");
    }
    // A good measurement commits; a later bad one leaves that commit alone.
    let good = ok(measure_stub(reply));
    let committed = |reply| {
        session_of(reply)
            .committed
            .as_ref()
            .map(|(revision, measured)| (revision.id, measured.clone()))
    };
    let before = committed(reply).unwrap();
    assert_eq!(good["key"]["revision"], before.0);
    for value in bad {
        let mut script = Script {
            value: Some(value),
            ..Script::default()
        };
        assert_eq!(measure_with(reply, &mut script).0, OCC_MEASURE_ERROR);
        assert_eq!(committed(reply), Some(before.clone()), "{value:e}");
    }
    free(reply);
}

#[test]
fn an_exhausted_account_refuses_with_its_error_json() {
    let (chat, app) = (account(0), account(1));
    let (stamp, layout) = (c(STAMP), c(LAYOUT));
    let (status, reply) = new_reply_raw(stamp.as_ptr(), layout.as_ptr(), chat, app);
    assert_eq!(status, OCC_OK);
    // Fill the rest of the chat account with a host-reported allocation.
    // SAFETY: `chat` is a live handle.
    let shared = &unsafe { &*chat }.account;
    let left = 96 * 1024 * 1024 - shared.usage().retained;
    let _filled = shared
        .report_allocation(RetainedAllocation {
            requested: (left - 64) / 16 * 16,
            unused_capacity: 0,
        })
        .unwrap();
    let (status, out) = finish(reply, b"Hello <Badge>x</Badge>", NO_RECORDS);
    assert_eq!(status, OCC_REFUSED);
    assert_eq!(
        json(&out.unwrap()),
        serde_json::json!({
            "error": "refused",
            "refused": {"kind": "aggregate", "scope": "chat"},
        })
    );
    // While the first session holds its source copies, a new session cannot
    // reserve its own. A handle cannot carry error JSON, so `*out` stays
    // NULL.
    let (status, refused) = new_reply_raw(stamp.as_ptr(), layout.as_ptr(), chat, app);
    assert_eq!((status, refused), (OCC_REFUSED, ptr::null_mut()));
    free(reply);
    // SAFETY: both handles are live and freed exactly once.
    unsafe {
        crate::occ_account_free(chat);
        crate::occ_account_free(app);
    }
}

#[test]
fn an_account_of_the_wrong_scope_is_refused() {
    let (chat, app) = (account(0), account(1));
    let (stamp, layout) = (c(STAMP), c(LAYOUT));
    // SAFETY: `app` is a live handle.
    assert_eq!(unsafe { &*app }.account.scope(), AccountScope::App);
    let (status, reply) = new_reply_raw(stamp.as_ptr(), layout.as_ptr(), app, chat);
    assert_eq!((status, reply), (OCC_REFUSED, ptr::null_mut()));
    // SAFETY: both handles are live and freed exactly once.
    unsafe {
        crate::occ_account_free(chat);
        crate::occ_account_free(app);
    }
}

#[test]
fn freeing_the_accounts_leaves_the_session_working() {
    let (chat, app) = (account(0), account(1));
    let (stamp, layout) = (c(STAMP), c(LAYOUT));
    let (status, reply) = new_reply_raw(stamp.as_ptr(), layout.as_ptr(), chat, app);
    assert_eq!(status, OCC_OK);
    // SAFETY: both handles are live and freed exactly once, before the
    // session that cloned them.
    unsafe {
        crate::occ_account_free(chat);
        crate::occ_account_free(app);
    }
    assert_eq!(chunk(reply, b"Hello <Badge>x</Badge> ").0, OCC_OK);
    ok(finish(reply, b"world", NO_RECORDS));
    ok(measure_stub(reply));
    assert!(session_of(reply).committed.is_some());
    free(reply);
}

#[test]
fn freeing_null_does_nothing() {
    // SAFETY: NULL is allowed.
    unsafe { occ_reply_free(ptr::null_mut()) };
}

#[test]
fn reply_new_refuses_bad_arguments_with_out_null() {
    let (chat, app) = (account(0), account(1));
    let (stamp, layout) = (c(STAMP), c(LAYOUT));
    let invalid = (OCC_INVALID_ARGUMENT, ptr::null_mut());
    // A NULL required pointer.
    assert_eq!(
        new_reply_raw(ptr::null(), layout.as_ptr(), chat, app),
        invalid
    );
    assert_eq!(
        new_reply_raw(stamp.as_ptr(), ptr::null(), chat, app),
        invalid
    );
    assert_eq!(
        new_reply_raw(stamp.as_ptr(), layout.as_ptr(), ptr::null(), app),
        invalid
    );
    assert_eq!(
        new_reply_raw(stamp.as_ptr(), layout.as_ptr(), chat, ptr::null()),
        invalid
    );
    // SAFETY: every pointer is live; NULL is allowed for `out`.
    let status =
        unsafe { occ_reply_new(stamp.as_ptr(), layout.as_ptr(), chat, app, ptr::null_mut()) };
    assert_eq!(status, OCC_INVALID_ARGUMENT);
    // Invalid UTF-8, malformed JSON, and unknown or missing envelope members.
    let bad_utf8 = std::ffi::CString::new(&b"{\"dialect\":\"1.\xFF\"}"[..]).unwrap();
    let bad_stamps = [
        bad_utf8,
        c("{\"dialect\":"),
        c("{\"dialect\":\"1.0\",\"extra\":1}"),
        c("{}"),
        c("{\"dialect\":1}"),
    ];
    for stamp in &bad_stamps {
        assert_eq!(
            new_reply_raw(stamp.as_ptr(), layout.as_ptr(), chat, app),
            invalid,
            "{stamp:?}"
        );
    }
    let bad_layouts = [
        c(&LAYOUT.replace('}', ",\"extra\":true}")),
        c(&LAYOUT.replace("\"320\"", "320")),
        c("[]"),
    ];
    for layout in &bad_layouts {
        assert_eq!(
            new_reply_raw(stamp.as_ptr(), layout.as_ptr(), chat, app),
            invalid,
            "{layout:?}"
        );
    }
    // SAFETY: both handles are live and freed exactly once.
    unsafe {
        crate::occ_account_free(chat);
        crate::occ_account_free(app);
    }
}

#[test]
fn session_entries_refuse_bad_arguments_with_out_null() {
    let invalid = (OCC_INVALID_ARGUMENT, None);
    // A NULL session.
    assert_eq!(chunk(ptr::null_mut(), b"x"), invalid);
    assert_eq!(finish(ptr::null_mut(), b"x", NO_RECORDS), invalid);
    assert_eq!(
        event(ptr::null_mut(), r#"{"kind":"account_release"}"#),
        invalid
    );
    assert_eq!(measure_stub(ptr::null_mut()), invalid);
    let reply = new_reply();
    // Bad records leave the session unfinished.
    for records in [
        r#"{"references":{}}"#,
        r#"{"references":{},"handles":[],"extra":1}"#,
        r#"{"references":{"07":{"state":"held"}},"handles":[]}"#,
        "",
    ] {
        assert_eq!(finish(reply, b"Hello", records), invalid, "{records}");
    }
    // SAFETY: `reply` is live; the NULL records pointer is refused.
    let result = call(|out| unsafe { occ_reply_finish(reply, ptr::null(), 0, ptr::null(), out) });
    assert_eq!(result, invalid);
    ok(finish(reply, b"Hello", NO_RECORDS));
    for event_json in [
        r#"{"kind":"account_release","extra":1}"#,
        r#"{"kind":"snapshot","reference":7}"#,
        r#"{"kind":"snapshot","reference":7,"result":{"ok":"!!!!"}}"#,
        "null",
    ] {
        assert_eq!(event(reply, event_json), invalid, "{event_json}");
    }
    // A NULL metrics table, a NULL callback and a NULL layout.
    let layout = c(LAYOUT);
    // SAFETY: `reply` and `layout` are live; the NULL table is refused.
    let result = call(|out| unsafe { occ_reply_measure(reply, layout.as_ptr(), ptr::null(), out) });
    assert_eq!(result, invalid);
    let mut script = Script::default();
    let mut metrics = table(&mut script);
    metrics.t = None;
    // SAFETY: as above; the NULL callback is refused before any call.
    let result = call(|out| unsafe { occ_reply_measure(reply, layout.as_ptr(), &metrics, out) });
    assert_eq!(result, invalid);
    let metrics = table(&mut script);
    // SAFETY: as above; the NULL layout is refused.
    let result = call(|out| unsafe { occ_reply_measure(reply, ptr::null(), &metrics, out) });
    assert_eq!(result, invalid);
    assert!(session_of(reply).committed.is_none());
    free(reply);
}
