//! The sample set the apps pin for Stop everything, `phone-reader-stop-all-v1`: each
//! `session.stop_all` served through the reader's frame loop on a paired phone's route,
//! with the reply bytes the app receives. Beside each request that reaches the runner,
//! a `.runner-request.json` file keeps the `session.stop_all` body Occipital sent. The
//! runner's replies are invented in the runner contract's shape. Its README says what
//! each file shows.

use super::*;
use crate::app_reader::{broca::tests::served_samples::Served, AppReader, SourceResolver};
use occipital_app_reader::source::TranscriptSource;
use std::time::Duration;

const SET: &str = "phone-reader-stop-all-v1";
const HEAD: &str = "head-session";

struct OneHead(Arc<FakeRunner>);

#[async_trait]
impl SendTargets for OneHead {
    async fn open(&self, session: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        assert_eq!(session, HEAD, "the sample stopped {session}");
        Ok(Arc::new(Target(
            self.0.clone(),
            SessionRef {
                runner: "thalamus-harness".into(),
                session: session.into(),
            },
        )))
    }
}

/// Stop everything needs no transcript source.
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

fn describe(stop_all: bool) -> Value {
    let mut groups = vec!["run_cancel", "delivery_order", "run_cancel_by_interrupt"];
    if stop_all {
        groups.push("stop_all");
    }
    json!({"result": {"capabilities": groups}})
}

/// Every provider answered: AFT killed a shell command and cancelled a remote build,
/// core paused a worker and a consult, and Basal's program had already finished.
fn full_answer() -> Value {
    json!({"providers": [
        {"provider": "aft", "outcome": "answered", "items": [
            {"kind": "shell", "id": "bash-4f2a", "label": "cargo test (workspace)", "outcome": "stopped"},
            {"kind": "remote_build", "id": "job-7c1e", "label": "Linux build, 8 vCPUs", "outcome": "stopped"}]},
        {"provider": "prefrontal-core", "outcome": "answered", "items": [
            {"kind": "worker", "id": "wi_3b9d0c1a", "label": "Fix the login retry", "outcome": "paused"},
            {"kind": "consult", "id": "consult_51e2", "label": "Review the cache design", "outcome": "paused"}]},
        {"provider": "basal", "outcome": "answered", "items": [
            {"kind": "program", "label": "Codemode program", "outcome": "already_finished"}]}],
        "runner": {"items": []}})
}

/// AFT answered, but one shell command wouldn't stop; core didn't answer within the
/// runner's 15 seconds; Cerebellum doesn't offer the operation.
fn partial_answer() -> Value {
    json!({"providers": [
        {"provider": "aft", "outcome": "answered", "items": [
            {"kind": "shell", "id": "bash-9d10", "label": "npm run watch", "outcome": "could_not_stop",
                "reason": "process_not_found"}]},
        {"provider": "prefrontal-core", "outcome": "no_answer"},
        {"provider": "cerebellum", "outcome": "not_declared"}],
        "runner": {"items": [
            {"kind": "fork_answer", "label": "Open fork answer", "outcome": "stopped"}]}})
}

#[tokio::test]
async fn the_stop_all_samples_are_what_the_reader_serves() {
    let f = granted_fixture(Kind::Head);
    let runner = f.runner.clone();
    let clock = f.now.clone();
    let sender = Arc::new(Sender::with_clock(
        Arc::new(OneHead(runner.clone())),
        Some(f.receipts.clone()),
        Arc::new(move || clock.load(Ordering::SeqCst)),
    ));
    let app = Arc::new(AppReader::with_sender(Arc::new(NoSources), sender));
    let mut served = Served::new(SET, app);
    let asked = |runner: &FakeRunner| -> Vec<Value> {
        runner
            .calls()
            .into_iter()
            .filter(|(method, _)| method == "session.stop_all")
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
    let request = |id: &str, run: Option<&str>| {
        let mut params = json!({"session": HEAD, "clientRequestId": id});
        if let Some(run) = run {
            params["runId"] = json!(run);
        }
        json!({"method": "session.stop_all", "params": params})
    };

    set("role.describe", describe(true));

    // 01: every provider answered.
    set("session.stop_all", json!({"result": full_answer()}));
    let first = request("6a0e2f4c-1b3d-4e5f-8a9b-0c1d2e3f4a01", Some("run_7d2e"));
    let reply = serve(&mut served, 1, "01-full", &first).await;
    assert_eq!(reply["result"], full_answer());
    let body = keep_runner(&mut served, "01-full", &runner);
    assert_eq!(
        body,
        json!({"request_id": "6a0e2f4c-1b3d-4e5f-8a9b-0c1d2e3f4a01", "run_id": "run_7d2e"})
    );

    // 02: the app's retry after a lost reply, under the same id. The runner answers
    // it with its first reply and stops nothing new; Occipital asks it the same way.
    let reply = serve(&mut served, 2, "02-repeat", &first).await;
    assert_eq!(reply["result"], full_answer());
    assert_eq!(keep_runner(&mut served, "02-repeat", &runner), body);

    // 03: a partial answer, with no run named.
    set("session.stop_all", json!({"result": partial_answer()}));
    let partial = request("6a0e2f4c-1b3d-4e5f-8a9b-0c1d2e3f4a02", None);
    let reply = serve(&mut served, 3, "03-partial", &partial).await;
    assert_eq!(reply["result"], partial_answer());
    let body = keep_runner(&mut served, "03-partial", &runner);
    assert_eq!(
        body,
        json!({"request_id": "6a0e2f4c-1b3d-4e5f-8a9b-0c1d2e3f4a02"})
    );

    // 04: a runner that doesn't offer Stop everything is never asked.
    set("role.describe", describe(false));
    let before = asked(&runner).len();
    let unsupported = request("6a0e2f4c-1b3d-4e5f-8a9b-0c1d2e3f4a03", None);
    let reply = serve(&mut served, 4, "04-unsupported", &unsupported).await;
    assert_eq!(reply["code"], "unsupported", "{reply}");
    assert_eq!(asked(&runner).len(), before);

    // 05: a sender without `interrupt` is refused by the runner, as Stop is.
    set("role.describe", describe(true));
    set(
        "session.stop_all",
        json!({"code": "scope_owner_mismatch", "message": "the plan grants this sender no interrupt"}),
    );
    let refused = request("6a0e2f4c-1b3d-4e5f-8a9b-0c1d2e3f4a04", None);
    let reply = serve(&mut served, 5, "05-not-authorized", &refused).await;
    assert_eq!(reply["code"], "enqueue_not_authorized", "{reply}");

    served.check();
}
