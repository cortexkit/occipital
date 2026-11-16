//! The sample set `phone-reader-send-host-refused-v1`: what the apps receive for
//! a message the agent's host refused for good (runner contract 0.8.58,
//! `not_delivered` with reason `host_refused`). Resending the same words can't
//! succeed, so the entry names why (`cause`) and the app offers the words back
//! for editing instead of a plain Try again.
//!
//! The runner's answers are the contract's own wire vectors (`send.json`,
//! `send-status.json`, `run-result.json`), and every reply is served through
//! Occipital's real request handling, byte for byte what a phone receives:
//! - `01-send-refused`: a send the runner answers `host_refused` at once, as it
//!   does for a repeat of a send that already ended so. Refused as final.
//! - `02-newest-page`: a held send the host then refused because it can't read
//!   the attached file, listed `unsent` with its reason, code and cause.

use super::tool_display_samples::AnyHarness;
use super::*;
use crate::app_reader::send::{SendTarget, SendTargets, Sender};
use crate::app_reader::send_store::Receipts;

const SET: &str = "phone-reader-send-host-refused-v1";
/// When both sends are made and the page is read.
const NOW: i64 = 1_794_850_000_000;

/// The named case in contract vector file `name`, from `list`, under `member`.
fn vector(name: &str, list: &str, member: &str, case: &str) -> Value {
    let (_, body) = cortexkit_role_llm_runner::vectors::FILES
        .iter()
        .find(|(file, _)| *file == name)
        .unwrap();
    let file: Value = serde_json::from_str(body).unwrap();
    file[list]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"].as_str().unwrap().contains(case))
        .unwrap()[member]
        .clone()
}

/// The chat's runner, answering each send with the next of its replies.
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
            "session.head" => json!({"incarnation": "inc-a", "first_ordinal": 1,
                "history_version": 1, "updated_at": 1,
                "last_run_state": {"run_id": "run_prev", "state": "completed"},
                "sender": {"modes": ["steer", "idle_start"], "interrupt": true}}),
            "role.describe" => json!({"capabilities": ["send_status"]}),
            "session.send" => self.sends.lock().unwrap().pop_front().unwrap(),
            "session.send_status" => vector("send-status.json", "replies", "reply", "host refusal"),
            "run.status" => vector("run-result.json", "answers", "answer", "unreadable"),
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
async fn the_send_host_refused_samples_are_what_the_reader_serves() {
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
                vector("send.json", "replies", "reply", "host refused"),
                json!({"state": "pending", "submission_id": "sub-5"}),
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
                "text": "Summarize the attached report."}}),
        json!({"mid": "msg_b", "ordinal": 2, "written_at": NOW - 540_000,
            "message": {"id": "msg_b", "type": "assistant",
                "time": {"created": NOW - 590_000, "completed": NOW - 540_000},
                "content": [{"type": "text", "text": "Send it when you're ready."}]}}),
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

    let request = send("c-refused", "Summarize the attached report.");
    let reply = served.read(1, &request).await;
    let refused = super::served_samples::decoded(&reply)["result"].clone();
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["code"], "not_delivered", "{refused}");
    assert_eq!(refused["reason"], "host_refused", "{refused}");
    assert_eq!(refused["terminal"], true, "{refused}");
    served.keep("01-send-refused", &request, reply);

    let held = served
        .read(2, &send("c-unsent", "Summarize the attached report now."))
        .await;
    let held = super::served_samples::decoded(&held)["result"].clone();
    assert_eq!(held["ok"], true, "{held}");
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
    assert_eq!(entry["reason"], "host_refused", "{entry}");
    assert_eq!(entry["runnerCode"], "provider_error", "{entry}");
    assert_eq!(entry["cause"], "attachment_unreadable", "{entry}");
    assert!(entry.get("retryAfterMs").is_none(), "{entry}");
    served.keep("02-newest-page", &request, reply);
    served.check();
}
