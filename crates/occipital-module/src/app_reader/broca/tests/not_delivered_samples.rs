//! The sample set `phone-reader-send-not-delivered-v1`: what the apps receive
//! for a message the runner proved never reached the agent's host
//! (`not_delivered`, runner contract 0.8.54).
//!
//! Each runner answer is Thalamus's own recorded body, copied unchanged in
//! `tests/data/thalamus-send-not-delivered-1116/`, and every reply is served
//! through Occipital's real request handling, so it is byte for byte what a
//! phone receives:
//!
//! - `01-send-refused`: a send the runner answers `not_delivered` at once (a
//!   repeat of a send that already ended so). It is refused as final.
//! - `02-newest-page`: a send the runner first held (`pending`), then proved
//!   undelivered. The sender, which asks the runner about each held send until
//!   it lands, lists it `unsent` with the runner's reason and the code its run
//!   ended with, read from `run.status`.

use super::tool_display_samples::AnyHarness;
use super::*;
use crate::app_reader::send::{SendTarget, SendTargets, Sender};
use crate::app_reader::send_store::Receipts;

const SET: &str = "phone-reader-send-not-delivered-v1";
/// When both sends are made and the page is read.
const NOW: i64 = 1_794_810_000_000;

fn recorded(name: &str) -> Value {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-send-not-delivered-1116")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// The chat's runner, answering each send with the next recorded reply.
struct Recorded {
    reference: SessionRef,
    sends: Mutex<std::collections::VecDeque<Value>>,
}

#[async_trait]
impl SendTarget for Recorded {
    fn reference(&self) -> &SessionRef {
        &self.reference
    }

    async fn call(&self, method: &'static str, _: Value) -> Result<Value, RunnerError> {
        Ok(match method {
            // A head with no run going that lets Occipital send to it.
            "session.head" => json!({"incarnation": "inc-a", "first_ordinal": 1,
                "history_version": 1, "updated_at": 1,
                "last_run_state": {"run_id": "run_prev", "state": "completed"},
                "sender": {"modes": ["steer", "idle_start"], "interrupt": true}}),
            "role.describe" => json!({"capabilities": ["send_status"]}),
            "session.send" => self.sends.lock().unwrap().pop_front().unwrap(),
            "session.send_status" => recorded("02-send-status-not-delivered-handover-failed.json"),
            "run.status" => recorded("04-run-status-handover-failed.json"),
            other => panic!("the sender called {other}"),
        })
    }
}

struct OneChat(Arc<Recorded>);

#[async_trait]
impl SendTargets for OneChat {
    async fn open(&self, _: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn the_send_not_delivered_samples_are_what_the_reader_serves() {
    let dir = tempfile::tempdir().unwrap();
    let receipts =
        Receipts::open(crate::store::tests::shared(&dir.path().join("store.db"))).unwrap();
    let runner = Arc::new(Recorded {
        reference: SessionRef {
            runner: THALAMUS_RUNNER_MODULE_ID.into(),
            session: "ses_head".into(),
        },
        sends: Mutex::new(
            [
                recorded("03-send-resend-same-id-not-delivered.json"),
                recorded("01-send-reply-pending.json"),
            ]
            .into(),
        ),
    });
    let sender = Arc::new(Sender::with_clock(
        Arc::new(OneChat(runner)),
        Some(receipts.clone()),
        Arc::new(|| NOW),
    ));
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = vec![
        json!({"mid": "msg_a", "ordinal": 1, "written_at": NOW - 600_000,
            "message": {"id": "msg_a", "type": "user", "time": {"created": NOW - 600_000},
                "text": "Run the migration check."}}),
        json!({"mid": "msg_b", "ordinal": 2, "written_at": NOW - 540_000,
            "message": {"id": "msg_b", "type": "assistant",
                "time": {"created": NOW - 590_000, "completed": NOW - 540_000},
                "content": [{"type": "text", "text": "The check passed."}]}}),
    ];
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::with_sender(
        Arc::new(BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::Hosts(
                THALAMUS_RUNNER_MODULE_ID,
                "opencode2",
            )))),
            Arc::new(AnyHarness(route)),
        )),
        sender.clone(),
    ));
    let mut served = super::served_samples::Served::new(SET, app);
    let send = |id: &str, text: &str| {
        json!({"method": super::super::super::SEND, "params": {"session": "ses_head",
            "harness": "opencode2", "text": text, "clientMessageId": id}})
    };

    let request = send("c-refused", "Deploy it to staging.");
    let reply = served.read(1, &request).await;
    let refused = super::served_samples::decoded(&reply)["result"].clone();
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["code"], "not_delivered", "{refused}");
    assert_eq!(refused["reason"], "handover_failed", "{refused}");
    assert_eq!(refused["terminal"], true, "{refused}");
    served.keep("01-send-refused", &request, reply);

    let held = served
        .read(2, &send("c-unsent", "Now run it on staging."))
        .await;
    let held = super::served_samples::decoded(&held)["result"].clone();
    assert_eq!(held["ok"], true, "{held}");
    // As when the runner announces the send changed: the sender asks the
    // runner for its state now, instead of at its next timed check.
    assert!(sender.nudge(held["effectId"].as_str().unwrap()));
    let listed = || {
        receipts
            .queue()
            .list("ses_head", NOW)
            .unwrap()
            .is_some_and(|(entries, _)| entries[0]["state"] == "unsent")
    };
    for _ in 0..500 {
        if listed() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(listed(), "the follower never listed the send unsent");

    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": "ses_head", "harness": "opencode2", "limit": 50}});
    let reply = served.read(3, &request).await;
    let page = super::served_samples::decoded(&reply)["result"].clone();
    let entries = page["queuedSends"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{page}");
    let entry = &entries[0];
    assert_eq!(entry["clientMessageId"], "c-unsent", "{entry}");
    assert_eq!(entry["state"], "unsent", "{entry}");
    assert_eq!(entry["reason"], "handover_failed", "{entry}");
    assert_eq!(entry["runnerCode"], "provider_error", "{entry}");
    assert!(entry.get("retryAfterMs").is_none(), "{entry}");
    served.keep("02-newest-page", &request, reply);
    served.check();
}
