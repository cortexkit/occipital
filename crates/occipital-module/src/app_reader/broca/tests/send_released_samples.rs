//! The sample set `phone-reader-send-released-v1`: what the apps receive for a
//! message sent to a chat its owner released, which the runner deletes soon.
//! The app keeps the words as a draft and says the chat is ending; it never
//! resends, since a send after the deletion would start an empty chat under
//! the old name.
//!
//! Each reply is the error frame Occipital's real request handling sends,
//! `{code, message, detail?}`, byte for byte what a phone receives:
//! - `01-send-released`: Broca's own `session_released` refusal, from its
//!   `session-retention-1123` samples, with the deletion time as
//!   `detail.deleteAfterMs`.
//! - `02-send-released-no-time`: the same refusal from a runner that names no
//!   deletion time, which carries no `detail`.

use super::tool_display_samples::AnyHarness;
use super::*;
use crate::app_reader::send::{SendTarget, SendTargets, Sender};
use crate::app_reader::send_store::Receipts;

const SET: &str = "phone-reader-send-released-v1";
const NOW: i64 = 1_795_394_242_800;

/// The runner's answer to each send, in order.
struct Recorded {
    reference: SessionRef,
    sends: Mutex<std::collections::VecDeque<Result<Value, RunnerError>>>,
}

#[async_trait]
impl SendTarget for Recorded {
    fn reference(&self) -> &SessionRef {
        &self.reference
    }

    async fn call(&self, method: &'static str, _: Value) -> Result<Value, RunnerError> {
        match method {
            "session.head" => Ok(json!({"incarnation": "inc-a", "first_ordinal": 1,
                "history_version": 1, "updated_at": 1,
                "last_run_state": {"run_id": "run_prev", "state": "completed"},
                "sender": {"modes": ["steer", "idle_start"], "interrupt": true}})),
            "role.describe" => Ok(json!({"capabilities": ["send_status"]})),
            "session.send" => self.sends.lock().unwrap().pop_front().unwrap(),
            other => panic!("the sender called {other}"),
        }
    }
}

struct OneChat(Arc<Recorded>);

#[async_trait]
impl SendTargets for OneChat {
    async fn open(&self, _: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        Ok(self.0.clone())
    }
}

/// Broca's recorded `session_released` refusal, and the same without its time.
fn refusals() -> [Result<Value, RunnerError>; 2] {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/broca-session-retention-1123/retention.released.send.json");
    let body: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let refusal = |detail: Option<Value>| {
        Err(RunnerError::Refused {
            code: body["code"].as_str().unwrap().into(),
            message: body["message"].as_str().unwrap().into(),
            detail,
        })
    };
    [refusal(body.get("detail").cloned()), refusal(None)]
}

#[tokio::test]
async fn the_send_released_samples_are_what_the_reader_serves() {
    let dir = tempfile::tempdir().unwrap();
    let receipts =
        Receipts::open(crate::store::tests::shared(&dir.path().join("store.db"))).unwrap();
    let runner = Arc::new(Recorded {
        reference: SessionRef {
            runner: BROCA_RUNNER_MODULE_ID.into(),
            session: "ses_head".into(),
        },
        sends: Mutex::new(refusals().into()),
    });
    let sender = Arc::new(Sender::with_clock(
        Arc::new(OneChat(runner)),
        Some(receipts),
        Arc::new(|| NOW),
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
        sender,
    ));
    let mut served = super::served_samples::Served::new(SET, app);
    let send = |id: &str| {
        json!({"method": super::super::super::SEND, "params": {"session": "ses_head",
            "harness": "broca", "text": "Is the release ready?", "clientMessageId": id}})
    };
    // A refused send is the error frame the app receives.
    async fn refused(
        served: &mut super::served_samples::Served,
        corr: u64,
        request: &Value,
    ) -> (Value, Vec<u8>) {
        served.send(corr, request).await;
        let (ty, reply) = served
            .next(corr, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(
            ty,
            subc_protocol::FrameType::Error,
            "{}",
            String::from_utf8_lossy(&reply)
        );
        (serde_json::from_slice(&reply).unwrap(), reply)
    }

    let request = send("c-released");
    let (answer, reply) = refused(&mut served, 1, &request).await;
    assert_eq!(answer["code"], "session_released", "{answer}");
    assert_eq!(
        answer["detail"],
        json!({"deleteAfterMs": 1_795_394_242_735_u64}),
        "{answer}"
    );
    served.keep("01-send-released", &request, reply);

    let request = send("c-released-no-time");
    let (answer, reply) = refused(&mut served, 2, &request).await;
    assert_eq!(answer["code"], "session_released", "{answer}");
    assert!(answer.get("detail").is_none(), "{answer}");
    served.keep("02-send-released-no-time", &request, reply);
    served.check();
}
