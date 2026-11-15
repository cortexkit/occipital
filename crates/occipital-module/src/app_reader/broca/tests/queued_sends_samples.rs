//! The sample set `phone-reader-send-unknown-v1`: an OpenCode 2 chat's newest
//! page with its `queuedSends` list holding each state a person's message can be
//! in before or instead of landing in the chat.
//!
//! The list is the one this module keeps for the apps' sends (`send_queue`),
//! written here through the same calls its follower makes when the runner
//! answers (`app_reader::send::tests` checks the follower makes them), then
//! served on the page through Occipital's real request handling. One send the
//! runner couldn't confirm did land: the chat holds its message under the same
//! `clientMessageId`, and the apps let the stored message win.

use super::tool_display_samples::AnyHarness;
use super::*;
use crate::app_reader::send::{SendTarget, SendTargets, Sender};
use crate::app_reader::send_queue::Queued;
use crate::app_reader::send_store::Receipts;

/// The list's sends go nowhere: the follower never resumes one here.
struct NoRunner;

#[async_trait]
impl SendTargets for NoRunner {
    async fn open(&self, _: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "no runner in this test",
        ))
    }
}

/// When the page is read; each send was accepted a minute apart before it.
const NOW: i64 = 1_794_720_000_000;

fn chat() -> Vec<Value> {
    vec![
        json!({"mid": "msg_a", "ordinal": 1, "written_at": NOW - 600_000,
            "message": {"id": "msg_a", "type": "user", "time": {"created": NOW - 600_000},
                "text": "Run the migration check."}}),
        json!({"mid": "msg_b", "ordinal": 2, "written_at": NOW - 540_000,
            "message": {"id": "msg_b", "type": "assistant",
                "time": {"created": NOW - 590_000, "completed": NOW - 540_000},
                "content": [{"type": "text", "text": "The check passed."}]}}),
        // The message of the send listed `unknown` below as `c-landed`.
        json!({"mid": "msg_c", "ordinal": 3, "written_at": NOW - 60_000,
            "client_message_id": "c-landed",
            "message": {"id": "msg_c", "type": "user", "time": {"created": NOW - 60_000},
                "text": "Now run it on staging."}}),
    ]
}

#[tokio::test]
async fn the_send_unknown_samples_are_what_the_reader_serves() {
    let dir = tempfile::tempdir().unwrap();
    let receipts =
        Receipts::open(crate::store::tests::shared(&dir.path().join("store.db"))).unwrap();
    let queue = receipts.queue();
    let accepted = |id: &str, at: i64| {
        queue
            .queue(&Queued {
                session: "ses_head".into(),
                client_message_id: id.into(),
                harness: Some("opencode2".into()),
                send_id: format!("send-{id}"),
                accepted_at: at,
                steer_until: None,
            })
            .unwrap();
    };
    // Waiting for the agent: the runner still holds it.
    accepted("c-queued", NOW - 30_000);
    // Never reached the agent: the runner couldn't bring its host back.
    accepted("c-unsent", NOW - 240_000);
    assert!(queue
        .unsent(
            "ses_head",
            "c-unsent",
            "handover_failed",
            None,
            NOW - 235_000
        )
        .unwrap());
    // May not have arrived: handed on, and the runner can't confirm it ran.
    accepted("c-unknown", NOW - 180_000);
    assert!(queue
        .unconfirmed("ses_head", "c-unknown", NOW - 175_000)
        .unwrap());
    // The same, but its message did land in the chat.
    accepted("c-landed", NOW - 61_000);
    assert!(queue
        .unconfirmed("ses_head", "c-landed", NOW - 58_000)
        .unwrap());

    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = chat();
    *route.run_state.lock().unwrap() = Value::Null;
    let sender = Arc::new(Sender::with_clock(
        Arc::new(NoRunner),
        Some(receipts),
        Arc::new(|| NOW),
    ));
    let app = Arc::new(super::super::super::AppReader::with_sender(
        Arc::new(BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::Hosts(
                THALAMUS_RUNNER_MODULE_ID,
                "opencode2",
            )))),
            Arc::new(AnyHarness(route)),
        )),
        sender,
    ));
    let mut served = super::served_samples::Served::new("phone-reader-send-unknown-v1", app);
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": "ses_head", "harness": "opencode2", "limit": 50}});
    let reply = served.read(1, &request).await;
    let page = super::served_samples::decoded(&reply)["result"].clone();
    let listed: Vec<(Value, Value, Value)> = page["queuedSends"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["clientMessageId"].clone(),
                entry["state"].clone(),
                entry.get("reason").cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            (json!("c-unsent"), json!("unsent"), json!("handover_failed")),
            (json!("c-unknown"), json!("unknown"), Value::Null),
            (json!("c-landed"), json!("unknown"), Value::Null),
            (json!("c-queued"), json!("queued"), Value::Null),
        ],
        "{page}"
    );
    let landed = page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["ordinal"] == 3)
        .unwrap();
    assert_eq!(landed["clientMessageId"], "c-landed", "{landed}");
    served.keep("01-newest-page", &request, reply);
    served.check();
}
