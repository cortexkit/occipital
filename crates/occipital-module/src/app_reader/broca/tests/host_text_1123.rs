//! Thalamus's samples for host-written user text marked on reads,
//! recorded from its committed tree and copied unchanged into
//! `tests/data/thalamus-host-text-1123/`. Occipital builds rows from the
//! `session.read` entries alone; the display and `tag_read` bodies are kept so
//! the checksums verify whole.
//!
//! - OpenCode 2: a message OpenCode stores as `synthetic` (here a client's
//!   notice) is named host text with no code. It reaches the apps as a row
//!   holding only a host `injected` part, as every synthetic row already does.
//! - Claude Code: the note Claude Code writes after a safety stop is named
//!   host text with the code `safety_stop_note`. It reaches the apps as a host
//!   `injected` part carrying that `hostCode`, and stays out of the row's text,
//!   so it never draws as the person's words.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/thalamus-host-text-1123")
}

fn read(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

fn row(entry: Value, profile: broca_entries::DisplayProfile) -> Value {
    broca_entries::display_rows(&[entry], false, profile, &[])
        .unwrap()
        .remove(0)
        .display
}

#[test]
fn the_host_text_samples_keep_their_checksums() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("f7890d6d"));
    let sums = String::from_utf8(sums).unwrap();
    assert_eq!(sums.lines().count(), 6);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

#[test]
fn a_marked_opencode2_synthetic_message_is_a_host_only_row() {
    let entry = read("opencode2-read.json");
    let text = entry["message"]["text"].clone();
    let row = row(entry, broca_entries::DisplayProfile::OpenCode2);
    assert_eq!((&row["role"], &row["text"]), (&json!("user"), &json!("")));
    assert_eq!(
        row["parts"],
        json!([{"type": "injected", "text": text, "host": true, "partIndex": 0, "partKey": "0"}])
    );
}

#[test]
fn claude_codes_safety_stop_note_is_coded_host_text() {
    let entry = read("claude-code-read.json");
    let text = entry["message"]["content"].clone();
    let row = row(entry, broca_entries::DisplayProfile::ClaudeCode);
    assert_eq!((&row["role"], &row["text"]), (&json!("user"), &json!("")));
    assert_eq!(
        row["parts"],
        json!([{"type": "injected", "text": text, "host": true,
            "hostCode": "safety_stop_note", "partIndex": 0, "partKey": "0"}])
    );
}
