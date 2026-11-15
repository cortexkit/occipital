//! The sample set the apps pin for "Send now", `phone-reader-promote-v1`: each
//! `session.promote_send` served through the reader's frame loop on a paired phone's
//! route, with the reply bytes the app receives. Beside each request that reaches the
//! runner, a `.runner-request.json` file keeps the `session.promote` body Occipital
//! sent. The runner's answers are the runner contract's own vectors
//! (`send-promote.json`, llm-runner 0.8.56), so the samples change when the contract's
//! do. Its README says what each file shows.

use super::*;
use crate::app_reader::{broca::tests::served_samples::Served, AppReader, SourceResolver};
use occipital_app_reader::source::TranscriptSource;
use std::time::Duration;

const SET: &str = "phone-reader-promote-v1";
const HEAD: &str = "head-session";

struct OneHead(Arc<FakeRunner>);

#[async_trait]
impl SendTargets for OneHead {
    async fn open(&self, session: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        assert_eq!(session, HEAD, "the sample promoted in {session}");
        Ok(Arc::new(Target(
            self.0.clone(),
            SessionRef {
                runner: "thalamus-harness".into(),
                session: session.into(),
            },
        )))
    }
}

/// "Send now" needs no transcript source.
struct NoSources;

#[async_trait]
impl SourceResolver for NoSources {
    async fn resolve(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        Ok(None)
    }
}

fn describe(promote: bool) -> Value {
    let mut groups = vec!["steer", "delivery_order", "send_status"];
    if promote {
        groups.push("send_promote");
    }
    json!({"result": {"capabilities": groups}})
}

/// The runner contract's promote vectors, by case name: `(request, reply)`.
fn contract_case(name: &str) -> Value {
    let file: Value =
        serde_json::from_str(cortexkit_role_llm_runner::vectors::SEND_PROMOTE_JSON).unwrap();
    file["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("the contract has no promote case {name}"))["reply"]
        .clone()
}

#[tokio::test]
async fn the_promote_samples_are_what_the_reader_serves() {
    let f = granted_fixture(Kind::Head);
    let runner = f.runner.clone();
    let clock = f.now.clone();
    let sender = Arc::new(Sender::with_clock(
        Arc::new(OneHead(runner.clone())),
        Some(f.receipts.clone()),
        Arc::new(move || clock.load(Ordering::SeqCst)),
    ));
    // The person's message, sent to wait for the running turn: the receipt it
    // leaves is how "Send now" names it to the runner.
    let queued = sender
        .send(
            json!({"session": HEAD, "text": "and check the retry path too",
            "clientMessageId": "msg-0b7c"}),
        )
        .await
        .unwrap();
    assert_eq!(queued["ok"], true, "{queued}");
    let sent_as = runner
        .calls()
        .into_iter()
        .find(|(method, _)| method == "session.send")
        .map(|(_, body)| body["send_id"].clone())
        .expect("the message reached the runner");

    let app = Arc::new(AppReader::with_sender(Arc::new(NoSources), sender));
    let mut served = Served::new(SET, app);
    let asked = |runner: &FakeRunner| -> Vec<Value> {
        runner
            .calls()
            .into_iter()
            .filter(|(method, _)| method == "session.promote")
            .map(|(_, params)| params)
            .collect()
    };
    let set = |method: &'static str, reply: Value| {
        runner.captured.lock().unwrap().insert(method, reply);
    };
    async fn serve(served: &mut Served, corr: u64, name: &str, request: &Value) -> Value {
        served.send(corr, request).await;
        let (_, reply) = served
            .next(corr, Duration::from_secs(5))
            .await
            .expect("the module answers");
        served.keep(name, request, reply.clone());
        serde_json::from_slice(&reply).unwrap()
    }
    let keep_runner = |served: &mut Served, name: &str, runner: &FakeRunner| -> Value {
        let body = asked(runner).pop().expect("the runner was asked");
        served.keep_file(
            &format!("{name}.runner-request.json"),
            body.to_string().into_bytes(),
        );
        body
    };
    let request = |message: &str, press: &str| {
        json!({"method": "session.promote_send", "params":
            {"session": HEAD, "clientMessageId": message, "clientRequestId": press}})
    };
    let mut corr = 0;
    let mut next = || {
        corr += 1;
        corr
    };

    set("role.describe", describe(true));

    // 01: still waiting, and the turn is running: it joins that turn.
    set(
        "session.promote",
        json!({"result": contract_case("queued_with_active_run")}),
    );
    let press = request("msg-0b7c", "press-5e21a9c4-01");
    let reply = serve(&mut served, next(), "01-promoted", &press).await;
    assert_eq!(reply["result"], json!({"outcome": "promoted"}), "{reply}");
    let body = keep_runner(&mut served, "01-promoted", &runner);
    assert_eq!(
        body,
        json!({"send_id": sent_as, "request_id": "press-5e21a9c4-01"}),
        "the runner is asked about the message's own send"
    );

    // 02: the app's retry after a lost reply, under the same press id. The runner
    // answers it with its first answer; Occipital asks it the same way.
    let reply = serve(&mut served, next(), "02-repeat", &press).await;
    assert_eq!(reply["result"], json!({"outcome": "promoted"}));
    assert_eq!(keep_runner(&mut served, "02-repeat", &runner), body);

    // 03: the turn ended first, so the message already started or joined one.
    // Nothing runs twice.
    set(
        "session.promote",
        json!({"result": contract_case("turn_ended_first")}),
    );
    let reply = serve(
        &mut served,
        next(),
        "03-delivered",
        &request("msg-0b7c", "press-5e21a9c4-03"),
    )
    .await;
    assert_eq!(reply["result"], json!({"outcome": "delivered"}));
    keep_runner(&mut served, "03-delivered", &runner);

    // 04: still waiting, but no turn is running to join.
    set(
        "session.promote",
        json!({"result": contract_case("parked_queue")}),
    );
    let reply = serve(
        &mut served,
        next(),
        "04-no-active-run",
        &request("msg-0b7c", "press-5e21a9c4-04"),
    )
    .await;
    assert_eq!(
        reply["result"],
        json!({"outcome": "not_promoted", "reason": "no_active_run"})
    );
    keep_runner(&mut served, "04-no-active-run", &runner);

    // 05: the runner has no such send waiting, or it ended without delivery.
    set(
        "session.promote",
        json!({"result": contract_case("unknown_or_other_sender")}),
    );
    let reply = serve(
        &mut served,
        next(),
        "05-not-queued",
        &request("msg-0b7c", "press-5e21a9c4-05"),
    )
    .await;
    assert_eq!(reply["result"], json!({"outcome": "not_queued"}));
    keep_runner(&mut served, "05-not-queued", &runner);

    // 06: a message never sent through Occipital can't be named to the runner,
    // which isn't asked.
    let before = asked(&runner).len();
    let reply = serve(
        &mut served,
        next(),
        "06-unknown-message",
        &request("msg-never-sent", "press-5e21a9c4-06"),
    )
    .await;
    assert_eq!(reply["code"], "not_found", "{reply}");
    assert_eq!(reply["detail"], json!({"field": "clientMessageId"}));
    assert_eq!(asked(&runner).len(), before);

    // 07: a runner without the group is never asked.
    set("role.describe", describe(false));
    let reply = serve(
        &mut served,
        next(),
        "07-unsupported",
        &request("msg-0b7c", "press-5e21a9c4-07"),
    )
    .await;
    assert_eq!(reply["code"], "unsupported", "{reply}");
    assert_eq!(asked(&runner).len(), before);

    // 08: the runner declares the group, but this chat's host takes no steer.
    set("role.describe", describe(true));
    set(
        "session.promote",
        json!({"code": "unsupported",
        "message": "this harness declares no steer"}),
    );
    let reply = serve(
        &mut served,
        next(),
        "08-harness-unsupported",
        &request("msg-0b7c", "press-5e21a9c4-08"),
    )
    .await;
    assert_eq!(reply["code"], "unsupported", "{reply}");

    // 09: a press id already used for another message.
    set(
        "session.promote",
        json!({"code": "request_id_reuse",
        "message": "request_id names another send", "detail": {"field": "send_id"}}),
    );
    let reply = serve(
        &mut served,
        next(),
        "09-press-id-reused",
        &request("msg-0b7c", "press-5e21a9c4-09"),
    )
    .await;
    assert_eq!(reply["code"], "client_request_id_conflict", "{reply}");
    assert_eq!(reply["detail"], json!({"field": "clientRequestId"}));

    // 10: the session's plan grants this module no steer.
    set(
        "session.promote",
        json!({"code": "scope_owner_mismatch",
        "message": "the plan grants this sender no steer"}),
    );
    let reply = serve(
        &mut served,
        next(),
        "10-not-authorized",
        &request("msg-0b7c", "press-5e21a9c4-10"),
    )
    .await;
    assert_eq!(reply["code"], "enqueue_not_authorized", "{reply}");

    served.check();
}
