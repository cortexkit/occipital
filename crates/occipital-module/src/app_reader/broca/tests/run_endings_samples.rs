//! How each way a run ends reads to the apps, served into the sample set
//! `phone-reader-run-endings-v1`.
//!
//! Thalamus, the runner module that hosts OpenCode 2 heads, recorded its own
//! replies for each ending from a real OpenCode 2.2.3 server. They are copied
//! byte for byte into `tests/data/thalamus-run-endings-v2` from the thalamus
//! repository's tag `phone-reader-run-endings-v2` (object `982dce470839…`,
//! commit `6c2bdc21fdc0…`), with its README, which says how each ending was
//! produced and stays as published so the checksums hold. Endings the runner
//! can't reach for real are heads built from a recorded one with only the
//! ending changed, named `*.synthetic.json` there and `*-synthetic` here.
//!
//! This test serves the apps' `session.details` and newest page for every
//! recorded ending, and `session.details` for every synthetic one, through the
//! module's own serving path over those replies. The recorded runner declares
//! display facts, so the reader asks for them, and a call no tool answered
//! carries the runner's `unanswered` reason on its tool part.

use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SET: &str = "phone-reader-run-endings-v1";
/// The digest of the recorded set's `SHA256SUMS` file as published with the tag.
const SUMS_SHA256: &str = "1a98d09b353d2d235eec12de18936de6b2dcba1ad6c155aee84df77c8f8bd57a";

const RECORDED: [&str; 7] = [
    "01-completed",
    "02-max-steps",
    "03-error-provider",
    "04-error-compaction-unavailable",
    "05-completed-tool-not-offered",
    "06-cancelled",
    "07-interrupted",
];

const SYNTHETIC: [&str; 9] = [
    "S1-active-compaction-wait",
    "S2-active-context-ceiling-wait",
    "S3-paused-auth-required",
    "S4-paused-restart",
    "S5-context-ceiling",
    "S6-error-transient",
    "S7-error-auth-required",
    "S8-error-provider-unavailable",
    "S9-error-context-overflow",
];

/// A recorded reply's body: its `result` when the file holds the whole
/// response, or the file itself when it holds only the body.
fn data(set: &str, name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(set)
        .join(name);
    let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    match value.get("result") {
        Some(result) => result.clone(),
        None => value,
    }
}

fn recorded_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/thalamus-run-endings-v2")
}

/// The ending's recorded head: `<case>-session-head.json`, or its synthetic one.
fn head_file(case: &str) -> String {
    if SYNTHETIC.contains(&case) {
        format!("{case}-session-head.synthetic.json")
    } else {
        format!("{case}-session-head.json")
    }
}

#[test]
fn the_run_endings_are_the_published_set() {
    let sums = std::fs::read(recorded_root().join("SHA256SUMS")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&sums)), SUMS_SHA256);
    let sums = String::from_utf8(sums).unwrap();
    assert_eq!(sums.lines().count(), 38, "37 replies and the README");
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(recorded_root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Serves one ending's recorded replies: its head, its read (with display facts
/// when asked for them, sliced to the requested ordinals), and the runner's
/// description from the recorded OpenCode 2 pages, which is the same runner's.
struct Ending(String);

struct Quiet;

#[async_trait]
impl EventStream for Quiet {
    async fn next(&mut self) -> Result<Option<Value>, ReadError> {
        std::future::pending().await
    }
}

#[async_trait]
impl BrocaRoute for Ending {
    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        let request: Value = serde_json::from_slice(&body(op, params)).unwrap();
        let params = &request["params"];
        match request["method"].as_str().unwrap() {
            "session.head" => result(data("thalamus-run-endings-v2", &head_file(&self.0))),
            "role.describe" => result(data("runner-pages-opencode2-v1", "09-role-describe.json")),
            "session.read" => {
                assert!(
                    !SYNTHETIC.contains(&self.0.as_str()),
                    "{} has no read",
                    self.0
                );
                let facts = if params["include_display_facts"] == true {
                    "-display-facts"
                } else {
                    ""
                };
                let mut read = data(
                    "thalamus-run-endings-v2",
                    &format!("{}-session-read{facts}.json", self.0),
                );
                let from = params["from_ordinal"].as_u64().unwrap_or(1);
                let limit = params["limit"].as_u64().unwrap_or(u64::MAX) as usize;
                let messages: Vec<Value> = read["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|entry| entry["ordinal"].as_u64().unwrap() >= from)
                    .take(limit)
                    .cloned()
                    .collect();
                read["messages"] = json!(messages);
                result(read)
            }
            method => panic!("the recorded runner was asked {method}"),
        }
    }

    async fn subscribe(&self, _live_ids: bool) -> Result<Box<dyn EventStream>, ReadError> {
        Ok(Box::new(Quiet))
    }
}

/// The ending a session names: session `ses_<case>` is that case's replies.
struct Endings;

#[async_trait]
impl BrocaRoutes for Endings {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "opencode2");
        let case = resolved.reference.session.strip_prefix("ses_").unwrap();
        Ok(Arc::new(Ending(case.to_owned())))
    }
}

/// Entorhinal's answer for any of these sessions: an OpenCode 2 head on Thalamus.
struct AnyOpenCode2Session;

#[async_trait]
impl Entorhinal for AnyOpenCode2Session {
    async fn request(&self, _: &str, params: Value) -> Result<Value, ReadError> {
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref",
            "incarnation":"entorhinal-test", "ended_at_ms":null, "runner":THALAMUS_RUNNER_MODULE_ID,
            "session":params["session"], "harness":"opencode2", "project_root":"/workspace/head",
            "network":"unknown", "network_history":[]}}),
        )
    }
}

/// What the head's ending becomes for the apps: run id, state and reason, and
/// the error's class, retry delay and reset time, each only when the head has it.
fn expected_last_run(head: &Value) -> Value {
    let run = &head["last_run_state"];
    let mut expected = json!({"runId": run["run_id"], "state": run["state"]});
    if let Some(reason) = run.get("reason") {
        expected["reason"] = reason.clone();
    }
    if let Some(error) = head.get("last_run_error") {
        let mut app = json!({"class": error["class"]});
        if let Some(secs) = error.get("retry_after_secs") {
            app["retryAfterSecs"] = secs.clone();
        }
        if let Some(at) = error.get("resets_at_ms") {
            app["resetsAtMs"] = at.clone();
        }
        expected["error"] = app;
    }
    expected
}

#[tokio::test]
async fn the_run_ending_samples_are_what_the_handlers_answer() {
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(AnyOpenCode2Session))),
            Arc::new(Endings),
        ),
    )));
    let mut served = super::served_samples::Served::new(SET, app);
    let mut corr = 0;
    for case in RECORDED.iter().chain(SYNTHETIC.iter()) {
        let synthetic = SYNTHETIC.contains(case);
        let head = data("thalamus-run-endings-v2", &head_file(case));
        let expected = expected_last_run(&head);
        let session = json!({"session": format!("ses_{case}"), "harness": "opencode2"});
        let suffix = if synthetic { "-synthetic" } else { "" };

        corr += 1;
        let request = json!({"method": super::super::super::DETAILS, "params": session});
        let reply = served.read(corr, &request).await;
        let details = super::served_samples::decoded(&reply)["result"].clone();
        assert_eq!(details["lastRun"], expected, "{case}: {details}");
        served.keep(&format!("{case}-details{suffix}"), &request, reply);

        if synthetic {
            continue;
        }
        corr += 1;
        let mut params = session.clone();
        params["limit"] = json!(50);
        let request = json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": params});
        let reply = served.read(corr, &request).await;
        let page = super::served_samples::decoded(&reply)["result"].clone();
        assert_eq!(page["lastRun"], expected, "{case}: {page}");
        // Every recorded run has ended, so none leaves the chat looking busy.
        assert_eq!(page["activity"], "idle", "{case}: {page}");
        assert!(
            !page["messages"].as_array().unwrap().is_empty(),
            "{case}: {page}"
        );
        let marks: Vec<(String, Value)> = page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["parts"].as_array().cloned().unwrap_or_default())
            .filter(|part| part["type"] == "tool")
            .map(|part| {
                (
                    part["callId"].as_str().unwrap().to_owned(),
                    part["unanswered"].clone(),
                )
            })
            .collect();
        let want = match *case {
            "02-max-steps" => vec![("call_probe".to_owned(), json!("not_executed"))],
            "05-completed-tool-not-offered" => vec![("call_absent".to_owned(), json!("denied"))],
            _ => marks
                .iter()
                .map(|(id, _)| (id.clone(), Value::Null))
                .collect(),
        };
        assert_eq!(
            marks, want,
            "{case}: a call no tool answered carries its reason"
        );
        served.keep(&format!("{case}-newest-page"), &request, reply);
    }
    served.check();
}
