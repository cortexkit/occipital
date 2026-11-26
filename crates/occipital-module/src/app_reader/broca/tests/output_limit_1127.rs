//! Thalamus's samples for its runaway-reply rules (runner contract 0.10.1),
//! recorded from its own serializers and copied unchanged into
//! `tests/data/thalamus-output-limit-v1/` with its README.
//!
//! A reply cut off at its output limit leaves its tool calls unexecuted, and
//! the display facts name why with an added `reason` beside `unanswered`. A
//! run stopped for that, or for repeating one call, ends in an error coded
//! `output_limit` or `tool_loop`. Occipital builds rows from the
//! `session.read` page, which reads as before: each cut call is an unanswered
//! call and the extra member is ignored. Occipital reads neither
//! `run.tool_calls` nor `run.result`, but a run's error reaches the apps from
//! the session's last run, which carries the same error, so the two
//! `run.result` errors stand in for it there.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/thalamus-output-limit-v1")
}

#[test]
fn the_output_limit_samples_keep_their_checksums() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("cff29d92"));
    let sums = String::from_utf8(sums).unwrap();
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The sample set the apps pin for runs the runner stopped. Its unsent
/// messages are written by the send tests' `run_stopped_1127`.
const STOPPED_SET: &str = "phone-reader-run-stopped-v1";

/// `value` as file `name` of [`STOPPED_SET`]: written there when
/// `OCCIPITAL_WRITE_SAMPLES` names a folder, else compared with the published
/// file.
fn stopped_sample(name: &str, value: &Value) {
    if let Some(folder) = std::env::var_os("OCCIPITAL_WRITE_SAMPLES") {
        let folder = PathBuf::from(folder).join(STOPPED_SET);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(name), serde_json::to_vec(value).unwrap()).unwrap();
        return;
    }
    let published = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../occipital-app-reader/test-vectors")
        .join(STOPPED_SET)
        .join(name);
    let published: Value = serde_json::from_slice(&std::fs::read(published).unwrap()).unwrap();
    assert_eq!(value, &published, "{name}");
}

/// The run errors Thalamus recorded for a run it stopped, by code.
fn stopped_run_error(code: &str) -> Value {
    let file = match code {
        "output_limit" => "run-result-output-limit.json",
        _ => "run-result-tool-loop.json",
    };
    serde_json::from_slice(&std::fs::read(root().join(file)).unwrap()).unwrap()
}

/// A run stopped for its output limit or for a tool loop names its code to
/// the apps as `providerCode`, as every code the runner contract names does.
/// The runner's message stays out. Each `lastRun` is a file of
/// [`STOPPED_SET`].
#[tokio::test]
async fn a_run_stopped_for_its_output_limit_or_a_tool_loop_names_its_code() {
    for (sample, code) in [
        ("01-last-run-output-limit.json", "output_limit"),
        ("02-last-run-tool-loop.json", "tool_loop"),
    ] {
        let error = stopped_run_error(code)["result"]["error"].clone();
        assert_eq!(error["provider_code"], code, "{sample}");
        let expected = format!(
            r#"{{"error":{{"class":"permanent","providerCode":"{code}"}},"reason":"error","runId":"r-cut","state":"error"}}"#
        );
        super::session_facts::assert_last_run_phone_bytes(
            json!({"run_id": "r-cut", "state": "error", "reason": "error"}),
            Some(error),
            &expected,
        )
        .await;
        let app = json!({"lastRun": serde_json::from_str::<Value>(&expected).unwrap()});
        stopped_sample(sample, &app);
    }
}

/// Every call a cut reply left unexecuted reads as unanswered
/// (`not_executed`), whether or not the runner adds its `reason`, and the
/// one call that ran reads as answered.
#[test]
fn calls_a_cut_reply_left_unexecuted_read_as_unanswered() {
    let page: Value =
        serde_json::from_slice(&std::fs::read(root().join("session-read.json")).unwrap()).unwrap();
    let entries = page["result"]["messages"].as_array().unwrap().clone();
    let rows = broca_entries::display_rows(
        &entries,
        false,
        broca_entries::DisplayProfile::OpenCode2,
        &[],
    )
    .unwrap();
    let calls: Vec<(String, Value)> = rows
        .iter()
        .flat_map(|row| row.display["parts"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["type"] == "tool")
        .map(|part| {
            (
                part["callId"].as_str().unwrap().to_owned(),
                part["unanswered"].clone(),
            )
        })
        .collect();
    let not_executed = json!("not_executed");
    assert_eq!(
        calls,
        [
            ("call-0".to_owned(), not_executed.clone()),
            ("call-1".to_owned(), not_executed.clone()),
            ("call-2".to_owned(), Value::Null),
            ("call-3".to_owned(), not_executed.clone()),
            ("call-4".to_owned(), not_executed.clone()),
            ("call-5".to_owned(), not_executed),
        ],
        "{}",
        serde_json::to_string(&rows.iter().map(|row| &row.display).collect::<Vec<_>>()).unwrap()
    );
}
