//! `phone-reader-request-context-v1`: what the apps get from
//! `session.request_context`, served through the app reader's own path from
//! runners' recorded plan views: Thalamus's from real OpenCode 2 and Claude Code
//! hosts (`tests/data/thalamus-plan-view-v1/`) and Broca's from its real
//! serializer (`tests/data/broca-plan-view-1112/`), both copied unchanged and
//! holding fixture text only. Each recorded reply is the runner's wire body:
//! `{"result": …}` for an answer, the refusal object for an ERROR frame. A
//! refusal goes through the same mapping the live route uses for a refused call
//! ([`super::super::read_call_error`]), so the app's error is the one a live
//! refusal gives. Broca's README speaks of its "new storage", the session store
//! every head lives in and the only one that serves plan views, and of
//! `on_prefix_rebuild`, a plan accepted to take effect the next time the
//! request's opening is rebuilt, so it shows as pending until then.

use super::served_samples::{decoded, Served};
use super::*;

/// Routes to the test session, whatever harness the request names.
struct AnyHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for AnyHarness {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// A recorded plan view: the folder under `tests/data/` and the file.
fn recorded(folder: &str, name: &str) -> Value {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(folder)
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// What `runner`'s route answers for one recorded reply: the answer, or the
/// refusal as the live route maps it.
fn runner_reply(runner: &str, body: Value) -> Result<Value, ReadError> {
    match body.get("result") {
        Some(result) => Ok(result.clone()),
        None => Err(super::super::read_call_error(
            ReadOp::PlanView,
            runner,
            CallError::Module(subc_protocol::ErrorBody {
                code: body["code"].as_str().unwrap().into(),
                message: body["message"].as_str().unwrap().into(),
                detail: body.get("detail").cloned(),
            }),
        )),
    }
}

/// Entorhinal's answer for the test session: a head of `harness` hosted by
/// `runner`.
struct Head {
    runner: &'static str,
    harness: &'static str,
}

#[async_trait]
impl Entorhinal for Head {
    async fn request(&self, method: &str, _: Value) -> Result<Value, ReadError> {
        assert!(
            matches!(
                method,
                "agent.session_for_harness" | "agent.session_for_runner"
            ),
            "unexpected Entorhinal call {method}"
        );
        Ok(
            json!({"result": {"agent_id": "agent-test", "ref_kind": "alfonso", "ref": "test-ref",
            "incarnation": "entorhinal-test", "ended_at_ms": null,
            "runner": self.runner, "session": "ses_head", "harness": self.harness,
            "project_root": "/workspace/head", "network": "unknown", "network_history": []}}),
        )
    }
}

/// The runner hosting `harness`: Broca for `broca`, Thalamus's for the rest.
fn runner_of(harness: &str) -> &'static str {
    if harness == "broca" {
        BROCA_RUNNER_MODULE_ID
    } else {
        THALAMUS_RUNNER_MODULE_ID
    }
}

fn app(route: &Arc<FakeRoute>, harness: &'static str) -> Arc<super::super::super::AppReader> {
    Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(Head {
                runner: runner_of(harness),
                harness,
            }))),
            Arc::new(AnyHarness(route.clone())),
        ),
    )))
}

fn request(harness: &str) -> Value {
    json!({"method": "session.request_context",
        "params": {"session": "ses_head", "harness": harness}})
}

/// Every file of each recorded set matches its `SHA256SUMS`, which lists
/// `files` of them.
#[test]
fn the_recorded_plan_views_keep_their_checksums() {
    for (folder, files) in [("thalamus-plan-view-v1", 13), ("broca-plan-view-1112", 5)] {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(folder);
        let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
        assert_eq!(sums.lines().count(), files, "{folder}");
        for line in sums.lines() {
            let (expected, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(root.join(name)).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                expected,
                "{folder}/{name}"
            );
        }
    }
}

/// Each recorded answer reaches the app in the apps' spelling with its text,
/// item ranges, schemas, tool order, estimates and pending notes unchanged.
/// The refusals keep their own codes, `no_plan` and `context_not_ready`, and a
/// Pi session, whose runner route doesn't list the read, is answered
/// `unsupported` without asking.
#[tokio::test]
async fn the_request_context_samples_are_what_the_reader_serves() {
    let mut served = Served::new(
        "phone-reader-request-context-v1",
        app(&FakeRoute::new(), "opencode2"),
    );
    let thalamus = "thalamus-plan-view-v1";
    let broca = "broca-plan-view-1112";
    let mut corr = 1;
    for (name, folder, file, harness, app_code) in [
        (
            "01-opencode2-in-force",
            thalamus,
            "01-opencode2-in-force.reply.json",
            "opencode2",
            None,
        ),
        (
            "02-opencode2-pending",
            thalamus,
            "02-opencode2-pending.reply.json",
            "opencode2",
            None,
        ),
        (
            "03-opencode2-applied",
            thalamus,
            "03-opencode2-applied.reply.json",
            "opencode2",
            None,
        ),
        (
            "04-claude-code-in-force",
            thalamus,
            "04-claude-code-in-force.reply.json",
            "claude-code",
            None,
        ),
        (
            "05-no-plan",
            thalamus,
            "05-no-plan.reply.json",
            "opencode2",
            Some("no_plan"),
        ),
        (
            "06-awaiting-plan-applied",
            thalamus,
            "06-awaiting-plan-applied.reply.json",
            "opencode2",
            Some("context_not_ready"),
        ),
        ("08-broca-applied", broca, "applied.json", "broca", None),
        ("09-broca-pending", broca, "pending.json", "broca", None),
        (
            "10-broca-no-plan",
            broca,
            "no-plan.json",
            "broca",
            Some("no_plan"),
        ),
    ] {
        let route = FakeRoute::new();
        *route.describe.lock().unwrap() = Some(describe_with(&["plan_view"]));
        let body = recorded(folder, file);
        *route.plan_view.lock().unwrap() = Some(runner_reply(runner_of(harness), body.clone()));
        served.serve_with(app(&route, harness));
        let request = request(harness);
        served.send(corr, &request).await;
        let (_, reply) = served.next(corr, Duration::from_secs(5)).await.unwrap();
        let answer = decoded(&reply);
        match app_code {
            Some(code) => assert_eq!(answer["code"], code, "{name}: {answer}"),
            None => {
                let result = &answer["result"];
                let view = &body["result"];
                assert_eq!(result["generation"], view["generation"], "{name}");
                assert_eq!(result["system"]["text"], view["system"]["text"], "{name}");
                assert_eq!(result["system"]["items"], view["system"]["items"], "{name}");
                assert_eq!(result["appliedAtMs"], view["applied_at_ms"], "{name}");
                assert_eq!(
                    result["newestStep"]["replyMessageId"], view["newest_step"]["reply_mid"],
                    "{name}"
                );
                assert_eq!(result["asPlanned"], true, "{name}");
                let names = |tools: &Value| -> Vec<Value> {
                    tools
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|tool| tool["name"].clone())
                        .collect()
                };
                assert_eq!(names(&result["tools"]), names(&view["tools"]), "{name}");
                for (tool, planned) in result["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(view["tools"].as_array().unwrap())
                {
                    assert_eq!(tool["inputSchema"], planned["input_schema"], "{name}");
                    assert_eq!(tool["provider"], planned["provider"], "{name}");
                    assert_eq!(tool["tokens"], planned["tokens"], "{name}");
                }
                assert_eq!(result["pending"], view["pending"], "{name}");
            }
        }
        served.keep(name, &request, reply);
        corr += 1;
    }
    // A Pi session: its route's `role.describe` doesn't list the read.
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&[]));
    served.serve_with(app(&route, "pi"));
    let request = request("pi");
    served.send(corr, &request).await;
    let (_, reply) = served.next(corr, Duration::from_secs(5)).await.unwrap();
    assert_eq!(decoded(&reply)["code"], "unsupported");
    assert!(route
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| call["method"] != "session.plan_view"));
    served.keep("07-pi-unsupported", &request, reply);
    served.check();
}
