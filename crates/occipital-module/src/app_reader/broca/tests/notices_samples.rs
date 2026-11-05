//! The notices samples the apps pin, `phone-reader-notices-v1`: the notices
//! modules sent the agent that rode its tool results (llm-runner's `notices`
//! group), which the apps show under the call that carried them. Its README
//! says what each file shows. A test OpenCode 2 session on Thalamus's runner
//! serves three calls; its read lists three notices in the `notices` display
//! fact, one from a module the apps don't know. A fourth notice arrives on the
//! runner's live stream after the page was read. `served_samples` says how the
//! set is checked and rewritten.

use super::served_samples::{decoded, Served};
use super::tool_display_samples::AnyHarness;
use super::*;
use cortexkit_test_support::ScratchDir;
use subc_protocol::FrameType;

const CONTEXT_NOTICE: &str = "Context is at 62%. Drop tool outputs you no longer need.";
const STATUS_NOTICE: &str = "[WK R1]";
const LATER_NOTICE: &str = "Context is at 71%. Summaries cover messages up to 40.";
/// A module the apps have no name for, shown by its bare id.
const UNKNOWN_SENDER: &str = "example-notifier";

/// One OpenCode 2 answer with three `bash` calls, each served with its key.
/// Its `notices` fact lists one notice on the first call and two from
/// different modules on the second, in delivery order; the third has none yet.
fn answer() -> Vec<Value> {
    let call = |id: &str, command: &str| {
        json!({"type": "tool", "id": id, "name": "bash",
            "state": {"status": "completed", "input": {"command": command},
                "content": [{"type": "text", "text": "ok\n"}]}})
    };
    let times = |id: &str, key: &str, at: u64| json!({"tool_call_id": id, "started_at": at, "finished_at": at + 40, "call_key": key});
    let notice = |part: u32, key: &str, send: &str, sender: &str, text: &str| {
        json!({"part": part, "call_key": key, "send_id": send, "sender": sender,
            "text": text, "delivered_as": "tool_result"})
    };
    vec![json!({"mid": "msg_a", "ordinal": 1,
    "tool_times": [
        times("call_one", "ck_one", 1_793_500_001_000),
        times("call_two", "ck_two", 1_793_500_002_000),
        times("call_three", "ck_three", 1_793_500_003_000)
    ],
    "notices": [
        notice(0, "ck_one", "send-1", "magic-context", CONTEXT_NOTICE),
        notice(1, "ck_two", "send-2", "prefrontal-core", STATUS_NOTICE),
        notice(1, "ck_two", "send-3", UNKNOWN_SENDER, CONTEXT_NOTICE)
    ],
    "message": {"id": "msg_a", "type": "assistant",
        "time": {"created": 1_793_500_000_400_u64, "completed": 1_793_500_009_000_u64},
        "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
        "content": [
            call("call_one", "cargo build"),
            call("call_two", "cargo test"),
            call("call_three", "git status"),
            {"type": "text", "text": "Done."}
        ]}})]
}

#[tokio::test]
async fn the_notices_samples_are_what_the_reader_serves() {
    let dir = ScratchDir::new("occipital-notices-samples");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap();
    let file = Arc::new(file);
    let (notices, _task) = crate::notices::Notices::start(file.clone());
    let slot = crate::notices::NoticeSlot::default();
    let _ = slot.set(notices.clone());

    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = answer();
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                OPENCODE_RUNNER_MODULE_ID,
            ))))),
            Arc::new(AnyHarness(route.clone())),
        )
        .with_notices(slot),
    )));
    app.set_notices(notices);
    let mut served = Served::new("phone-reader-notices-v1", app);
    let session = json!({"session": "ses_head", "harness": "opencode2"});
    let request = |method: &str, extra: Value| {
        let mut params = session.clone();
        for (name, value) in extra.as_object().unwrap() {
            params[name] = value.clone();
        }
        json!({"method": method, "params": params})
    };
    let calls = |reply: &[u8]| decoded(reply)["result"]["calls"].clone();

    // 01: the page names each call by its key; the notices aren't on it.
    let page_request = request(super::super::super::TRANSCRIPT_PAGE, json!({"limit": 50}));
    let page = served.read(1, &page_request).await;
    let parts = &decoded(&page)["result"]["messages"][0]["parts"];
    assert_eq!(parts[1]["callKey"], "ck_two", "{parts}");
    assert!(!String::from_utf8_lossy(&page).contains(STATUS_NOTICE));
    served.keep("01-page", &page_request, page);
    for _ in 0..1_000 {
        if file
            .for_call(OPENCODE_RUNNER_MODULE_ID, "ses_head", "ck_two")
            .unwrap()
            .len()
            == 2
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    // 02: one notice on the first call, two from different modules on the
    // second in delivery order, and the third call, which none rode, left out.
    let read_request = request(
        crate::notices::READ,
        json!({"callKeys": ["ck_one", "ck_two", "ck_three"]}),
    );
    let read = served.read(2, &read_request).await;
    assert_eq!(
        calls(&read),
        json!({
            "ck_one": [{"sender": "magic-context", "text": CONTEXT_NOTICE, "deliveredAs": "tool_result"}],
            "ck_two": [
                {"sender": "prefrontal-core", "text": STATUS_NOTICE, "deliveredAs": "tool_result"},
                {"sender": UNKNOWN_SENDER, "text": CONTEXT_NOTICE, "deliveredAs": "tool_result"}
            ]
        })
    );
    served.keep("02-notices", &read_request, read);

    // 03 and 04: the watch listens, then names the third call when a notice
    // rides it on the runner's live stream.
    let watch_request = request(crate::notices::WATCH, json!({}));
    served.send(3, &watch_request).await;
    let (ty, open) = served.next(3, Duration::from_secs(5)).await.unwrap();
    assert_eq!(ty, FrameType::StreamData);
    assert_eq!(decoded(&open), json!({"callKeys": []}));
    route
        .emit(
            json!({"kind": "control", "event": {"type": "notice_delivered",
            "mid": "msg_a", "part": 2, "call_key": "ck_three", "send_id": "send-4",
            "sender": "magic-context", "text": LATER_NOTICE}}),
        )
        .await;
    let (_, hint) = served.next(3, Duration::from_secs(5)).await.unwrap();
    assert_eq!(decoded(&hint), json!({"callKeys": ["ck_three"]}));
    served.cancel(3);
    served.keep("03-notices-watch-open", &watch_request, open);
    served.keep("04-notices-watch-hint", &watch_request, hint);

    // 05: read again, the third call has the notice that arrived.
    let again_request = request(crate::notices::READ, json!({"callKeys": ["ck_three"]}));
    let again = served.read(5, &again_request).await;
    assert_eq!(
        calls(&again),
        json!({"ck_three": [{"sender": "magic-context", "text": LATER_NOTICE, "deliveredAs": "tool_result"}]})
    );
    served.keep("05-notices-after-hint", &again_request, again);

    served.check();
}
