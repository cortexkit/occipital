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
