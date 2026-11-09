//! Text a host writes into a user message, such as Claude Code's
//! `[Request interrupted by user]` after a stop, is not the person's words. A
//! runner names those parts by key in the read's `host_parts` display fact
//! (runner contract 0.8.33), optionally with a code for a marker it matched
//! exactly. The apps get each as an `injected` part marked `host`, with its
//! `hostCode`, and its text stays out of the row's `text`, which the phone
//! draws the person's bubble from.

use super::*;
use cortexkit_role_llm_runner::vectors;
use std::path::Path;

fn vector(name: &str) -> Value {
    serde_json::from_str(vectors::file(name).unwrap()).unwrap()
}

/// The app row for one read entry of a Claude Code session.
fn row(entry: &Value) -> Value {
    broca_entries::display_rows(
        std::slice::from_ref(entry),
        false,
        broca_entries::DisplayProfile::ClaudeCode,
        &[],
    )
    .unwrap()
    .remove(0)
    .display
}

/// Every host part in the contract's cases becomes a host `injected` part
/// with its code, when it has one, and the row's text keeps only the sender's
/// words. The text of a coded part is kept too, as the fallback for a code an
/// app doesn't know.
#[test]
fn the_contract_cases_serve_host_parts_apart_from_the_persons_words() {
    let file = vector("display-host-codes.json");
    let cases = file["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 4);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let row = row(&case["read"]);
        let expected: Vec<Value> = case["row"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|part| {
                let mut app = if part["host"] == true {
                    json!({"type": "injected", "text": part["text"], "host": true})
                } else {
                    json!({"type": "text", "text": part["text"]})
                };
                if let Some(code) = part.get("host_code") {
                    app["hostCode"] = code.clone();
                }
                app
            })
            .collect();
        let served: Vec<Value> = row["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|part| {
                let mut part = part.clone();
                for key in ["partIndex", "partKey"] {
                    part.as_object_mut().unwrap().remove(key);
                }
                part
            })
            .collect();
        assert_eq!(served, expected, "{name}");
        let words: String = case["row"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["host"] != true)
            .filter_map(|part| part["text"].as_str())
            .collect();
        assert_eq!(row["text"], words, "{name}");
    }
}

/// A fact naming no part of the entry marks nothing, and a fact on an entry
/// that isn't a user message is ignored: only a user message can hold text
/// the host wrote in its sender's place.
#[test]
fn a_fact_marks_only_a_user_part_it_names() {
    for (role, key) in [("user", "missing"), ("assistant", "0")] {
        let entry = json!({"ordinal": 1, "mid": "m1", "role": role,
            "message": {"role": role, "content": [{"type": "text", "text": "Words."}]},
            "host_parts": [{"part": key}]});
        let row = row(&entry);
        assert_eq!(row["parts"][0]["type"], "text", "{role}: {row}");
        assert_eq!(row["text"], "Words.", "{role}: {row}");
    }
}

/// Broca names its parts by served part key, and the same fact marks them.
#[test]
fn broca_entries_take_the_fact_by_part_key() {
    let entry = json!({"mid": "m1", "ordinal": 1, "part_keys": ["k-host", "k-words"],
        "message": {"role": "user", "content": [
            {"type": "text", "text": "[Request interrupted by user]\n"},
            {"type": "text", "text": "Go on."}]},
        "host_parts": [{"part": "k-host", "host_code": "interrupted"}]});
    let row = broca_entries::rows(&[entry], false)
        .unwrap()
        .remove(0)
        .display;
    assert_eq!(row["text"], "Go on.", "{row}");
    assert_eq!(
        (
            &row["parts"][0]["type"],
            &row["parts"][0]["host"],
            &row["parts"][0]["hostCode"]
        ),
        (&json!("injected"), &json!(true), &json!("interrupted"))
    );
}

/// In a real Claude Code session (Thalamus's recorded `runner-pages-claude-code-v1`
/// set, which `claude_code_pages` checks), the interrupt marker Claude Code
/// wrote before the person's next message leaves the bubble once the runner
/// names it.
#[test]
fn a_recorded_interrupt_marker_leaves_the_persons_bubble() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/data/runner-pages-claude-code-v1/02-tool-outcomes/session-read-media-refs.json",
    );
    let read: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut entry = read["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["ordinal"] == 14)
        .unwrap()
        .clone();
    assert_eq!(
        entry["message"]["content"][1]["text"],
        "[Request interrupted by user]\n"
    );
    assert!(row(&entry)["text"]
        .as_str()
        .unwrap()
        .starts_with("[Request interrupted by user]"));
    entry["host_parts"] = json!([{"part": "1", "host_code": "interrupted"}]);
    let row = row(&entry);
    assert_eq!(row["text"], "Call echo, then wait for it.", "{row}");
    assert_eq!(row["parts"][0]["hostCode"], "interrupted", "{row}");
}
