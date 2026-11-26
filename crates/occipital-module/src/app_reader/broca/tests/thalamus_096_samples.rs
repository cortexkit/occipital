//! Thalamus's samples for runner contract 0.9.6, copied unchanged into
//! `tests/data/`. Occipital builds rows from the `session.read` entries alone;
//! the display entries are kept so the checksums verify whole, and give the
//! words each row must carry.
//!
//! - `thalamus-steer-frame-v2`: six messages a person sent while a real
//!   Claude Code 2.1.331 turn ran, each read with its `host_frame` fact. Each
//!   reaches the apps as the person's row: their words as its text, and
//!   Claude Code's two wrapper sentences as host parts. Two of the messages
//!   copy a wrapper sentence into the words; those copies stay words.
//! - `thalamus-runner-entry-v1`: the empty reply Thalamus writes when the
//!   host stored none, marked `runner_entry`, and a Pi model's own empty
//!   reply, which carries no mark.

use super::*;
use std::path::{Path, PathBuf};

fn root(set: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(set)
}

fn read(set: &str, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root(set).join(name)).unwrap()).unwrap()
}

fn row(entry: Value, profile: broca_entries::DisplayProfile) -> Value {
    broca_entries::display_rows(&[entry], false, profile, &[])
        .unwrap()
        .remove(0)
        .display
}

/// Every file of `set` matches its `SHA256SUMS`, whose own digest starts with
/// `sums`, the value Thalamus published with the set.
fn check_sums(set: &str, sums: &str, files: usize) {
    let listed = std::fs::read(root(set).join("SHA256SUMS")).unwrap();
    assert!(
        format!("{:x}", Sha256::digest(&listed)).starts_with(sums),
        "{set}"
    );
    let listed = String::from_utf8(listed).unwrap();
    assert_eq!(listed.lines().count(), files, "{set}");
    for line in listed.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root(set).join(name)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            expected,
            "{set}/{name}"
        );
    }
}

#[test]
fn the_samples_keep_their_checksums() {
    check_sums("thalamus-steer-frame-v2", "7a77faed20eb30e2", 22);
    check_sums("thalamus-runner-entry-v1", "e30631b10cebcffe", 5);
}

#[test]
fn each_recorded_mid_turn_message_is_the_persons_words_between_two_host_parts() {
    let set = "thalamus-steer-frame-v2";
    for n in 1..=6 {
        let entry = read(set, &format!("steer-{n}/read-entry.json"));
        let display = read(set, &format!("steer-{n}/display-entry.json"));
        let words = display["row"]["parts"][1]["text"].clone();
        let head = display["row"]["parts"][0]["text"].clone();
        let tail = display["row"]["parts"][2]["text"].clone();
        let row = row(entry, broca_entries::DisplayProfile::ClaudeCode);
        assert_eq!(
            (&row["role"], &row["text"]),
            (&json!("user"), &words),
            "steer {n}"
        );
        assert_eq!(row["clientMessageId"], json!(format!("client-steer-{n}")));
        assert_eq!(
            row["parts"],
            json!([
                {"type": "injected", "text": head, "host": true,
                    "hostCode": "claude_code_mid_turn",
                    "partIndex": 0, "partKey": "0#host_frame:head"},
                {"type": "text", "text": words, "partIndex": 0, "partKey": "0"},
                {"type": "injected", "text": tail, "host": true,
                    "hostCode": "claude_code_mid_turn",
                    "partIndex": 0, "partKey": "0#host_frame:tail"},
            ]),
            "steer {n}"
        );
    }
}

#[test]
fn only_the_runner_written_empty_reply_carries_the_mark() {
    let set = "thalamus-runner-entry-v1";
    let written = row(
        read(set, "opencode2-read-entry.json"),
        broca_entries::DisplayProfile::OpenCode2,
    );
    assert_eq!(
        written["runnerEntry"],
        json!({"code": "host_stored_no_reply"})
    );
    assert_eq!(written["parts"], json!([]));
    let native = row(
        read(set, "pi-native-read-entry.json"),
        broca_entries::DisplayProfile::Pi,
    );
    assert_eq!(native["runnerEntry"], Value::Null);
    assert_eq!(native["parts"], json!([]));
}
