//! Chats on a runner whose stored bodies the runner contract projects to display
//! rows (OpenCode 2 and Pi) reach the apps in the same app row shape as Broca's.
//!
//! The bodies are the contract's own: `display-opencode-v2.json` and
//! `display-pi.json` from cortexkit-role-llm-runner 0.8.0's
//! `test-vectors/llm-runner-v1`, with the checksums it publishes for them. Each
//! case pairs a stored body with the display row the contract projects from it.

use std::path::{Path, PathBuf};

use occipital_app_reader::broca_entries::{
    display_rows, DisplayProfile, CALL_ID, PART_INDEX, PART_KEY,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/llm-runner-0.8.0")
}

fn cases(file: &str) -> Vec<Value> {
    let set: Value = serde_json::from_slice(&std::fs::read(root().join(file)).unwrap()).unwrap();
    set["rows"].as_array().unwrap().clone()
}

/// One read entry per contract case, in case order, as a runner serves it.
fn entries(file: &str) -> Vec<Value> {
    cases(file)
        .iter()
        .enumerate()
        .map(|(ordinal, case)| {
            json!({"mid": format!("m{ordinal}"), "ordinal": ordinal + 1, "written_at": 1_000 + ordinal,
                "message": case["body"]})
        })
        .collect()
}

#[test]
fn the_contract_display_vectors_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 2);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Every app part carries the members the apps key on, and nothing in the
/// contract's own display shape: a part is `tool`, never `tool_call` or
/// `tool_result`, and is keyed by `partKey`, never `part`.
fn assert_app_shape(display: &Value, name: &str) {
    for member in ["role", "text", "parts", "blockSummaries"] {
        assert!(display.get(member).is_some(), "{name}: no {member}");
    }
    for part in display["parts"].as_array().unwrap() {
        assert!(part.get("part").is_none(), "{name}: contract key {part}");
        assert!(
            !matches!(part["type"].as_str(), Some("tool_call" | "tool_result")),
            "{name}: contract part {part}"
        );
        assert!(part[PART_KEY].is_string(), "{name}: {part}");
        assert!(part[PART_INDEX].is_u64(), "{name}: {part}");
    }
}

#[test]
fn every_contract_case_becomes_an_app_row() {
    for (file, profile) in [
        ("display-opencode-v2.json", DisplayProfile::OpenCode2),
        ("display-pi.json", DisplayProfile::Pi),
    ] {
        let cases = cases(file);
        for full in [false, true] {
            let rows = display_rows(&entries(file), full, profile, &[]).unwrap();
            assert_eq!(rows.len(), cases.len(), "{file}");
            for (row, case) in rows.iter().zip(&cases) {
                let name = format!("{file}: {}", case["name"]);
                assert!(row.display.get("unreadable").is_none(), "{name}");
                assert_app_shape(&row.display, &name);
                let want = &case["row"];
                let role = match want["role"].as_str().unwrap() {
                    "toolResult" => "tool",
                    role => role,
                };
                assert_eq!(row.display["role"], role, "{name}");
                assert_eq!(row.display["text"], want["text"], "{name}");
                if let Some(model) = want.get("model") {
                    // A served model isn't an app row member today.
                    assert!(model.is_string(), "{name}");
                }
            }
        }
    }
}

/// A tool call and its result become one app tool part, keyed by the call's
/// part key and id, with the result's length and, on an exact read, its text.
#[test]
fn an_opencode_2_tool_call_and_its_result_are_one_tool_part() {
    let file = "display-opencode-v2.json";
    let cases = cases(file);
    let index = cases
        .iter()
        .position(|case| {
            let parts = case["row"]["parts"].as_array().unwrap();
            parts.iter().any(|part| part["type"] == "tool_call")
                && parts.iter().any(|part| part["type"] == "tool_result")
        })
        .unwrap();
    let want = &cases[index]["row"]["parts"];
    let call = want
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["type"] == "tool_call")
        .unwrap();
    let result = want
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["type"] == "tool_result")
        .unwrap();
    let entries = entries(file);
    let row = &display_rows(
        &entries[index..=index],
        false,
        DisplayProfile::OpenCode2,
        &[],
    )
    .unwrap()[0];
    let tools: Vec<&Value> = row.display["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "tool")
        .collect();
    assert_eq!(tools.len(), 1, "{}", row.display);
    let tool = tools[0];
    assert_eq!(tool["tool"], call["name"]);
    assert_eq!(tool[CALL_ID], call["tool_call_id"]);
    assert_eq!(tool[PART_KEY], call["part"]);
    assert_eq!(tool["arguments"], call["arguments"]);
    assert_eq!(
        tool["outputBytes"],
        result["content"].as_str().unwrap().len()
    );
    assert_eq!(row.display["toolCalls"][0][CALL_ID], call["tool_call_id"]);
    assert_eq!(row.display["blockSummaries"], json!([call["name"]]));

    // On an exact read too, the result is inside its call, not a part of its own.
    let exact = &display_rows(
        &entries[index..=index],
        true,
        DisplayProfile::OpenCode2,
        &[],
    )
    .unwrap()[0];
    let tools: Vec<&Value> = exact.display["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "tool")
        .collect();
    assert_eq!(tools.len(), 1, "{}", exact.display);
    assert_eq!(tools[0]["content"], result["content"]);
}

/// A Pi tool result, stored as its own message, folds into the call in an
/// earlier message, as Broca's do; the result's own row then shows no tool part.
#[test]
fn a_pi_tool_result_folds_into_its_call_in_an_earlier_message() {
    let file = "display-pi.json";
    let cases = cases(file);
    let call = cases
        .iter()
        .position(|case| {
            case["row"]["parts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|part| part["type"] == "tool_call")
        })
        .unwrap();
    let id = cases[call]["row"]["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["type"] == "tool_call")
        .unwrap()["tool_call_id"]
        .clone();
    let result = cases
        .iter()
        .position(|case| {
            case["row"]["parts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|part| part["type"] == "tool_result" && part["tool_call_id"] == id)
        })
        .unwrap();
    assert!(result > call);
    let rows = display_rows(&entries(file), false, DisplayProfile::Pi, &[]).unwrap();
    let tool = rows[call].display["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["type"] == "tool")
        .unwrap()
        .clone();
    assert_eq!(tool[CALL_ID], id);
    assert!(tool["outputBytes"].is_u64(), "{tool}");
    assert_eq!(rows[result].display["role"], "tool");
    assert!(!rows[result].display["parts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["type"] == "tool"));
}

/// A Pi assistant reply is written when it ends, so its write time is its end.
/// OpenCode 2's isn't known to be, so without a served end it stays unknown.
#[test]
fn only_pi_dates_a_reply_by_its_write() {
    for (file, profile, dated) in [
        ("display-pi.json", DisplayProfile::Pi, true),
        ("display-opencode-v2.json", DisplayProfile::OpenCode2, false),
    ] {
        let rows = display_rows(&entries(file), false, profile, &[]).unwrap();
        let reply = rows
            .iter()
            .find(|row| row.display["role"] == "assistant")
            .unwrap();
        assert_eq!(
            reply.display.get("completedAtMs").is_some(),
            dated,
            "{file}"
        );
        assert_eq!(reply.display.get("completion").is_some(), !dated, "{file}");
    }
}

/// A body the contract refuses to project stays visible as the unreadable
/// placeholder, and a served end time and app message id reach the row.
#[test]
fn refused_bodies_are_unreadable_and_served_facts_reach_the_row() {
    let compaction =
        json!({"mid": "c", "ordinal": 1, "message": {"type": "compaction", "id": "x", "data": {}}});
    let rows = display_rows(&[compaction], false, DisplayProfile::OpenCode2, &[]).unwrap();
    assert_eq!(rows[0].display["unreadable"], true);
    assert_eq!(rows[0].mid, "c");

    let mut entry = entries("display-opencode-v2.json")
        .into_iter()
        .find(|entry| entry["message"]["type"] == "assistant")
        .unwrap();
    entry["completed_at"] = json!(4_242);
    entry["client_message_id"] = json!("phone-1");
    let rows = display_rows(&[entry], false, DisplayProfile::OpenCode2, &[]).unwrap();
    assert_eq!(rows[0].display["completedAtMs"], 4_242);
    assert_eq!(rows[0].display["clientMessageId"], "phone-1");
}
