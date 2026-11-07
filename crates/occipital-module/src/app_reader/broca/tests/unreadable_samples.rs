//! The sample set `phone-reader-unreadable-v1`: an OpenCode 2 chat whose middle
//! message can't be read, beside two that can. A body the runner contract
//! refuses to project (here a compaction checkpoint, which is never served)
//! becomes the unreadable placeholder: `unreadable: true`, role `unknown`, and
//! one text part with no `partKey`, keeping its ordinal and place. The apps draw
//! a quiet marker from the flag, never from the placeholder's text.

use super::tool_display_samples::AnyHarness;
use super::*;

fn entries() -> Vec<Value> {
    vec![
        json!({"mid": "msg_a", "ordinal": 1, "written_at": 1_000,
            "message": {"id": "msg_a", "type": "user", "time": {"created": 1_000_u64},
                "content": [{"type": "text", "text": "Show me the plan."}]}}),
        json!({"mid": "msg_b", "ordinal": 2, "written_at": 2_000,
            "message": {"id": "msg_b", "type": "compaction", "content": []}}),
        json!({"mid": "msg_c", "ordinal": 3, "written_at": 3_000,
            "message": {"id": "msg_c", "type": "assistant",
                "time": {"created": 2_500_u64, "completed": 3_000_u64},
                "content": [{"type": "text", "text": "Here it is."}]}}),
    ]
}

#[tokio::test]
async fn the_unreadable_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries();
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::Hosts(
                THALAMUS_RUNNER_MODULE_ID,
                "opencode2",
            )))),
            Arc::new(AnyHarness(route)),
        ),
    )));
    let mut served = super::served_samples::Served::new("phone-reader-unreadable-v1", app);
    let session = json!({"session": "ses_head", "harness": "opencode2"});

    let mut params = session.clone();
    params["limit"] = json!(50);
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": params});
    let reply = served.read(1, &request).await;
    let page = super::served_samples::decoded(&reply)["result"].clone();
    let messages = page["messages"].as_array().unwrap();
    let shown: Vec<_> = messages
        .iter()
        .map(|message| {
            (
                message["ordinal"].clone(),
                message.get("unreadable").cloned(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            (json!(1), None),
            (json!(2), Some(json!(true))),
            (json!(3), None)
        ],
        "{page}"
    );
    let unreadable = &messages[1];
    assert_eq!(unreadable["role"], "unknown");
    assert!(
        unreadable["parts"][0].get("partKey").is_none(),
        "{unreadable}"
    );
    served.keep("01-newest-page", &request, reply);

    let mut params = session;
    params["messageOrdinal"] = json!(2);
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": params});
    let reply = served.read(2, &request).await;
    let page = super::served_samples::decoded(&reply)["result"].clone();
    assert_eq!(page["messages"][0]["unreadable"], true, "{page}");
    served.keep("02-exact-read", &request, reply);
    served.check();
}

/// The sample set `phone-reader-unreadable-part-v1`: an OpenCode 2 chat whose
/// messages each hold one part the runner contract can't show (runner
/// contract 0.8.22). The person's message keeps its text, and its photo,
/// whose stored data isn't base64, becomes an `unreadable` part in its place,
/// adding nothing to the row's text. The answer's tool result, which held such
/// a file, keeps its text and its size.
fn bad_parts() -> Vec<Value> {
    vec![
        json!({"mid": "msg_a", "ordinal": 1, "written_at": 1_000,
            "message": {"id": "msg_a", "type": "user", "time": {"created": 1_000_u64},
                "text": "Here is the screenshot.",
                "files": [{"mime": "image/png", "data": "not base64!"}]}}),
        json!({"mid": "msg_b", "ordinal": 2, "written_at": 3_000,
        "message": {"id": "msg_b", "type": "assistant",
            "time": {"created": 2_000_u64, "completed": 3_000_u64},
            "content": [
                {"type": "tool", "id": "call_one", "name": "read",
                    "state": {"status": "completed", "input": {"path": "shot.png"},
                        "content": [{"type": "text", "text": "An image of 1200x800."},
                            {"type": "file", "mime": "image/png", "data": "not base64!"}]}},
                {"type": "text", "text": "The screenshot shows the error."}
            ]}}),
    ]
}

#[tokio::test]
async fn the_unreadable_part_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = bad_parts();
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::Hosts(
                THALAMUS_RUNNER_MODULE_ID,
                "opencode2",
            )))),
            Arc::new(AnyHarness(route)),
        ),
    )));
    let mut served = super::served_samples::Served::new("phone-reader-unreadable-part-v1", app);
    let session = json!({"session": "ses_head", "harness": "opencode2"});

    let mut params = session.clone();
    params["limit"] = json!(50);
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": params});
    let reply = served.read(1, &request).await;
    assert!(!String::from_utf8_lossy(&reply).contains("not base64!"));
    let page = super::served_samples::decoded(&reply)["result"].clone();
    let messages = page["messages"].as_array().unwrap();
    assert!(
        messages
            .iter()
            .all(|message| message.get("unreadable").is_none()),
        "{page}"
    );
    let person = &messages[0];
    assert_eq!(person["text"], "Here is the screenshot.");
    let kinds: Vec<&str> = person["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| part["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["text", "unreadable"], "{person}");
    let tool = &messages[1]["parts"][0];
    assert_eq!(tool["type"], "tool", "{tool}");
    assert_eq!(tool["outputBytes"], "An image of 1200x800.".len());
    served.keep("01-newest-page", &request, reply);

    let mut params = session;
    params["messageOrdinal"] = json!(2);
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": params});
    let reply = served.read(2, &request).await;
    let page = super::served_samples::decoded(&reply)["result"].clone();
    assert_eq!(
        page["messages"][0]["parts"][0]["content"],
        "An image of 1200x800."
    );
    served.keep("02-exact-read", &request, reply);
    served.check();
}
