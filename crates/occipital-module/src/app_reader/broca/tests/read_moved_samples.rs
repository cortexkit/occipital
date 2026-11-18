//! The sample set `phone-reader-read-moved-v1`: what the apps receive when
//! they read a chat whose agent moved to another computer, from this
//! computer's Occipital. The runner here no longer serves the session and
//! refuses every read `session_held_elsewhere`, which is final here, so the app
//! gets that code, never `provider_unavailable`, and looks the agent up again
//! instead of retrying.
//!
//! Each reply is the error frame Occipital's real request handling sends,
//! `{code, message, detail?}`:
//! - `01-page-runner-moved` and `02-details-runner-moved`: Broca's own refusal
//!   (`held_elsewhere_1118/read-refused.json`, recorded from Broca's serve
//!   path), which names no new computer yet.
//! - `03-page-runner-moved-holder`: the runner contract's sample that names
//!   the new computer, served with it as `holder`.

use super::tool_display_samples::AnyHarness;
use super::*;

const SET: &str = "phone-reader-read-moved-v1";

/// Broca's recorded refusal of a read of a session moved away.
const BROCA_READ: &str = include_str!("held_elsewhere_1118/read-refused.json");
/// Broca's recorded refusal of a send to it, byte for byte the same.
const BROCA_SEND: &str = include_str!("held_elsewhere_1118/send-refused.json");

/// `body` as the runner's daemon call returns it.
fn refusal(body: &Value) -> subc_protocol::ErrorBody {
    subc_protocol::ErrorBody {
        code: body["code"].as_str().unwrap().into(),
        message: body["message"].as_str().unwrap().into(),
        detail: body.get("detail").cloned(),
    }
}

/// The runner contract's `session-held-elsewhere.json` sample that names the
/// new computer.
fn contract_with_holder() -> Value {
    let (_, file) = cortexkit_role_llm_runner::vectors::FILES
        .iter()
        .find(|(name, _)| *name == "session-held-elsewhere.json")
        .unwrap();
    let file: Value = serde_json::from_str(file).unwrap();
    file["samples"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sample| sample["error"]["detail"]["holder"].is_string())
        .unwrap()["error"]
        .clone()
}

/// Broca's own bodies: the same refusal for a send and for a read, and it
/// names no new computer, so a send reaches the app without `detail`, as
/// `phone-reader-send-moved-v1`'s fourth sample shows.
#[test]
fn broca_s_held_elsewhere_bodies_read_as_the_contract_s_code() {
    assert_eq!(BROCA_READ, BROCA_SEND);
    let body: Value = serde_json::from_str(BROCA_READ).unwrap();
    assert_eq!(body["code"], crate::moves::SESSION_HELD_ELSEWHERE);
    assert_eq!(body["detail"], json!({}));
    assert!(crate::moves::runner_holder(body.get("detail")).is_empty());
    let refused = crate::app_reader::send::refused(
        body["code"].as_str().unwrap(),
        body["message"].as_str().unwrap(),
        body.get("detail"),
    );
    assert_eq!(refused.code, crate::moves::SESSION_HELD_ELSEWHERE);
    assert_eq!(refused.detail, None);
}

#[tokio::test]
async fn the_read_moved_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::Hosts(
                BROCA_RUNNER_MODULE_ID,
                "broca",
            )))),
            Arc::new(AnyHarness(route.clone())),
        ),
    )));
    let mut served = super::served_samples::Served::new(SET, app);
    // A refused read is the error frame the app receives.
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

    let broca: Value = serde_json::from_str(BROCA_READ).unwrap();
    *route.refusal.lock().unwrap() = Some(refusal(&broca));
    let page = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": "ses_head"}});
    let (answer, reply) = refused(&mut served, 1, &page).await;
    assert_eq!(answer["code"], "session_held_elsewhere", "{answer}");
    assert!(answer.get("detail").is_none(), "{answer}");
    served.keep("01-page-runner-moved", &page, reply);

    let details = json!({"method": super::super::super::DETAILS,
        "params": {"session": "ses_head"}});
    let (answer, reply) = refused(&mut served, 2, &details).await;
    assert_eq!(answer["code"], "session_held_elsewhere", "{answer}");
    served.keep("02-details-runner-moved", &details, reply);

    let named = contract_with_holder();
    *route.refusal.lock().unwrap() = Some(refusal(&named));
    let (answer, reply) = refused(&mut served, 3, &page).await;
    assert_eq!(answer["code"], "session_held_elsewhere", "{answer}");
    assert_eq!(
        answer["detail"],
        json!({"holder": named["detail"]["holder"]}),
        "{answer}"
    );
    served.keep("03-page-runner-moved-holder", &page, reply);
    served.check();
}

/// Any other runner refusal of a read still reaches the app as the runner
/// being unavailable, which it retries. A `holder` that isn't a box id is left
/// out of a moved chat's refusal, which then carries no `detail`.
#[test]
fn only_a_moved_session_s_read_is_final() {
    let read = |code: &str, detail: Option<Value>| {
        super::super::read_call_error(
            ReadOp::Read,
            BROCA_RUNNER_MODULE_ID,
            CallError::Module(subc_protocol::ErrorBody {
                code: code.into(),
                message: "refused".into(),
                detail,
            }),
        )
    };
    assert_eq!(read("session_moving", None).code, "provider_unavailable");
    let moved = read(
        "session_held_elsewhere",
        Some(json!({"holder": "not a box id"})),
    );
    assert_eq!(moved.code, "session_held_elsewhere");
    assert_eq!(moved.detail, None);
}
