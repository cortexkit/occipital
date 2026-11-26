//! Thalamus's wave-1 samples for its run-answer fix, recorded from its
//! committed tree and copied unchanged into
//! `tests/data/thalamus-host-parity-wave1/` with its README.
//!
//! A completed run's `final_message` now always names the last model step's
//! assistant message. When the host stored none, the runner writes an empty
//! assistant entry of its own (`thalamus-final-…`, no content, marked as the
//! runner's). Occipital builds rows from the `session.read` pages; the run
//! results, step bodies and display history are kept so the checksums verify
//! whole. A Pi reply that really was empty (`pi-native-empty-answer`) sits
//! beside them, to compare.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/thalamus-host-parity-wave1")
}

fn read(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

fn rows(name: &str, profile: broca_entries::DisplayProfile) -> Vec<Value> {
    let page = read(name);
    let entries = page["messages"].as_array().unwrap().clone();
    broca_entries::display_rows(&entries, false, profile, &[])
        .unwrap()
        .into_iter()
        .map(|row| json!({"mid": row.mid, "display": row.display}))
        .collect()
}

#[test]
fn the_wave1_samples_keep_their_checksums() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("19af7847"));
    let sums = String::from_utf8(sums).unwrap();
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The runner's empty entry reads as an assistant row with no parts and no
/// text, after the rows before it, which read as they always did.
#[test]
fn the_runners_empty_final_entry_is_an_empty_assistant_row() {
    for (name, profile, before) in [
        (
            "opencode2-session-read-display-empty.json",
            broca_entries::DisplayProfile::OpenCode2,
            vec!["hello"],
        ),
        (
            "claude-code-session-read-display-empty.json",
            broca_entries::DisplayProfile::ClaudeCode,
            vec!["synthetic please", "No response requested."],
        ),
    ] {
        let rows = rows(name, profile);
        let (last, earlier) = rows.split_last().unwrap();
        let texts: Vec<&str> = earlier
            .iter()
            .map(|row| row["display"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, before, "{name}");
        assert!(
            last["mid"].as_str().unwrap().starts_with("thalamus-final-"),
            "{name}"
        );
        let display = &last["display"];
        assert_eq!(
            (&display["role"], &display["parts"], &display["text"]),
            (&json!("assistant"), &json!([]), &json!("")),
            "{name}"
        );
    }
}

/// A Pi session's run status (Pi is one of the hosts Thalamus runs) names
/// its provider error with the contract's `provider_error`, with
/// the provider's own code in `provider_detail_code`, so a send whose run
/// ended on it is listed with a code the apps can word.
#[test]
fn pis_provider_error_names_a_contract_code() {
    let status = read("pi-run-status-provider-error.json");
    let code = status["error"]["provider_code"].as_str().unwrap();
    assert_eq!(code, "provider_error");
    assert!(cortexkit_role_llm_runner::errors::provider_codes::CODES.contains(&code));
}

/// A Pi reply that really was empty reads exactly like the runner's own empty
/// entry: an assistant row with no parts and no text. Nothing on the row tells
/// the two apart yet.
#[test]
fn a_real_empty_pi_reply_reads_like_the_runners_entry() {
    let rows = rows(
        "pi-session-read-display-empty-native.json",
        broca_entries::DisplayProfile::Pi,
    );
    let last = rows.last().unwrap();
    assert_eq!(last["mid"], "pi-native-empty-answer");
    let display = &last["display"];
    assert_eq!(
        (&display["role"], &display["parts"], &display["text"]),
        (&json!("assistant"), &json!([]), &json!(""))
    );
}
