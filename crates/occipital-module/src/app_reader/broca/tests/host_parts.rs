//! Text a host writes into a user message, such as Claude Code's
//! `[Request interrupted by user]` after a stop, is not the person's words. A
//! runner names those parts by key in the read's `host_parts` display fact
//! (runner contract 0.8.33), optionally with a code for a marker it matched
//! exactly. The apps get each as an `injected` part marked `host`, with its
//! `hostCode`, and its text stays out of the row's `text`, which the phone
//! draws the person's bubble from.
//!
//! A host can also write parts into a reply (runner contract 0.8.46): the
//! empty reasoning blocks that record that a fallback model answered. Those
//! are bookkeeping, not anyone's words, so the apps don't get them at all.
//! The runner's `model_fallback` fact names both models instead, and the row
//! carries it as `modelFallback` for a quiet note under the reply.

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

/// The contract's cases: in a user message, every host part becomes a host
/// `injected` part with its code, when it has one, and the row's text keeps
/// only the sender's words. The text of a coded part is kept too, as the
/// fallback for a code an app doesn't know. In a reply, a host part is left
/// out, and `model_fallback` reaches the row as `modelFallback` only when
/// the read names it: a boundary marker that doesn't parse names none.
#[test]
fn the_contract_cases_serve_host_parts_apart_from_the_persons_words() {
    let file = vector("display-host-codes.json");
    let cases = file["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 7);
    let mut fallbacks = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let row = row(&case["read"]);
        let reply = case["read"]["role"] == "assistant";
        let fallback = &case["row"]["model_fallback"];
        assert_eq!(fallback, &case["read"]["model_fallback"], "{name}");
        if fallback.is_null() {
            assert!(row.get("modelFallback").is_none(), "{name}: {row}");
        } else {
            fallbacks += 1;
            assert_eq!(&row["modelFallback"], fallback, "{name}: {row}");
        }
        let expected: Vec<Value> = case["row"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| !(reply && part["host"] == true))
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
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect();
        assert_eq!(row["text"], words, "{name}");
    }
    // The parsed boundary marker and the runner's own record.
    assert_eq!(fallbacks, 2);
}

/// Broca names a reply's host parts by served part key too, and leaves the
/// reasoning the model wrote after them in place.
#[test]
fn a_broca_reply_drops_its_host_parts_and_keeps_the_rest() {
    let entry = json!({"mid": "m1", "ordinal": 1, "part_keys": ["k-mark", "k-think", "k-text"],
        "message": {"role": "assistant", "content": [
            {"type": "reasoning", "text": "\u{2060}",
                "signature": "cortexkit-server-fallback-v1:claude-fable-5-2|claude-opus-4-8"},
            {"type": "reasoning", "text": "Checking the file first."},
            {"type": "text", "text": "Done."}]},
        "host_parts": [{"part": "k-mark", "host_code": "server_fallback"}],
        "model_fallback": {"requested": "claude-fable-5-2", "answered": "claude-opus-4-8"}});
    let row = broca_entries::rows(&[entry], false)
        .unwrap()
        .remove(0)
        .display;
    let kinds: Vec<&Value> = row["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| &part["type"])
        .collect();
    assert_eq!(kinds, ["reasoning", "text"], "{row}");
    assert_eq!(row["parts"][0]["text"], "Checking the file first.", "{row}");
    assert_eq!(row["parts"][0]["partIndex"], 1, "{row}");
    assert_eq!(
        row["modelFallback"],
        json!({"requested": "claude-fable-5-2", "answered": "claude-opus-4-8"})
    );
}

/// A `model_fallback` the contract refuses (repeated or empty ids), or one on
/// a message that isn't a reply, costs only the note: the message still reads
/// whole.
#[test]
fn a_fallback_fact_that_doesnt_hold_is_left_out() {
    for (role, fallback) in [
        ("assistant", json!({"requested": "m", "answered": "m"})),
        ("assistant", json!({"requested": "", "answered": "m"})),
        ("assistant", json!("m")),
        ("user", json!({"requested": "a", "answered": "b"})),
    ] {
        let entry = json!({"ordinal": 1, "mid": "m1", "role": role,
            "message": {"role": role, "content": [{"type": "text", "text": "Words."}]},
            "model_fallback": fallback});
        for row in [
            row(&entry),
            broca_entries::rows(std::slice::from_ref(&entry), false)
                .unwrap()
                .remove(0)
                .display,
        ] {
            assert_eq!(row["text"], "Words.", "{role} {fallback}: {row}");
            assert!(
                row.get("modelFallback").is_none(),
                "{role} {fallback}: {row}"
            );
        }
    }
}

/// A fact naming no part of the entry marks nothing, and a fact on an entry
/// that is neither a user message nor a reply is ignored.
#[test]
fn a_fact_marks_only_a_part_it_names() {
    for (role, key) in [("user", "missing"), ("assistant", "missing")] {
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

/// OpenCode 2 writes some text into a conversation itself, as its own native
/// `synthetic` message, which the runner contract counts as host text whole:
/// it carries no `host_parts` fact. The apps get it in the shape of a person's
/// message that holds only host text: a `user` row with empty `text` whose
/// only part is a host `injected` part, with no code. The phone draws nothing
/// for that row and the desktop hides it, so the text is never anyone's words,
/// unless a code tells it what to say.
/// OpenCode 2's other native roles keep their names.
#[test]
fn an_opencode2_synthetic_message_is_host_text() {
    let entry = |kind: &str| {
        json!({"ordinal": 1, "mid": "m1",
            "message": {"id": "msg_1", "type": kind, "text": "Continue if you have next steps."}})
    };
    let row = |kind: &str| {
        broca_entries::display_rows(
            &[entry(kind)],
            false,
            broca_entries::DisplayProfile::OpenCode2,
            &[],
        )
        .unwrap()
        .remove(0)
        .display
    };
    let synthetic = row("synthetic");
    assert_eq!(
        (&synthetic["role"], &synthetic["text"]),
        (&json!("user"), &json!("")),
        "{synthetic}"
    );
    assert_eq!(
        synthetic["parts"],
        json!([{"type": "injected", "text": "Continue if you have next steps.",
            "host": true, "partIndex": 0, "partKey": "0"}])
    );
    for kind in ["system", "skill"] {
        let row = row(kind);
        assert_eq!(row["role"], kind, "{row}");
        assert_eq!(row["parts"][0]["type"], "text", "{row}");
    }
}

/// A `host_parts` fact on an OpenCode 2 `synthetic` message names a code for
/// the wording, such as `continued_after_interruption` for the fixed text
/// OpenCode writes to make the model go on after an interrupted reply. The
/// row stays host text either way; a fact naming no part of it adds nothing.
#[test]
fn an_opencode2_synthetic_message_takes_its_code_from_the_fact() {
    let text = "The previous response was interrupted. Continue from where you left off \
        without repeating completed content.";
    let row = |facts: Value| {
        let entry = json!({"ordinal": 1, "mid": "m1", "host_parts": facts,
            "message": {"id": "msg_1", "type": "synthetic", "text": text}});
        broca_entries::display_rows(
            &[entry],
            false,
            broca_entries::DisplayProfile::OpenCode2,
            &[],
        )
        .unwrap()
        .remove(0)
        .display
    };
    let coded = row(json!([{"part": "0", "host_code": "continued_after_interruption"}]));
    assert_eq!(
        (&coded["role"], &coded["text"]),
        (&json!("user"), &json!("")),
        "{coded}"
    );
    assert_eq!(
        coded["parts"],
        json!([{"type": "injected", "text": text, "host": true,
            "hostCode": "continued_after_interruption", "partIndex": 0, "partKey": "0"}])
    );
    for facts in [
        json!([]),
        json!([{"part": "7", "host_code": "continued_after_interruption"}]),
    ] {
        let row = row(facts);
        assert_eq!(row["parts"][0]["host"], true, "{row}");
        assert!(row["parts"][0].get("hostCode").is_none(), "{row}");
    }
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
