use super::*;
use crate::app_reader::tests::serve;
use crate::app_reader::{TRANSCRIPT_PAGE, TRANSCRIPT_WATCH};
use cortexkit_role_llm_runner::display::{DisplayEntry, DisplayRow};

fn history_page(messages: Vec<DisplayEntry>) -> Value {
    let value = json!({"messages":messages, "lineage_id":"cut-a", "history_version":7});
    let page: DisplayPage = serde_json::from_value(value).unwrap();
    serde_json::to_value(page).unwrap()
}

fn route() -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["history_display"]));
    let entries: Vec<_> = ["user", "assistant", "user", "tool", "system", "user"]
        .into_iter()
        .enumerate()
        .map(|(index, role)| {
            let ordinal = 20 + index as u64;
            json!({"mid":format!("m{ordinal}"), "ordinal":ordinal, "written_at":705 + index,
                "message":{"role":role, "content":[{"type":"text", "text":format!("raw {ordinal}")}]}})
        })
        .collect();
    for chunk in entries.chunks(2) {
        let messages = chunk
            .iter()
            .map(|entry| {
                let ordinal = entry["ordinal"].as_u64().unwrap();
                // Different display content must not replace the raw message content.
                let mut row = DisplayRow::new(entry["message"]["role"].as_str().unwrap(), vec![]);
                if ordinal != 25 {
                    row = row.with_client_message_id(format!("phone-{ordinal}"));
                }
                DisplayEntry::new(entry["mid"].as_str().unwrap(), 999_999, row)
                    .with_ordinal(ordinal)
            })
            .collect();
        route.history_replies.lock().unwrap().insert(
            chunk[0]["ordinal"].as_u64().unwrap(),
            Ok(history_page(messages)),
        );
    }
    *route.entries.lock().unwrap() = entries;
    route
}

fn requests(route: &FakeRoute) -> Vec<Value> {
    route
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == "session.history" && call["params"]["form"] == "display")
        .map(|call| call["params"].clone())
        .collect()
}

fn page_params() -> Value {
    json!({"session":"alfonso:s1", "harness":"broca", "limit":6})
}

async fn page_bytes(route: Arc<FakeRoute>) -> Vec<u8> {
    let (_, app) = serving(route);
    serve(&app, TRANSCRIPT_PAGE, &page_params()).await
}

async fn baseline() -> Vec<u8> {
    let route = route();
    *route.describe.lock().unwrap() = Some(describe_with(&[]));
    page_bytes(route).await
}

fn messages(reply: &Value) -> &[Value] {
    let body = reply.get("result").unwrap_or(reply);
    body["messages"]
        .as_array()
        .or_else(|| body["newTurns"].as_array())
        .unwrap()
}

fn assert_echoes(reply: &Value) {
    let rows = messages(reply);
    assert!(!rows.is_empty(), "{reply}");
    for row in rows {
        match row["messageId"].as_str().unwrap() {
            "m20" => assert_eq!(row["clientMessageId"], "phone-20"),
            "m22" => assert_eq!(row["clientMessageId"], "phone-22"),
            _ => assert!(row.get("clientMessageId").is_none(), "{row}"),
        }
    }
}

#[tokio::test]
async fn display_history_echoes_only_matching_user_client_ids() {
    let route = route();
    let (_, app) = serving(route.clone());
    let page = app.read(TRANSCRIPT_PAGE, page_params()).await.unwrap();
    assert_eq!(messages(&page).len(), 6);
    assert_echoes(&page);

    let mut without_echo = page.clone();
    for row in without_echo["messages"].as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("clientMessageId");
    }
    let baseline: Value = serde_json::from_slice(&baseline().await).unwrap();
    assert_eq!(without_echo, baseline["result"], "only the echo may change");

    let older = app
        .read(
            TRANSCRIPT_PAGE,
            json!({"session":"alfonso:s1", "beforeOrdinal":22}),
        )
        .await
        .unwrap();
    assert_echoes(&older);
    let exact = app
        .read(
            TRANSCRIPT_PAGE,
            json!({"session":"alfonso:s1", "messageOrdinal":20}),
        )
        .await
        .unwrap();
    assert_echoes(&exact);
    let mut claim = page_params();
    claim["claim"] =
        json!({"heldFloorMessageId":"m20", "sinceMessageId":"m21", "heldVersion":page["version"]});
    let delta = app.read(TRANSCRIPT_PAGE, claim.clone()).await.unwrap();
    assert_echoes(&delta);
    let watch: Value =
        serde_json::from_slice(&serve(&app, TRANSCRIPT_WATCH, &claim).await).unwrap();
    assert_echoes(&watch);
    claim["claim"]["heldVersion"] = json!("expired-version");
    let resync = app.read(TRANSCRIPT_PAGE, claim).await.unwrap();
    assert_eq!(resync["claim"], "cursor_expired");
    assert_echoes(&resync["resync"]);
    assert!(!requests(&route).is_empty());
}

/// An OpenCode 2 entry names its role in `message.type`, not `message.role`: the
/// echo still finds its user rows and takes the id from display history.
#[tokio::test]
async fn display_history_echoes_onto_opencode_2_shaped_user_entries() {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["history_display"]));
    let row = DisplayRow::new("user", vec![]).with_client_message_id("phone-18");
    route.history_replies.lock().unwrap().insert(
        18,
        Ok(history_page(vec![
            DisplayEntry::new("msg_u", 999_999, row).with_ordinal(18),
            DisplayEntry::new("msg_a", 999_999, DisplayRow::new("assistant", vec![]))
                .with_ordinal(19),
        ])),
    );
    let (sources, _) = serving(route.clone());
    let source = sources.source(&resolved().reference).await.unwrap();
    let mut entries = vec![
        json!({"mid":"msg_u", "ordinal":18, "written_at":1,
            "message":{"id":"msg_u", "type":"user", "text":"hi", "time":{"created":1}}}),
        json!({"mid":"msg_a", "ordinal":19, "written_at":2,
            "message":{"id":"msg_a", "type":"assistant", "content":[]}}),
    ];
    source.echo_client_message_ids(&mut entries).await;
    assert_eq!(entries[0]["client_message_id"], "phone-18", "{entries:?}");
    assert!(entries[1].get("client_message_id").is_none());
    assert_eq!(requests(&route).len(), 1);
}

/// A runner declaring `read_display_facts` is asked for them on every `session.read`,
/// and serves the client ids and tool times beside each entry's `mid`, so the reader
/// makes no display-history read: one read per page.
#[tokio::test]
async fn display_facts_on_the_read_replace_the_display_history_read() {
    let route = route();
    *route.describe.lock().unwrap() =
        Some(describe_with(&["history_display", "read_display_facts"]));
    {
        let mut entries = route.entries.lock().unwrap();
        for entry in entries.iter_mut() {
            let ordinal = entry["ordinal"].as_u64().unwrap();
            if entry["message"]["role"] == "user" && ordinal != 25 {
                entry["client_message_id"] = json!(format!("phone-{ordinal}"));
            }
        }
        entries[1]["message"]["content"] = json!([
            {"type": "tool_call", "tool_call_id": "call_ls", "name": "bash", "input": {"command": "ls"}},
            {"type": "tool_call", "tool_call_id": "call_stop", "name": "bash", "input": {"command": "sleep 9"}}
        ]);
        entries[1]["tool_times"] = json!([
            {"tool_call_id": "call_ls", "started_at": 1000, "finished_at": 1200},
            {"tool_call_id": "call_stop", "started_at": 1300, "unanswered": "cancelled"}
        ]);
    }
    let (_, app) = serving(route.clone());
    let page = app.read(TRANSCRIPT_PAGE, page_params()).await.unwrap();
    assert_echoes(&page);
    let tools: Vec<_> = messages(&page)[1]["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| {
            (
                part["startedAtMs"].clone(),
                part["finishedAtMs"].clone(),
                part["unanswered"].clone(),
            )
        })
        .collect();
    assert_eq!(
        tools,
        [
            (json!(1000), json!(1200), Value::Null),
            (json!(1300), Value::Null, json!("cancelled")),
        ]
    );
    assert!(requests(&route).is_empty(), "{:?}", requests(&route));
    // Every range read that builds rows asks for the facts. The one-message tail
    // read that only finds the newest ordinal builds no rows, so it doesn't.
    let reads: Vec<_> = route
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| {
            call["method"] == "session.read" && call["params"].get("from_ordinal").is_some()
        })
        .map(|call| call["params"]["include_display_facts"].clone())
        .collect();
    assert!(!reads.is_empty());
    assert!(reads.iter().all(|flag| *flag == true), "{reads:?}");
}

#[tokio::test]
async fn display_history_is_not_called_without_the_capability() {
    let route = route();
    *route.describe.lock().unwrap() = Some(describe_with(&[]));
    assert_eq!(page_bytes(route.clone()).await, baseline().await);
    assert!(requests(&route).is_empty());
}

#[tokio::test]
async fn display_history_is_not_called_without_user_rows() {
    let route = route();
    for entry in route.entries.lock().unwrap().iter_mut() {
        entry["message"]["role"] = json!("assistant");
    }
    page_bytes(route.clone()).await;
    assert!(requests(&route).is_empty());
}

#[tokio::test]
async fn display_history_is_not_called_if_any_entry_lacks_written_at() {
    let route = route();
    {
        let mut entries = route.entries.lock().unwrap();
        entries.truncate(2);
        entries[1].as_object_mut().unwrap().remove("written_at");
        // Another timestamp is not enough to make Broca's display range representable.
        entries[1]["created_at_ms"] = json!(706);
    }
    route.history_replies.lock().unwrap().insert(
        20,
        Err(ReadError::new(
            "history_unrepresentable",
            "display range lacks a write time",
        )),
    );
    let page = page_bytes(route.clone()).await;
    let reply: Value = serde_json::from_slice(&page).unwrap();
    assert_eq!(messages(&reply).len(), 2);
    assert!(messages(&reply)[0].get("clientMessageId").is_none());
    assert_eq!(
        requests(&route).len(),
        0,
        "unrepresentable range was requested"
    );
}

#[tokio::test]
async fn display_history_refusal_preserves_the_served_page_bytes() {
    let route = route();
    for reply in route.history_replies.lock().unwrap().values_mut() {
        *reply = Err(
            ReadError::new("history_unrepresentable", "no write time for display row")
                .with_detail(json!({"ordinal":21, "field":"written_at"})),
        );
    }
    assert_eq!(page_bytes(route.clone()).await, baseline().await);
    assert_eq!(requests(&route).len(), 3, "one attempt per raw page");
}

#[tokio::test]
async fn display_history_mismatched_mid_or_ordinal_gives_no_echo() {
    for mismatch in ["mid", "ordinal"] {
        let route = route();
        for reply in route.history_replies.lock().unwrap().values_mut() {
            let page = reply.as_mut().unwrap();
            for entry in page["messages"].as_array_mut().unwrap() {
                entry[mismatch] = if mismatch == "mid" {
                    json!("another-message")
                } else {
                    json!(900)
                };
            }
        }
        assert_eq!(
            page_bytes(route.clone()).await,
            baseline().await,
            "{mismatch}"
        );
        assert_eq!(requests(&route).len(), 3);
    }
}

#[tokio::test]
async fn display_history_decode_transport_and_frame_errors_preserve_page_bytes() {
    for failure in [
        Ok(json!({"messages":"not an array"})),
        Err(ReadError::new("provider_unavailable", "transport failed")),
        Err(ReadError::new("reply_too_large", "reply exceeds frame")),
    ] {
        let route = route();
        for reply in route.history_replies.lock().unwrap().values_mut() {
            *reply = failure.clone();
        }
        assert_eq!(page_bytes(route.clone()).await, baseline().await);
        assert_eq!(
            requests(&route).len(),
            3,
            "history failures are not retried"
        );
    }
}

#[tokio::test]
async fn display_history_requests_exactly_each_raw_page_range_in_display_form() {
    let route = route();
    let source = BrocaSource::new(resolved(), route.clone());
    source.newest(6).await.unwrap();
    assert_eq!(
        requests(&route),
        vec![
            json!({"from_ordinal":20, "limit":2, "form":"display"}),
            json!({"from_ordinal":22, "limit":2, "form":"display"}),
            json!({"from_ordinal":24, "limit":2, "form":"display"}),
        ]
    );
}
