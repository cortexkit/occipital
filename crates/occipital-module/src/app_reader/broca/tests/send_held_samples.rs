//! The sample set `phone-reader-send-held-v1`: messages sent while the chat's
//! runner restarts for an update, which this module keeps and resends
//! (`app_reader::send::resend`). Each reply is what Occipital's real request
//! handling sends, byte for byte what a phone receives:
//! - `01-send-held`: a send while the runner can't be reached, accepted with
//!   `held: true`.
//! - `02-page-held`: the newest page, listing that send `queued` with `held`
//!   (`sinceMs`, `untilMs`).
//! - `03-send-behind`: a later send once the runner is back, kept behind the
//!   first so it can't overtake it.
//! - `04-page-two-held`: both held, in the order they were sent.
//! - `05-send-now-held`: "Send now" on a held message, answered `not_queued`.
//! - `06-page-taken`: the runner took both, in order, and holds them for the
//!   next turn: listed `queued`, no longer `held`.
//! - `07-send-steer-held`, `08-page-steer-held`: a steer sent while the
//!   runner restarts, held like any send.
//! - `09-page-given-up`: a send the runner never took in 10 minutes, listed
//!   `unsent` with reason `runner_unavailable`.

use super::tool_display_samples::AnyHarness;
use super::*;
use crate::app_reader::send::{SendTarget, SendTargets, Sender};
use crate::app_reader::send_store::Receipts;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

const SET: &str = "phone-reader-send-held-v1";
const NOW: i64 = 1_795_640_000_000;

/// A runner that is down while it restarts, and once back queues every send
/// behind its running turn.
struct Restarting {
    reference: SessionRef,
    down: AtomicBool,
    sends: Mutex<Vec<Value>>,
}

#[async_trait]
impl SendTarget for Restarting {
    fn reference(&self) -> &SessionRef {
        &self.reference
    }

    async fn call(&self, method: &'static str, params: Value) -> Result<Value, RunnerError> {
        if self.down.load(Ordering::SeqCst) {
            return Err(RunnerError::Unreachable(
                "route.open refused: module_reloading".into(),
            ));
        }
        match method {
            "session.head" => Ok(json!({"incarnation": "inc-a", "first_ordinal": 1,
                "history_version": 1, "updated_at": 1,
                "last_run_state": {"run_id": "run_1", "state": "active"},
                "sender": {"modes": ["steer", "idle_start"], "interrupt": true}})),
            "role.describe" => Ok(json!({"capabilities": ["send_status", "send_promote"]})),
            "session.send" => {
                let mut sends = self.sends.lock().unwrap();
                sends.push(params);
                Ok(json!({"state": "pending", "submission_id": format!("sub-{}", sends.len())}))
            }
            "session.send_status" => Ok(json!({"state": "pending", "submission_id": "sub-1"})),
            other => Err(RunnerError::Unreachable(format!(
                "not in this test: {other}"
            ))),
        }
    }
}

struct OneChat(Arc<Restarting>);

#[async_trait]
impl SendTargets for OneChat {
    async fn open(&self, _: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        Ok(self.0.clone())
    }
}

/// The waiting list on a served page, as `(clientMessageId, state, held,
/// reason)`.
fn listed(reply: &[u8]) -> Vec<(Value, Value, Value, Value)> {
    let page = super::served_samples::decoded(reply)["result"].clone();
    page["queuedSends"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .map(|entry| {
            let member = |name: &str| entry.get(name).cloned().unwrap_or(Value::Null);
            (
                member("clientMessageId"),
                member("state"),
                member("held"),
                member("reason"),
            )
        })
        .collect()
}

#[tokio::test]
async fn the_send_held_samples_are_what_the_reader_serves() {
    let dir = tempfile::tempdir().unwrap();
    let receipts =
        Receipts::open(crate::store::tests::shared(&dir.path().join("store.db"))).unwrap();
    let runner = Arc::new(Restarting {
        reference: SessionRef {
            runner: BROCA_RUNNER_MODULE_ID.into(),
            session: "ses_head".into(),
        },
        down: AtomicBool::new(true),
        sends: Mutex::default(),
    });
    let now = Arc::new(AtomicI64::new(NOW));
    let clock = now.clone();
    let sender = Arc::new(Sender::with_clock(
        Arc::new(OneChat(runner.clone())),
        Some(receipts),
        Arc::new(move || clock.load(Ordering::SeqCst)),
    ));
    let route = FakeRoute::new();
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::with_sender(
        Arc::new(BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::Hosts(
                BROCA_RUNNER_MODULE_ID,
                "broca",
            )))),
            Arc::new(AnyHarness(route)),
        )),
        sender.clone(),
    ));
    let mut served = super::served_samples::Served::new(SET, app);
    let send = |id: &str, text: &str, mode: &str| {
        json!({"method": super::super::super::SEND, "params": {"session": "ses_head",
            "harness": "broca", "text": text, "clientMessageId": id, "mode": mode}})
    };
    let page = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": "ses_head", "harness": "broca", "limit": 50}});
    let held = |since: i64| json!({"sinceMs": since, "untilMs": since + 600_000});
    let mut corr = 0;
    let mut next = || {
        corr += 1;
        corr
    };

    // 01, 02: sent while the runner restarts.
    let request = send("c-first", "Is the release ready?", "queue");
    let reply = served.read(next(), &request).await;
    let answer = super::served_samples::decoded(&reply)["result"].clone();
    assert_eq!(
        (&answer["ok"], &answer["held"]),
        (&json!(true), &json!(true)),
        "{answer}"
    );
    served.keep("01-send-held", &request, reply);
    let reply = served.read(next(), &page).await;
    assert_eq!(
        listed(&reply),
        [(json!("c-first"), json!("queued"), held(NOW), Value::Null)]
    );
    served.keep("02-page-held", &page, reply);

    // 03, 04: the runner is back, but a later send waits behind the first.
    runner.down.store(false, Ordering::SeqCst);
    now.store(NOW + 2_000, Ordering::SeqCst);
    let request = send("c-second", "And tag it when it is.", "queue");
    let reply = served.read(next(), &request).await;
    let answer = super::served_samples::decoded(&reply)["result"].clone();
    assert_eq!(answer["held"], true, "{answer}");
    assert!(runner.sends.lock().unwrap().is_empty());
    served.keep("03-send-behind", &request, reply);
    let reply = served.read(next(), &page).await;
    assert_eq!(
        listed(&reply),
        [
            (json!("c-first"), json!("queued"), held(NOW), Value::Null),
            (
                json!("c-second"),
                json!("queued"),
                held(NOW + 2_000),
                Value::Null
            ),
        ]
    );
    served.keep("04-page-two-held", &page, reply);

    // 05: "Send now" can't move a message the runner hasn't got.
    let request = json!({"method": super::super::super::PROMOTE, "params": {
        "session": "ses_head", "harness": "broca", "clientMessageId": "c-first",
        "clientRequestId": "press-1"}});
    let reply = served.read(next(), &request).await;
    assert_eq!(
        super::served_samples::decoded(&reply)["result"],
        json!({"outcome": "not_queued"})
    );
    served.keep("05-send-now-held", &request, reply);

    // 06: the resender sends both, in order, under their own send ids.
    now.store(NOW + 3_000, Ordering::SeqCst);
    assert!(sender.resend_kept_now("ses_head").await);
    assert!(sender.resend_kept_now("ses_head").await);
    let texts: Vec<Value> = runner
        .sends
        .lock()
        .unwrap()
        .iter()
        .map(|body| body["prompt"].clone())
        .collect();
    assert_eq!(
        texts,
        [
            json!("Is the release ready?"),
            json!("And tag it when it is.")
        ]
    );
    let reply = served.read(next(), &page).await;
    assert_eq!(
        listed(&reply),
        [
            (json!("c-first"), json!("queued"), Value::Null, Value::Null),
            (json!("c-second"), json!("queued"), Value::Null, Value::Null),
        ]
    );
    served.keep("06-page-taken", &page, reply);

    // 07, 08: a steer sent during the next restart is held like any send.
    runner.down.store(true, Ordering::SeqCst);
    now.store(NOW + 60_000, Ordering::SeqCst);
    let request = send("c-steer", "Skip the staging step.", "steer");
    let reply = served.read(next(), &request).await;
    assert_eq!(
        super::served_samples::decoded(&reply)["result"]["held"],
        true
    );
    served.keep("07-send-steer-held", &request, reply);
    let reply = served.read(next(), &page).await;
    assert_eq!(
        listed(&reply)[2],
        (
            json!("c-steer"),
            json!("queued"),
            held(NOW + 60_000),
            Value::Null
        )
    );
    served.keep("08-page-steer-held", &page, reply);

    // 09: the runner never comes back within 10 minutes.
    now.store(NOW + 60_000 + 600_000, Ordering::SeqCst);
    assert!(sender.resend_kept_now("ses_head").await);
    let reply = served.read(next(), &page).await;
    assert_eq!(
        listed(&reply)[2],
        (
            json!("c-steer"),
            json!("unsent"),
            Value::Null,
            json!("runner_unavailable")
        )
    );
    served.keep("09-page-given-up", &page, reply);
    served.check();
}
