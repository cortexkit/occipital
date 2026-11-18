//! The sample set `phone-reader-send-moved-v1`: what the apps receive for a
//! message sent to a chat whose agent is moving, or has moved, to another
//! computer. The app keeps the words as a draft and offers them again once it
//! knows where the agent lives; it never resends on its own.
//!
//! Each reply is the error frame Occipital's real request handling sends,
//! `{code, message, detail?}`, byte for byte what a phone receives:
//! - `01-send-moving`: the agent is moving now. Occipital holds every send to
//!   its chats from the move's start, so none lands after the cut
//!   (`session_moving`).
//! - `02-send-moved-here`: the move finished and Occipital here let the chat
//!   go (`session_held_elsewhere`, the new computer as `holder`).
//! - `03-send-runner-moved` and `04-send-runner-moved-no-holder`: the runner
//!   here refuses the send because the chat moved (llm-runner 0.8.63's own
//!   samples, with and without the new computer), served as the same code.

use super::tool_display_samples::AnyHarness;
use super::*;
use crate::app_reader::send::{SendTarget, SendTargets, Sender};
use crate::app_reader::send_store::Receipts;
use crate::moves::state::{Cut, Cuts, Side, Step};

const SET: &str = "phone-reader-send-moved-v1";
const NOW: i64 = 1_794_950_000_000;
/// The computer the agent moved to, as a box id.
const HOLDER: &str = "628c5a7a92589861e4f9f74836c34819";

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

/// The runner contract's refusal sample `n` from `session-held-elsewhere.json`.
fn runner_refusal(n: usize) -> Result<Value, RunnerError> {
    let (_, file) = cortexkit_role_llm_runner::vectors::FILES
        .iter()
        .find(|(name, _)| *name == "session-held-elsewhere.json")
        .unwrap();
    let file: Value = serde_json::from_str(file).unwrap();
    let error = &file["samples"][n]["error"];
    Err(RunnerError::Refused {
        code: error["code"].as_str().unwrap().into(),
        message: error["message"].as_str().unwrap().into(),
        detail: error.get("detail").cloned(),
    })
}

#[tokio::test]
async fn the_send_moved_samples_are_what_the_reader_serves() {
    let dir = tempfile::tempdir().unwrap();
    let receipts =
        Receipts::open(crate::store::tests::shared(&dir.path().join("store.db"))).unwrap();
    let runner = Arc::new(Recorded {
        reference: SessionRef {
            runner: THALAMUS_RUNNER_MODULE_ID.into(),
            session: "ses_head".into(),
        },
        sends: Mutex::new([runner_refusal(0), runner_refusal(1)].into()),
    });
    let sender = Arc::new(Sender::with_clock(
        Arc::new(OneChat(runner)),
        Some(receipts),
        Arc::new(|| NOW),
    ));
    let cuts = Arc::new(Cuts::open(dir.path()).unwrap());
    sender.set_moves(cuts.clone());
    let route = FakeRoute::new();
    *route.run_state.lock().unwrap() = Value::Null;
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
    let mut served = super::served_samples::Served::new(SET, app);
    let send = |id: &str| {
        json!({"method": super::super::super::SEND, "params": {"session": "ses_head",
            "harness": "opencode2", "text": "Is the release ready?", "clientMessageId": id}})
    };
    // A refused send is the error frame the app receives: `{code, message,
    // detail?}`.
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

    let mut cut = Cut {
        agent: "agent_0000000000000101".into(),
        side: Some(Side::Source),
        step: Step::Prepared,
        item: None,
        sessions: vec![crate::moves::payload::Session {
            runner: THALAMUS_RUNNER_MODULE_ID.into(),
            harness: "opencode2".into(),
            session: "ses_head".into(),
        }],
        holder: None,
        pending: false,
        at_ms: NOW - 1_000,
    };
    let cut_id = "0".repeat(32);
    cuts.save(&cut_id, &cut).unwrap();
    let request = send("c-moving");
    let (answer, reply) = refused(&mut served, 1, &request).await;
    assert_eq!(answer["code"], "session_moving", "{answer}");
    served.keep("01-send-moving", &request, reply);

    cut.step = Step::Released;
    cut.holder = Some(HOLDER.into());
    cut.at_ms = NOW - 500;
    cuts.save(&cut_id, &cut).unwrap();
    let request = send("c-moved-here");
    let (answer, reply) = refused(&mut served, 2, &request).await;
    assert_eq!(answer["code"], "session_held_elsewhere", "{answer}");
    assert_eq!(answer["detail"], json!({"holder": HOLDER}), "{answer}");
    served.keep("02-send-moved-here", &request, reply);

    // The runner's own refusal: Occipital here holds nothing for the chat.
    cut.step = Step::Aborted;
    cut.at_ms = NOW - 100;
    cuts.save(&cut_id, &cut).unwrap();
    let request = send("c-runner-moved");
    let (answer, reply) = refused(&mut served, 3, &request).await;
    assert_eq!(answer["code"], "session_held_elsewhere", "{answer}");
    assert_eq!(
        answer["detail"],
        json!({"holder": "0123456789abcdef0123456789abcdef"}),
        "{answer}"
    );
    served.keep("03-send-runner-moved", &request, reply);

    let request = send("c-runner-moved-no-holder");
    let (answer, reply) = refused(&mut served, 4, &request).await;
    assert_eq!(answer["code"], "session_held_elsewhere", "{answer}");
    assert!(answer.get("detail").is_none(), "{answer}");
    served.keep("04-send-runner-moved-no-holder", &request, reply);
    served.check();
}
