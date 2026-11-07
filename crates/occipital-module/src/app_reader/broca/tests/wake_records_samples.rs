//! The wake records samples the apps pin, `phone-reader-wake-records-v1`: what
//! core's wakes held, which the apps draw as chips instead of the prompt text
//! (`crate::notices::wakes`). Its README says what each file shows. A test
//! OpenCode 2 session on Thalamus's runner holds core's wakes, one of them
//! riding a tool result, the person's own message, and a wake whose history
//! moved here from another session. Core writes each wake's record before it
//! sends, here straight through the write handler. `served_samples` says how
//! the set is checked and rewritten.

use super::served_samples::{decoded, Served};
use super::tool_display_samples::AnyHarness;
use super::*;
use cortexkit_test_support::ScratchDir;
use subc_client_rs::HandlerOutcome;
use subc_protocol::Principal;

const CORE: &str = "prefrontal-core";
const SESSION: &str = "ses_head";
/// The session a moved wake was sent to, before its history moved here.
const BEFORE_MOVE: &str = "ses_before_move";

fn send(n: u32) -> String {
    format!("inbox-send_{n:016x}")
}

/// A user-role row core sent, or the person sent through Occipital.
fn user(ordinal: u64, sender: &str, send_id: &str, text: &str, notice: bool) -> Value {
    let mut entry = json!({"mid": format!("msg_{ordinal}"), "ordinal": ordinal, "role": "user",
        "written_at": 1_793_600_000_000_u64 + ordinal * 1_000,
        "sender": sender, "send_id": send_id, "delivered_as": {"mode": "idle_start"},
        "message": {"id": format!("msg_{ordinal}"), "type": "user", "text": text,
            "time": {"created": 1_793_600_000_000_u64 + ordinal * 1_000}}});
    if notice {
        entry["notice"] = json!(true);
    }
    entry
}

fn chat() -> Vec<Value> {
    vec![
        user(
            1,
            CORE,
            &send(1),
            "<system-reminder>\nInbox: 7 notices.\n</system-reminder>",
            true,
        ),
        json!({"mid": "msg_2", "ordinal": 2,
        "tool_times": [{"tool_call_id": "call_one", "started_at": 1_793_600_002_100_u64,
            "finished_at": 1_793_600_002_160_u64, "call_key": "ck_one"}],
        "notices": [{"part": 0, "call_key": "ck_one", "send_id": send(2), "sender": CORE,
            "text": "<system-reminder>\nInbox: 1 notice.\n</system-reminder>",
            "delivered_as": "tool_result"}],
        "message": {"id": "msg_2", "type": "assistant",
            "time": {"created": 1_793_600_002_000_u64, "completed": 1_793_600_003_000_u64},
            "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
            "content": [
                {"type": "tool", "id": "call_one", "name": "bash",
                    "state": {"status": "completed", "input": {"command": "cargo test"},
                        "content": [{"type": "text", "text": "ok\n"}]}},
                {"type": "text", "text": "The tests pass."}
            ]}}),
        user(
            3,
            CORE,
            &send(3),
            "<system-reminder>\nInbox: 40 notices.\n</system-reminder>",
            true,
        ),
        user(
            4,
            CORE,
            &send(4),
            "<system-reminder>\nInbox: 1 notice.\n</system-reminder>",
            true,
        ),
        user(
            5,
            "occipital",
            "app-send-5",
            "Please check the build.",
            false,
        ),
        user(
            6,
            CORE,
            &send(6),
            "<system-reminder>\nInbox: 1 notice.\n</system-reminder>",
            true,
        ),
    ]
}

fn item(kind: &str, refs: Value, excerpt: &str) -> Value {
    json!({"kind": kind, "refs": refs, "at": 1_793_599_990_000_u64, "excerpt": excerpt})
}

const HEAD: &str = "agent_0a1b2c3d4e5f6a7b";
const PEER: &str = "agent_7b6a5f4e3d2c1b0a";

/// A wake holding one item of every kind: a peer message, a fold of a
/// room's posts, a worker's question, a work result, a sidekick answer, an
/// approval card, and a member outside those kinds as a generic notice.
fn every_kind() -> Value {
    let mut peer = item(
        "peer_message",
        json!({"id": "pm_0123456789abcdef"}),
        "Can you review the store change before I merge it?",
    );
    peer["from"] = json!({"agent_id": PEER});
    peer["to"] = json!({"agent_id": HEAD, "display_name": "Example head"});
    json!({"items": [
        peer,
        item("room_post", json!({"room": "rm_example", "first_seq": 40, "last_seq": 44}),
            "The release candidate is tagged."),
        item("worker_question", json!({"ask": "ask_example", "task": "bg_example"}),
            "Should the cache keep entries across restarts?"),
        item("work_result", json!({"task": "bg_example", "run": "run_example", "item": "wi_example"}),
            "Delivered: the cache keeps entries for 24 hours."),
        item("sidekick_answer", json!({"id": "sk_example"}), "The retry limit is set in one place."),
        item("approval_card", json!({"id": "acc_example"}), "Approved: open the network for the build."),
        item("notice", json!({"family": "consult_result", "title": "Design review finished"}),
            "The panel agreed with the plan."),
    ]})
}

#[tokio::test]
async fn the_wake_records_samples_are_what_the_reader_serves() {
    let dir = ScratchDir::new("occipital-wake-records-samples");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, _task) = crate::notices::Notices::start(file.clone());
    let slot = crate::notices::NoticeSlot::default();
    let _ = slot.set(notices.clone());

    // Core's writes, each before its send. Message 4's send has none, as when
    // the write missed core's deadline.
    let riding = json!({"items": [item("work_result", json!({"task": "bg_example"}),
        "Delivered: the flaky test is fixed.")]});
    let cut = json!({"items": [
        item("peer_message", json!({"id": "pm_fedcba9876543210"}), "Here are the numbers you asked for."),
        item("room_post", json!({"room": "rm_example", "first_seq": 45, "last_seq": 45}), "Build is green."),
    ], "omittedItems": 38});
    let mut cut_peer = cut["items"][0].clone();
    cut_peer["from"] = json!({"agent_id": PEER});
    cut_peer["to"] = json!({"agent_id": HEAD});
    let mut cut = cut;
    cut["items"][0] = cut_peer;
    let moved = json!({"items": [item("sidekick_answer", json!({"id": "sk_moved"}),
        "Found it: the limit lives in the config file.")]});
    for (session, n, body) in [
        (SESSION, 1, every_kind()),
        (SESSION, 2, riding.clone()),
        (SESSION, 3, cut.clone()),
        (BEFORE_MOVE, 6, moved.clone()),
    ] {
        let write = json!({"kind": "wake", "runner": {"kind": "reserved",
            "module_id": THALAMUS_RUNNER_MODULE_ID}, "session": session, "send_id": send(n),
            "body": body});
        let reply = crate::notices::wakes::write(
            Some(&notices),
            Some(Principal::Reserved {
                module_id: CORE.into(),
            }),
            &write,
        )
        .await;
        assert!(
            matches!(reply, HandlerOutcome::Response(_)),
            "{n}: {reply:?}"
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
    let mut served = Served::new("phone-reader-wake-records-v1", app);
    let request = |method: &str, name: &str, value: Value| {
        let mut params = json!({"session": SESSION, "harness": "opencode2"});
        params[name] = value;
        json!({"method": method, "params": params})
    };

    // 01: the page draws core's wakes as notice rows, and carries no record.
    let page_request = request(super::super::super::TRANSCRIPT_PAGE, "limit", json!(50));
    let page = served.read(1, &page_request).await;
    let rows = decoded(&page)["result"]["messages"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 6, "{rows}");
    assert_eq!(
        rows[0]["notice"],
        json!({"sender": CORE, "deliveredAs": "idle_start"})
    );
    assert!(!String::from_utf8_lossy(&page).contains("pm_0123456789abcdef"));
    served.keep("01-page", &page_request, page);

    // 02: the records of the rows the app draws: every kind, a cut record,
    // and the moved row's from its first session. The row whose send has no
    // record, and the person's own, are left out.
    let ids = json!(["msg_1", "msg_3", "msg_4", "msg_5", "msg_6"]);
    let records_request = request(crate::notices::wakes::READ, "messageIds", ids);
    let records = served.read(2, &records_request).await;
    assert_eq!(
        decoded(&records)["result"],
        json!({"messages": {"msg_1": every_kind(), "msg_3": cut, "msg_6": moved}})
    );
    served.keep("02-wake-records", &records_request, records);

    // 03: the wake that rode the call's result carries its record.
    let notices_request = request(crate::notices::READ, "callKeys", json!(["ck_one"]));
    let mut read = Vec::new();
    for corr in 3..1_000 {
        read = served.read(corr, &notices_request).await;
        if !decoded(&read)["result"]["calls"]["ck_one"].is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        decoded(&read)["result"]["calls"]["ck_one"],
        json!([{"sender": CORE, "text": "<system-reminder>\nInbox: 1 notice.\n</system-reminder>",
            "deliveredAs": "tool_result", "wake": riding}])
    );
    served.keep("03-notices-riding-wake", &notices_request, read);

    served.check();
}
