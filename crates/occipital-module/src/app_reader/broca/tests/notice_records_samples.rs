//! The notice records samples the apps pin, `phone-reader-notice-records-v1`:
//! the data a module writes for its own notices, which the apps draw as a card
//! from that module's notice view instead of the folded notice text
//! (`crate::notices::notice_records`, tool-display §10). Its README says what
//! each file shows. A test OpenCode 2 session on Thalamus's runner holds AFT's
//! notices about background commands, one of them riding a tool result. AFT
//! writes each notice's record before it sends, here straight through the
//! write handler, and its notice views are kept as if fetched. `served_samples`
//! says how the set is checked and rewritten.

use super::served_samples::{decoded, Served};
use super::tool_display_samples::{template, AnyHarness};
use super::*;
use crate::display::app::{ToolDisplay, TEMPLATE, VIEWS};
use crate::display::store::{DisplayStore, ViewState};
use cortexkit_role_tool_provider::views::{NoticeView, NoticeViewDefinition, NoticeViewTemplates};
use cortexkit_test_support::ScratchDir;
use subc_protocol::Principal;

const AFT: &str = "aft";
const SESSION: &str = "ses_head";
/// The session a moved notice was sent to, before its history moved here.
const BEFORE_MOVE: &str = "ses_before_move";

fn send(n: u32) -> String {
    format!("aft-notice-{n}")
}

/// A user-role row AFT sent as its own notice.
fn notice_row(ordinal: u64, text: &str) -> Value {
    json!({"mid": format!("msg_{ordinal}"), "ordinal": ordinal, "role": "user",
        "written_at": 1_793_700_000_000_u64 + ordinal * 1_000,
        "sender": AFT, "send_id": send(ordinal as u32), "delivered_as": {"mode": "idle_start"},
        "notice": true,
        "message": {"id": format!("msg_{ordinal}"), "type": "user", "text": text,
            "time": {"created": 1_793_700_000_000_u64 + ordinal * 1_000}}})
}

fn reminder(text: &str) -> String {
    format!("<system-reminder>\n{text}\n</system-reminder>")
}

fn chat() -> Vec<Value> {
    vec![
        notice_row(
            1,
            &reminder("Background command finished: cargo test (exit 0)."),
        ),
        json!({"mid": "msg_2", "ordinal": 2,
        "tool_times": [{"tool_call_id": "call_one", "started_at": 1_793_700_002_100_u64,
            "finished_at": 1_793_700_002_160_u64, "call_key": "ck_one"}],
        "notices": [{"part": 0, "call_key": "ck_one", "send_id": send(2), "sender": AFT,
            "text": reminder("Background command still running: cargo build (5 min)."),
            "delivered_as": "tool_result"}],
        "message": {"id": "msg_2", "type": "assistant",
            "time": {"created": 1_793_700_002_000_u64, "completed": 1_793_700_003_000_u64},
            "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
            "content": [
                {"type": "tool", "id": "call_one", "name": "read",
                    "state": {"status": "completed", "input": {"path": "README.md"},
                        "content": [{"type": "text", "text": "# Example\n"}]}},
                {"type": "text", "text": "The build is still going."}
            ]}}),
        notice_row(
            3,
            &reminder("Background command failed: cargo clippy (exit 101)."),
        ),
        notice_row(
            4,
            &reminder("Watch matched: \"error[E0308]\" in the build output."),
        ),
        notice_row(
            5,
            &reminder("Background command still running: cargo build --release (10 min)."),
        ),
        notice_row(
            6,
            &reminder("Background command finished: cargo build --release (exit 0)."),
        ),
        notice_row(
            7,
            &reminder("Background command finished: npm test (exit 0)."),
        ),
        notice_row(8, &reminder("Background command paused.")),
        notice_row(9, &reminder("Background command finished: make (exit 0).")),
    ]
}

fn finished(status: &str, exit_code: i64, title: &str) -> Value {
    json!({"status": status, "exit_code": exit_code, "title": title})
}

/// What AFT writes for each notice: its view, fold key and data. Notice 9's
/// write missed AFT's deadline, so it has none. Notice 8 names a view AFT
/// doesn't serve.
fn records() -> Vec<(&'static str, u32, Value)> {
    let body = |view: &str, key: Option<&str>, data: Value| {
        let mut body = json!({"view": view, "data": data});
        if let Some(key) = key {
            body["key"] = json!(key);
        }
        body
    };
    vec![
        (
            SESSION,
            1,
            body(
                "finished:1",
                Some("task-bg_1"),
                finished("finished", 0, "cargo test"),
            ),
        ),
        (
            SESSION,
            2,
            body(
                "still_running:1",
                Some("task-bg_2"),
                json!({"elapsed_seconds": 300, "title": "cargo build"}),
            ),
        ),
        (
            SESSION,
            3,
            body(
                "finished:1",
                Some("task-bg_3"),
                finished("failed", 101, "cargo clippy"),
            ),
        ),
        (
            SESSION,
            4,
            body(
                "watch:1",
                None,
                json!({"matched_text": "error[E0308]", "title": "build output"}),
            ),
        ),
        (
            SESSION,
            5,
            body(
                "still_running:1",
                Some("task-bg_5"),
                json!({"elapsed_seconds": 600, "title": "cargo build --release"}),
            ),
        ),
        (
            SESSION,
            6,
            body(
                "finished:1",
                Some("task-bg_5"),
                finished("finished", 0, "cargo build --release"),
            ),
        ),
        (
            BEFORE_MOVE,
            7,
            body(
                "finished:1",
                Some("task-bg_7"),
                finished("finished", 0, "npm test"),
            ),
        ),
        (
            SESSION,
            8,
            body(
                "paused:1",
                Some("task-bg_8"),
                json!({"title": "cargo bench"}),
            ),
        ),
    ]
}

const FINISHED_ROW: &str =
    "<ToolHeader icon=\"terminal\" title={input.title} failed={input.status == \"failed\"} />\n";
const FINISHED_PAGE: &str =
    "<TextContent text={input.title + \" exited with \" + Text(input.exit_code)} />\n";
const RUNNING_ROW: &str = "<ToolHeader icon=\"terminal\" title={input.title} />\n";
const RUNNING_PAGE: &str = "<TextContent text={input.title + \" has run for \" + Text(input.elapsed_seconds) + \" s\"} />\n";
const WATCH_ROW: &str = "<ToolHeader icon=\"terminal\" title={input.matched_text} />\n";
const WATCH_PAGE: &str = "<TextContent text={input.title} />\n";

fn notice_view(name: &str, properties: Value, row: &str, page: &str) -> NoticeView {
    let required: Vec<&String> = properties.as_object().unwrap().keys().collect();
    NoticeView::new(
        NoticeViewDefinition::new(
            name,
            1,
            json!({"type": "object", "properties": properties, "required": required,
                "additionalProperties": false}),
        ),
        NoticeViewTemplates::new(template("tool_row", row), template("tool_page", page)),
        1,
    )
}

/// AFT's notice views, kept as if fetched, and its `unknown` for the view it
/// doesn't serve.
fn seed_views(store: &DisplayStore) {
    let views = [
        notice_view(
            "finished",
            json!({"status": {"type": "string", "enum": ["finished", "failed"]},
                "exit_code": {"type": "integer"}, "title": {"type": "string"}}),
            FINISHED_ROW,
            FINISHED_PAGE,
        ),
        notice_view(
            "still_running",
            json!({"elapsed_seconds": {"type": "integer"}, "title": {"type": "string"}}),
            RUNNING_ROW,
            RUNNING_PAGE,
        ),
        notice_view(
            "watch",
            json!({"matched_text": {"type": "string"}, "title": {"type": "string"}}),
            WATCH_ROW,
            WATCH_PAGE,
        ),
    ];
    for view in views {
        let key = format!("nv1:{AFT}:{}:1", view.notice.name);
        let state = ViewState::View {
            manifest: serde_jcs::to_vec(&view).unwrap(),
            input_schema: serde_jcs::to_vec(&view.notice.data_schema).unwrap(),
        };
        store.keep_view(AFT, &key, &state, "1.0", 1).unwrap();
    }
    store
        .keep_view(AFT, "nv1:aft:paused:1", &ViewState::Unknown, "1.0", 1)
        .unwrap();
}

/// The record as the apps are served it, for the notice `n` wrote.
fn served_record(n: u32) -> Value {
    let (_, _, body) = records().into_iter().find(|(_, m, _)| *m == n).unwrap();
    let (name, version) = body["view"].as_str().unwrap().split_once(':').unwrap();
    let mut record = json!({"sender": AFT, "view": format!("nv1:{AFT}:{name}:{version}"),
        "data": body["data"]});
    if let Some(key) = body.get("key") {
        record["key"] = key.clone();
    }
    record
}

#[tokio::test]
async fn the_notice_records_samples_are_what_the_reader_serves() {
    let dir = ScratchDir::new("occipital-notice-records-samples");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, _task) = crate::notices::Notices::start(file);
    let slot = crate::notices::NoticeSlot::default();
    let _ = slot.set(notices.clone());
    let store = Arc::new(DisplayStore::open(&descriptor, main.data_root()).unwrap());
    seed_views(&store);

    for (session, n, body) in records() {
        let write = json!({"kind": "notice", "runner": {"kind": "reserved",
            "module_id": THALAMUS_RUNNER_MODULE_ID}, "session": session, "send_id": send(n),
            "body": body});
        let reply = crate::notices::notice_records::store(
            Some(&notices),
            None,
            Some(Principal::Reserved {
                module_id: AFT.into(),
            }),
            &write,
        )
        .await;
        assert_eq!(
            reply,
            Ok(cortexkit_role_tool_display::WriteReply::Stored),
            "{n}"
        );
    }

    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = chat();
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                THALAMUS_RUNNER_MODULE_ID,
            ))))),
            Arc::new(AnyHarness(route.clone())),
        )
        .with_notices(slot),
    )));
    app.set_notices(notices);
    let hints = tokio::sync::broadcast::channel(16).0;
    app.set_tool_display(Arc::new(ToolDisplay::new(store, None, hints)));
    let mut served = Served::new("phone-reader-notice-records-v1", app);
    let request = |method: &str, members: Value| {
        let mut params = json!({"session": SESSION, "harness": "opencode2"});
        for (name, value) in members.as_object().unwrap() {
            params[name] = value.clone();
        }
        json!({"method": method, "params": params})
    };

    // 01: the page draws AFT's notices as notice rows, and carries no record.
    let page_request = request(super::super::super::TRANSCRIPT_PAGE, json!({"limit": 50}));
    let page = served.read(1, &page_request).await;
    let rows = decoded(&page)["result"]["messages"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 9, "{rows}");
    assert!(!String::from_utf8_lossy(&page).contains("task-bg_1"));
    served.keep("01-page", &page_request, page);

    // 02: the records of the rows the app draws, asked for. Each record whose
    // view AFT serves joins; the moved row takes the record sent to its first
    // session; notice 8 names a view AFT doesn't serve and notice 9 has no
    // record, so both are left out and draw the folded notice.
    let ids = json!(["msg_1", "msg_3", "msg_4", "msg_5", "msg_6", "msg_7", "msg_8", "msg_9"]);
    let asked = request(
        crate::notices::wakes::READ,
        json!({"messageIds": ids, "acceptNoticeRecords": true}),
    );
    let records = served.read(2, &asked).await;
    let expected: serde_json::Map<String, Value> = [1, 3, 4, 5, 6, 7]
        .into_iter()
        .map(|n| (format!("msg_{n}"), served_record(n)))
        .collect();
    assert_eq!(
        decoded(&records)["result"],
        json!({"messages": {}, "notices": expected})
    );
    served.keep("02-wake-records-with-notices", &asked, records);

    // 03: without the opt-in the reply is the one older apps decode.
    let plain = request(crate::notices::wakes::READ, json!({"messageIds": ids}));
    let reply = served.read(3, &plain).await;
    assert_eq!(decoded(&reply)["result"], json!({"messages": {}}));
    served.keep("03-wake-records-without-opt-in", &plain, reply);

    // 04: the notice that rode the call's result carries its send's record.
    let riding = request(
        crate::notices::READ,
        json!({"callKeys": ["ck_one"], "acceptNoticeRecords": true}),
    );
    let mut read = Vec::new();
    for corr in 4..1_000 {
        read = served.read(corr, &riding).await;
        if !decoded(&read)["result"]["calls"]["ck_one"].is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        decoded(&read)["result"]["calls"]["ck_one"],
        json!([{"sender": AFT,
            "text": reminder("Background command still running: cargo build (5 min)."),
            "deliveredAs": "tool_result", "noticeRecord": served_record(2)}])
    );
    served.keep("04-notices-riding", &riding, read);

    // 05: the notice views by key: AFT's three, its `unknown` for the view
    // it doesn't serve, and a key naming another module, which only that
    // module could serve.
    let pins = json!([
        "nv1:aft:finished:1",
        "nv1:aft:still_running:1",
        "nv1:aft:watch:1",
        "nv1:aft:paused:1",
        "nv1:cerebellum:finished:1"
    ]);
    let views_request = json!({"method": VIEWS, "params": {"pins": pins}});
    let views = served.read(1_001, &views_request).await;
    let answers = decoded(&views)["result"]["views"].clone();
    for key in [
        "nv1:aft:finished:1",
        "nv1:aft:still_running:1",
        "nv1:aft:watch:1",
    ] {
        let answer = &answers[key];
        assert_eq!(answer["provider"], AFT, "{key}: {answer}");
        assert!(
            answer["row"].is_string() && answer["page"].is_string(),
            "{key}: {answer}"
        );
    }
    assert_eq!(answers["nv1:aft:paused:1"], "unknown");
    assert_eq!(answers["nv1:cerebellum:finished:1"], "unavailable");
    served.keep("05-notice-views", &views_request, views);

    // 06: a notice view's row template by digest, unchanged.
    let digest = answers["nv1:aft:finished:1"]["row"].clone();
    let template_request = json!({"method": TEMPLATE, "params": {"digest": digest}});
    let document = served.read(1_002, &template_request).await;
    assert_eq!(
        decoded(&document)["result"]["template"]["source"],
        FINISHED_ROW
    );
    served.keep("06-notice-template", &template_request, document);

    served.check();
}
