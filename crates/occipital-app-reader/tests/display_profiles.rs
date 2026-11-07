//! Chats on a runner whose stored bodies the runner contract projects to display
//! rows (OpenCode 2, Claude Code and Pi) reach the apps in the same app row shape
//! as Broca's.
//!
//! The bodies are the contract's own, with the checksums it publishes for them:
//! `display-opencode-v2.json` and `display-pi.json` from
//! cortexkit-role-llm-runner 0.8.0's `test-vectors/llm-runner-v1`, and
//! `display-anthropic-messages.json` (Claude Code's stored format) from 0.8.17's.
//! Each case pairs a stored body with the display row the contract projects from
//! it.

use std::path::{Path, PathBuf};

use occipital_app_reader::broca_entries::{
    display_rows, DisplayProfile, CALL_ID, PART_INDEX, PART_KEY,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const ANTHROPIC: &str = "display-anthropic-messages.json";

/// The folder of the contract release `file` was copied from.
fn folder(file: &str) -> PathBuf {
    let release = if file == ANTHROPIC { "0.8.17" } else { "0.8.0" };
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/llm-runner-{release}"))
}

fn cases(file: &str) -> Vec<Value> {
    let set: Value =
        serde_json::from_slice(&std::fs::read(folder(file).join(file)).unwrap()).unwrap();
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
    for (file, count) in [("display-pi.json", 2), (ANTHROPIC, 1)] {
        let root = folder(file);
        let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
        assert_eq!(sums.lines().count(), count);
        for line in sums.lines() {
            let (expected, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(root.join(name)).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
        }
    }
}

/// The app role a contract display row becomes: Pi's `toolResult`, and a Claude
/// Code user message holding only tool results, are the apps' `tool` role.
fn app_role(profile: DisplayProfile, row: &Value) -> String {
    let parts = row["parts"].as_array().unwrap();
    let only_results = !parts.is_empty() && parts.iter().all(|part| part["type"] == "tool_result");
    match (profile, row["role"].as_str().unwrap()) {
        (_, "toolResult") => "tool".into(),
        (DisplayProfile::ClaudeCode, "user") if only_results => "tool".into(),
        (_, role) => role.into(),
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
        (ANTHROPIC, DisplayProfile::ClaudeCode),
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
                assert_eq!(row.display["role"], app_role(profile, want), "{name}");
                assert_eq!(row.display["text"], want["text"], "{name}");
                if let Some(model) = want.get("model") {
                    // A served model isn't an app row member today.
                    assert!(model.is_string(), "{name}");
                }
            }
        }
    }
}

/// A file the person attached reaches the app as a `file` part with its name,
/// type, size and digest, beside the message's text or on its own. The
/// contract's display row names the type, size and digest only in the file's
/// media reference.
#[test]
fn an_attached_file_carries_its_name_type_size_and_digest() {
    for (file, profile, name, text, filename) in [
        (
            "display-opencode-v2.json",
            DisplayProfile::OpenCode2,
            "2.2.3-seq-54",
            "S0_TAIL_KEEP",
            Some("pixel.png"),
        ),
        (
            "display-opencode-v2.json",
            DisplayProfile::OpenCode2,
            "synthetic-existing-image-ref",
            "",
            Some("pixel.png"),
        ),
        (
            "display-pi.json",
            DisplayProfile::Pi,
            "recorded-user-image",
            "HEAD_FIXTURE array user",
            None,
        ),
    ] {
        let at = cases(file)
            .iter()
            .position(|case| case["name"] == name)
            .unwrap();
        let entry = &entries(file)[at..=at];
        let row = &display_rows(entry, false, profile, &[]).unwrap()[0];
        assert_eq!(row.display["role"], "user", "{name}");
        assert_eq!(row.display["text"], text, "{name}");
        let files: Vec<&Value> = row.display["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "file")
            .collect();
        assert_eq!(files.len(), 1, "{name}: {}", row.display);
        let mut want = json!({"type": "file", "mime": "image/png", "byteCount": 69,
            "sha256": "b1ff9c8ea3a780bad09b346c423d2d0e46815926879b18e841d928376a946640",
            PART_KEY: files[0][PART_KEY], PART_INDEX: files[0][PART_INDEX]});
        if let Some(filename) = filename {
            want["filename"] = json!(filename);
        }
        assert_eq!(files[0], &want, "{name}");
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
/// OpenCode 2's and Claude Code's aren't known to be, so without a served end it
/// stays unknown.
#[test]
fn only_pi_dates_a_reply_by_its_write() {
    for (file, profile, dated) in [
        ("display-pi.json", DisplayProfile::Pi, true),
        ("display-opencode-v2.json", DisplayProfile::OpenCode2, false),
        (ANTHROPIC, DisplayProfile::ClaudeCode, false),
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

/// A served creation time is the row's start, exactly as recorded, on both
/// display-row runners: before the write, and kept even when it is later than
/// the served finish. Without one, the write time is held to the finish.
#[test]
fn a_served_creation_time_is_the_rows_start_as_recorded() {
    for (file, profile) in [
        ("display-opencode-v2.json", DisplayProfile::OpenCode2),
        ("display-pi.json", DisplayProfile::Pi),
    ] {
        let all = entries(file);
        let reply = display_rows(&all, false, profile, &[])
            .unwrap()
            .into_iter()
            .find(|row| row.display["role"] == "assistant")
            .unwrap();
        let mut entry = all
            .into_iter()
            .find(|entry| entry["mid"] == reply.mid.as_str())
            .unwrap();
        entry["written_at"] = json!(90_000);
        entry["completed_at"] = json!(80_000);
        let start = |entry: &Value| {
            display_rows(std::slice::from_ref(entry), false, profile, &[]).unwrap()[0].written_at
        };
        assert_eq!(
            start(&entry),
            Some(80_000),
            "{file}: write held to the finish"
        );
        entry["created_at"] = json!(20_000);
        assert_eq!(start(&entry), Some(20_000), "{file}: served start");
        entry["created_at"] = json!(85_000);
        assert_eq!(start(&entry), Some(85_000), "{file}: kept as recorded");
    }
}

/// Claude Code stores a tool's answer the Messages API way: a `tool_use` block
/// in the assistant's message, its `tool_result` in the next user message. The
/// answer folds into the call, and the user message that holds only results is
/// a `tool` row with nothing of its own on a page, never a message from the
/// person. A user message with the person's text beside a result stays theirs.
#[test]
fn a_claude_code_tool_result_is_the_tools_not_the_persons() {
    let entry = |ordinal: usize, message: Value| {
        json!({"mid": format!("m{ordinal}"), "ordinal": ordinal, "written_at": 1_000 + ordinal,
            "message": message})
    };
    let entries = [
        entry(1, json!({"role": "user", "content": "Read the notes"})),
        entry(
            2,
            json!({"role": "assistant", "content": [
            {"type": "text", "text": "Reading."},
            {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"file_path": "/w/notes.md"}}]}),
        ),
        entry(
            3,
            json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "line one"}]}),
        ),
        entry(
            4,
            json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_2", "name": "Read", "input": {"file_path": "/w/gone.md"}}]}),
        ),
        entry(
            5,
            json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_2", "is_error": true,
                "content": "File does not exist."},
            {"type": "text", "text": "Then check the other folder"}]}),
        ),
    ];
    let rows = display_rows(&entries, false, DisplayProfile::ClaudeCode, &[]).unwrap();
    let roles: Vec<_> = rows.iter().map(|row| row.display["role"].clone()).collect();
    assert_eq!(roles, ["user", "assistant", "tool", "assistant", "user"]);
    let read = &rows[1].display["parts"][1];
    assert_eq!(
        (&read["type"], &read["tool"]),
        (&json!("tool"), &json!("Read")),
        "{read}"
    );
    assert_eq!(read["outputBytes"], "line one".len(), "{read}");
    assert!(read.get("isError").is_none(), "{read}");
    assert_eq!(rows[2].display["parts"], json!([]));
    assert_eq!(rows[3].display["parts"][0]["isError"], true);
    assert_eq!(rows[4].display["text"], "Then check the other folder");
    let parts = rows[4].display["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 1, "{parts:?}");
    // An exact read of the tool row shows the answer itself.
    let full = display_rows(&entries, true, DisplayProfile::ClaudeCode, &[]).unwrap();
    assert_eq!(full[1].display["parts"][1]["content"], "line one");
}
