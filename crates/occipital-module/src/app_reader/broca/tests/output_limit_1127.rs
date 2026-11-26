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
//! `run.tool_calls` nor `run.result`; those bodies are kept so the checksums
//! verify whole.

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
