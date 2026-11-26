//! A background command's end as the apps receive it when prefrontal's wake
//! tells the agent and AFT's own card draws it (tool-display §8.4). AFT keys
//! its notice record by a record id it never sends, and prefrontal's wake
//! record points at it as `notice_record: {writer: "aft", id}`. The app sample
//! sets `phone-reader-wake-notice-record-v1` and `-v2` are what the apps
//! receive for a chat holding such wakes: the row takes AFT's card when the
//! record qualifies, and keeps prefrontal's own card otherwise. v2 adds a wake
//! that holds a second notice beside the pointed one, which keeps its own card
//! because AFT's would hide that notice.
//!
//! AFT's records and views are its own, from `tests/data/aft-notice-1120/`
//! (see `aft_notice_1120`). Prefrontal's wake bodies take the contract's
//! example for an awaited-work wake (tool-display's `wakes.json`,
//! `awaited_work_notice_record`), with each pointer naming the AFT record of
//! its case.

use super::aft_notice_1120::{keep_aft_views, served, write, ByHarnessForAft, AFT, SESSION};
use super::served_samples::{decoded, Served};
use super::tool_display_samples::AnyHarness;
use super::*;
use crate::display::app::ToolDisplay;
use crate::display::store::DisplayStore;
use cortexkit_test_support::ScratchDir;
use subc_protocol::Principal;

const PREFRONTAL: &str = "prefrontal";

/// One wake of the chat: the row it arrives as, prefrontal's send id for it,
/// the AFT notice its wake points at, if any, and what AFT wrote for that
/// notice.
struct Case {
    mid: &'static str,
    send_id: &'static str,
    title: &'static str,
    points_at: Option<&'static str>,
    aft_wrote: Wrote,
    /// Whether the wake holds a second notice beside the one it points at.
    crowded: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Wrote {
    /// AFT's record as it wrote it.
    Record,
    /// Nothing: the pointer names a record AFT never stored.
    Nothing,
    /// AFT's record without its `command`, which its view's data schema
    /// requires, so the record can't be drawn with that view.
    Unfit,
}

/// The chat's wakes in order. `msg_1` is an answer whose tool result carried
/// the first wake; the others are their own messages.
const CASES: [Case; 6] = [
    Case {
        mid: "msg_1",
        send_id: "pf-wake-riding",
        title: "Watched command exited",
        points_at: Some("watch-task-exit"),
        aft_wrote: Wrote::Record,
        crowded: false,
    },
    Case {
        mid: "msg_2",
        send_id: "pf-wake-finished",
        title: "Background command finished",
        points_at: Some("finished"),
        aft_wrote: Wrote::Record,
        crowded: false,
    },
    Case {
        mid: "msg_3",
        send_id: "pf-wake-no-record",
        title: "Background command failed",
        points_at: Some("failed"),
        aft_wrote: Wrote::Nothing,
        crowded: false,
    },
    Case {
        mid: "msg_4",
        send_id: "pf-wake-unfit",
        title: "Background command timed out",
        points_at: Some("timed-out"),
        aft_wrote: Wrote::Unfit,
        crowded: false,
    },
    Case {
        mid: "msg_5",
        send_id: "pf-wake-watch",
        title: "Watch matched",
        points_at: Some("watch-match"),
        aft_wrote: Wrote::Record,
        crowded: false,
    },
    Case {
        mid: "msg_6",
        send_id: "pf-wake-browser",
        title: "Browser task finished",
        points_at: None,
        aft_wrote: Wrote::Nothing,
        crowded: false,
    },
];

/// v2's last wake: AFT wrote its record, but the wake holds a second notice.
const CROWDED: Case = Case {
    mid: "msg_7",
    send_id: "pf-wake-two-items",
    title: "Background command stopped",
    points_at: Some("stopped"),
    aft_wrote: Wrote::Record,
    crowded: true,
};

/// The wakes of `set`, in the chat's order.
fn cases(set: &str) -> Vec<&'static Case> {
    let mut cases: Vec<&'static Case> = CASES.iter().collect();
    if set.ends_with("-v2") {
        cases.push(&CROWDED);
    }
    cases
}

/// Prefrontal's wake write for `case`.
fn wake_write(case: &Case) -> Value {
    let mut body = json!({"items": [{"kind": "notice",
        "refs": {"family": "worker_notice", "title": case.title},
        "at": 1_793_800_000_000_u64,
        "excerpt": format!("{}.", case.title)}]});
    if case.crowded {
        body["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind": "notice",
            "refs": {"family": "worker_notice", "title": "Worker notice"},
            "at": 1_793_800_000_000_u64,
            "excerpt": "The review worker is waiting for an answer."}));
    }
    if let Some(kind) = case.points_at {
        body["notice_record"] = json!({"writer": AFT, "id": write(kind)["send_id"]});
    }
    json!({"kind": "wake",
        "runner": {"kind": "reserved", "module_id": THALAMUS_RUNNER_MODULE_ID},
        "session": SESSION, "send_id": case.send_id, "body": body})
}

/// A user-role row prefrontal sent as its own wake.
fn wake_row(ordinal: u64, case: &Case) -> Value {
    let text = format!("<system-reminder>\n{}.\n</system-reminder>", case.title);
    let at = 1_793_800_000_000_u64 + ordinal * 1_000;
    json!({"mid": case.mid, "ordinal": ordinal, "role": "user", "written_at": at,
        "sender": PREFRONTAL, "send_id": case.send_id, "delivered_as": {"mode": "idle_start"},
        "notice": true,
        "message": {"id": case.mid, "type": "user", "text": text, "time": {"created": at}}})
}

fn chat(cases: &[&Case]) -> Vec<Value> {
    let riding = cases[0];
    let mut rows = vec![json!({"mid": riding.mid, "ordinal": 1,
    "tool_times": [{"tool_call_id": "call_one", "started_at": 1_793_800_000_100_u64,
        "finished_at": 1_793_800_000_160_u64, "call_key": "ck_one"}],
    "notices": [{"part": 0, "call_key": "ck_one", "send_id": riding.send_id,
        "sender": PREFRONTAL,
        "text": format!("<system-reminder>\n{}.\n</system-reminder>", riding.title),
        "delivered_as": "tool_result"}],
    "message": {"id": riding.mid, "type": "assistant",
        "time": {"created": 1_793_800_000_000_u64, "completed": 1_793_800_001_000_u64},
        "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
        "content": [
            {"type": "tool", "id": "call_one", "name": "read",
                "state": {"status": "completed", "input": {"path": "README.md"},
                    "content": [{"type": "text", "text": "# Example\n"}]}},
            {"type": "text", "text": "The watched build ended."}
        ]}})];
    for (n, case) in cases.iter().enumerate().skip(1) {
        rows.push(wake_row(n as u64 + 1, case));
    }
    rows
}

#[tokio::test]
async fn a_wake_draws_aft_s_card_only_when_the_record_it_points_at_qualifies() {
    serve_set("phone-reader-wake-notice-record-v1").await;
}

/// A wake that holds a second notice beside the one it points at keeps its
/// own card, since AFT's card in its place would hide that notice.
#[tokio::test]
async fn a_wake_with_a_second_notice_keeps_its_own_card() {
    serve_set("phone-reader-wake-notice-record-v2").await;
}

async fn serve_set(set: &'static str) {
    let cases = cases(set);
    let dir = ScratchDir::new("occipital-wake-notice-record-1126");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, _task) = crate::notices::Notices::start(file);
    let slot = crate::notices::NoticeSlot::default();
    let _ = slot.set(notices.clone());
    let store = Arc::new(DisplayStore::open(&descriptor, main.data_root()).unwrap());
    let (_keys, _fetching) = keep_aft_views(&store).await;

    let as_module = |module: &str| {
        Some(Principal::Reserved {
            module_id: module.into(),
        })
    };
    for case in &cases {
        let outcome =
            crate::notices::wakes::write(Some(&notices), as_module(PREFRONTAL), &wake_write(case))
                .await;
        assert!(
            matches!(outcome, subc_client_rs::HandlerOutcome::Response(_)),
            "{}",
            case.mid
        );
        let Some(kind) = case.points_at else { continue };
        let mut aft = write(kind);
        match case.aft_wrote {
            Wrote::Record => {}
            Wrote::Nothing => continue,
            Wrote::Unfit => {
                aft["body"]["data"]
                    .as_object_mut()
                    .unwrap()
                    .remove("command");
            }
        }
        crate::notices::notice_records::store(Some(&notices), None, as_module(AFT), &aft)
            .await
            .unwrap();
    }

    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = chat(&cases);
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(ByHarnessForAft))),
            Arc::new(AnyHarness(route.clone())),
        )
        .with_notices(slot),
    )));
    app.set_notices(notices);
    let hints = tokio::sync::broadcast::channel(16).0;
    app.set_tool_display(Arc::new(ToolDisplay::new(store, None, hints)));
    let mut served_set = Served::new(set, app);
    let request = |method: &str, members: Value| {
        let mut params = json!({"session": SESSION, "harness": "opencode2"});
        for (name, value) in members.as_object().unwrap() {
            params[name] = value.clone();
        }
        json!({"method": method, "params": params})
    };

    // 01: the page: prefrontal's wakes, and no row of AFT's own.
    let page_request = request(super::super::super::TRANSCRIPT_PAGE, json!({"limit": 50}));
    let page = served_set.read(1, &page_request).await;
    let rows = decoded(&page)["result"]["messages"].clone();
    assert_eq!(rows.as_array().unwrap().len(), cases.len());
    assert!(!page.windows(5).any(|window| window == b"\"aft\""));
    served_set.keep("01-page", &page_request, page);

    // 02: each wake row's record, and AFT's card where the one its wake
    // points at qualifies.
    let ids: Vec<&str> = cases.iter().skip(1).map(|case| case.mid).collect();
    let asked = request(
        crate::notices::wakes::READ,
        json!({"messageIds": ids, "acceptNoticeRecords": true}),
    );
    let records = served_set.read(2, &asked).await;
    let wakes: serde_json::Map<String, Value> = cases
        .iter()
        .skip(1)
        .map(|case| (case.mid.to_owned(), wake_write(case)["body"].clone()))
        .collect();
    let cards: serde_json::Map<String, Value> = cases
        .iter()
        .skip(1)
        .filter(|case| case.aft_wrote == Wrote::Record && !case.crowded)
        .map(|case| (case.mid.to_owned(), served(case.points_at.unwrap())))
        .collect();
    assert_eq!(
        decoded(&records)["result"],
        json!({"messages": wakes, "notices": cards})
    );
    served_set.keep("02-wake-records", &asked, records);

    // 03: the same read without the opt-in: prefrontal's cards only, as
    // today's apps decode it.
    let plain = request(crate::notices::wakes::READ, json!({"messageIds": ids}));
    let records = served_set.read(3, &plain).await;
    assert_eq!(decoded(&records)["result"], json!({"messages": wakes}));
    served_set.keep("03-wake-records-without-opt-in", &plain, records);

    // 04: the wake that rode the call's result, with AFT's card.
    let riding = request(
        crate::notices::READ,
        json!({"callKeys": ["ck_one"], "acceptNoticeRecords": true}),
    );
    let mut read = Vec::new();
    for corr in 4..1_000 {
        read = served_set.read(corr, &riding).await;
        if !decoded(&read)["result"]["calls"]["ck_one"].is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let notice = decoded(&read)["result"]["calls"]["ck_one"][0].clone();
    assert_eq!(notice["wake"], wake_write(cases[0])["body"]);
    assert_eq!(notice["noticeRecord"], served("watch-task-exit"));
    served_set.keep("04-notices-riding", &riding, read);

    served_set.check();
}
