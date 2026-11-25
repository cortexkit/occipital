//! Runner contract 0.9.7's list of host frames (llm-runner §9.7.1.1), and the
//! sample set the apps pin for it, `phone-reader-host-frames-v2`, served
//! through Occipital's own handlers for a Claude Code chat as a phone's route
//! reaches it. `served_samples` says how a set is checked and rewritten.
//!
//! From Claude Code 2.1.332 one stored message can hold several messages the
//! person sent during one tool call, each wrapped in Claude Code's own two
//! sentences, beside the tool's result. The read names each wrapper in its
//! `host_frames` list, and the row lists each message's words part in
//! `hostFrames`, so the apps draw each as its own message. A row framed by the
//! single `host_frame` an older runner serves gets a list of one, so the apps
//! follow one rule. The set replaces `phone-reader-host-frame-v1`, whose rows
//! had no list.

use super::claude_code_samples::{AnyHarness, ClaudeCodeHead};
use super::served_samples::{decoded, Served};
use super::*;
use cortexkit_role_llm_runner::attribution::host_frames::{
    CLAUDE_CODE_MID_TURN, CLAUDE_CODE_MID_TURN_HEAD, CLAUDE_CODE_MID_TURN_TAIL,
    CLAUDE_CODE_MID_TURN_TAIL_FOLLOWED,
};

/// A sample read entry from the contract's `steer-frame-claude-code-v2`.
fn sample(name: &str) -> Value {
    let name = format!("steer-frame-claude-code-v2/{name}.json");
    serde_json::from_str(cortexkit_role_llm_runner::vectors::file(&name).unwrap()).unwrap()
}

/// Recorded mid-turn message `n` (1 to 6) from Claude Code 2.1.331, as a
/// runner serves it with the single `host_frame` fact.
fn recorded(n: usize) -> Value {
    let name = format!("steer-frame-claude-code-v1/steer-{n}/read-entry.json");
    let mut entry: Value =
        serde_json::from_str(cortexkit_role_llm_runner::vectors::file(&name).unwrap()).unwrap();
    entry["host_frame"] = frame(0, false);
    entry
}

/// The fact naming Claude Code's wrapper around stored part `part`. A wrapper
/// with another one after it in the same message ends in one more line break.
fn frame(part: u32, followed: bool) -> Value {
    let tail = if followed {
        CLAUDE_CODE_MID_TURN_TAIL_FOLLOWED
    } else {
        CLAUDE_CODE_MID_TURN_TAIL
    };
    json!({"part": part, "code": CLAUDE_CODE_MID_TURN,
        "head_bytes": CLAUDE_CODE_MID_TURN_HEAD.len(), "tail_bytes": tail.len()})
}

fn row(entry: Value) -> Value {
    broca_entries::display_rows(
        &[entry],
        false,
        broca_entries::DisplayProfile::ClaudeCode,
        &[],
    )
    .unwrap()
    .remove(0)
    .display
}

/// The `hostFrames` list a row serves for words in stored parts `parts`.
fn listed(parts: &[&str]) -> Value {
    parts
        .iter()
        .map(|part| json!({"partKey": part, "code": CLAUDE_CODE_MID_TURN}))
        .collect()
}

/// Each part's key and whether a host wrote it.
fn keys(row: &Value) -> Vec<(String, bool)> {
    row["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| {
            (
                part["partKey"].as_str().unwrap().to_owned(),
                part["host"] == true,
            )
        })
        .collect()
}

/// Two messages sent during one tool call: each framed part is split into the
/// two host sentences around the person's words, the list names both words
/// parts in stored order, and the row's text joins only the words. The tool
/// result is not one of them.
#[test]
fn two_frames_are_two_messages_in_one_row() {
    let row = row(sample("two-frames-read"));
    assert_eq!(row["role"], json!("user"));
    assert_eq!(row["hostFrames"], listed(&["1", "2"]));
    assert_eq!(
        row["text"],
        json!("Try the blue dial. Leave the brass latch open.")
    );
    let framed: Vec<(String, bool)> = keys(&row)
        .into_iter()
        .filter(|(key, _)| key.starts_with('1') || key.starts_with('2'))
        .collect();
    assert_eq!(
        framed,
        [
            ("1#host_frame:head".to_owned(), true),
            ("1".to_owned(), false),
            ("1#host_frame:tail".to_owned(), true),
            ("2#host_frame:head".to_owned(), true),
            ("2".to_owned(), false),
            ("2#host_frame:tail".to_owned(), true),
        ]
    );
}

/// A list of one, and the single fact an older runner serves, both give a
/// list of one entry.
#[test]
fn one_frame_is_a_list_of_one() {
    assert_eq!(row(sample("one-frame-read"))["hostFrames"], listed(&["1"]));
    assert_eq!(row(recorded(1))["hostFrames"], listed(&["0"]));
}

/// Each frame is checked on its own against the stored text: one wrong byte
/// in the second wrapper leaves that part whole, and the first still splits,
/// whether the runner served only the first or both candidates.
#[test]
fn a_frame_that_fails_costs_only_itself() {
    for name in ["bad-tail-read", "bad-tail-candidates"] {
        let row = row(sample(name));
        assert_eq!(row["hostFrames"], listed(&["1"]), "{name}");
        let second = row["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["partKey"] == "2")
            .unwrap();
        assert!(
            second["text"]
                .as_str()
                .unwrap()
                .starts_with(CLAUDE_CODE_MID_TURN_HEAD),
            "{name}"
        );
    }
}

/// Facts the contract refuses together read as absent, and the message shows
/// as stored: a list naming one part twice, and both the list and the single
/// fact on one entry.
#[test]
fn refused_frames_leave_the_message_as_stored() {
    let mut both = sample("one-frame-read");
    both["host_frame"] = frame(1, false);
    for (name, entry) in [("refused", sample("refused-read")), ("both", both)] {
        let row = row(entry);
        assert_eq!(row["hostFrames"], Value::Null, "{name}");
        assert!(keys(&row).iter().all(|(_, host)| !host), "{name}");
    }
}

/// A Claude Code chat holding `entries`, served on the phone's route.
fn chat(entries: Vec<Value>) -> Arc<super::super::super::AppReader> {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries;
    *route.run_state.lock().unwrap() = Value::Null;
    Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(ClaudeCodeHead))),
            Arc::new(AnyHarness(route.clone())),
        ),
    )))
}

/// `phone-reader-host-frames-v2` (v1's Simple page names Simple rule 2), in
/// stored order:
/// - rows 1 to 6: the six messages Claude Code 2.1.331 recorded, each with the
///   single `host_frame` fact, as `phone-reader-host-frame-v1` holds them;
/// - row 7: the reply whose tool call the next rows' results answer;
/// - rows 8 to 11: the contract's 2.1.332 cases, each a tool result beside
///   framed parts (two frames, one frame, a second frame that fails, and a
///   list the contract refuses);
/// - row 12: two messages in one tool call where the first is a wake notice
///   a module sent, which the Simple view leaves out;
/// - row 13: row 1's message as an older runner serves it, with no fact.
///
/// Every message names this module as its sender, as one typed in the apps
/// does. The set holds the newest page and the same page in the Simple view.
#[tokio::test]
async fn the_host_frames_samples_are_what_the_reader_serves() {
    let from_the_apps = |mut entry: Value, ordinal: u64, mid: &str| {
        entry["sender"] = json!(crate::MODULE_ID);
        entry["ordinal"] = json!(ordinal);
        entry["mid"] = json!(mid);
        entry
    };
    let mut entries: Vec<Value> = (1..=6)
        .map(|n| from_the_apps(recorded(n), n as u64, &format!("steer-{n}")))
        .collect();
    entries.push(
        json!({"mid": "reply-dial", "ordinal": 7, "written_at": 1_800_000_000_000_u64,
        "message": {"role": "assistant", "content": [
            {"type": "text", "text": "Opening the dial."},
            {"type": "tool_use", "id": "call_dial", "name": "dial", "input": {"code": 4}}]}}),
    );
    for (ordinal, name) in [
        (8, "two-frames-read"),
        (9, "one-frame-read"),
        (10, "bad-tail-read"),
        (11, "refused-read"),
    ] {
        entries.push(from_the_apps(sample(name), ordinal, name));
    }
    let notice = "<system-reminder>\nInbox: 1 notice (1 wake).\n</system-reminder>";
    let wrapped = |words: &str, tail: &str| json!({"type": "text", "text": format!("{CLAUDE_CODE_MID_TURN_HEAD}{words}{tail}")});
    let mut woken = sample("two-frames-read");
    woken["message"]["content"] = json!([
        {"type": "tool_result", "tool_use_id": "call_dial",
            "content": [{"type": "text", "text": "The dial is unlocked."}]},
        wrapped(notice, CLAUDE_CODE_MID_TURN_TAIL_FOLLOWED),
        wrapped("Leave the brass latch open.", CLAUDE_CODE_MID_TURN_TAIL),
    ]);
    woken["host_frames"] = json!([frame(1, true), frame(2, false)]);
    entries.push(from_the_apps(woken, 12, "woken-and-sent"));
    let mut older = from_the_apps(recorded(1), 13, "steer-1-without-the-fact");
    older.as_object_mut().unwrap().remove("host_frame");
    entries.push(older);

    let mut served = Served::new("phone-reader-host-frames-v2", chat(entries));
    let page = |view: Option<&str>| {
        let mut params = json!({"session": "ses_head", "harness": "claude-code", "limit": 50});
        if let Some(view) = view {
            params["view"] = json!(view);
        }
        json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": params})
    };

    let request = page(None);
    let reply = served.read(1, &request).await;
    let rows = decoded(&reply)["result"]["messages"].clone();
    served.keep("01-newest-page", &request, reply);
    let lists: Vec<(Value, Value)> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["messageId"].clone(), row["hostFrames"].clone()))
        .collect();
    let mut expected: Vec<(Value, Value)> = (1..=6)
        .map(|n| (json!(format!("steer-{n}")), listed(&["0"])))
        .collect();
    expected.extend([
        (json!("reply-dial"), Value::Null),
        (json!("two-frames-read"), listed(&["1", "2"])),
        (json!("one-frame-read"), listed(&["1"])),
        (json!("bad-tail-read"), listed(&["1"])),
        (json!("refused-read"), Value::Null),
        (json!("woken-and-sent"), listed(&["1", "2"])),
        (json!("steer-1-without-the-fact"), Value::Null),
    ]);
    assert_eq!(lists, expected);

    // The Simple view drops the wake notice's words, and its entry with them,
    // so every entry left names a part the row still holds.
    let request = page(Some("simple"));
    let reply = served.read(2, &request).await;
    let rows = decoded(&reply)["result"]["messages"].clone();
    served.keep("02-simple-page", &request, reply);
    let woken = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["messageId"] == "woken-and-sent")
        .unwrap();
    assert_eq!(woken["hostFrames"], listed(&["2"]));
    for row in rows.as_array().unwrap() {
        let held: Vec<(String, bool)> = keys(row);
        for entry in row["hostFrames"].as_array().into_iter().flatten() {
            assert!(
                held.iter().any(|(key, _)| entry["partKey"] == key.as_str()),
                "{row}"
            );
        }
    }
    served.check();
}

/// A file of Agent Runner's recordings in `tests/data/thalamus-host-frames-v1/`,
/// copied byte for byte from real Claude Code runs: two messages Claude Code
/// 2.1.332 stored beside one tool result, and one Claude Code 2.1.331 stored
/// as its own `system` message.
fn recording(case: &str, file: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-host-frames-v1")
        .join(case)
        .join(file);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// `phone-reader-host-frames-recorded-v1`: the two recorded read entries,
/// served as Agent Runner serves them, on one newest page. Each row lists
/// the words parts the runner's own display row holds, with the same keys,
/// words and order, and the row's text is the runner's.
#[tokio::test]
async fn the_recorded_host_frames_are_what_the_reader_serves() {
    let cases = ["two-frames-2.1.332", "one-frame-2.1.331"];
    let entries = cases
        .iter()
        .map(|case| recording(case, "session-read-message.json"))
        .collect();
    let mut served = Served::new("phone-reader-host-frames-recorded-v1", chat(entries));
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": "ses_head", "harness": "claude-code", "limit": 50}});
    let reply = served.read(1, &request).await;
    let rows = decoded(&reply)["result"]["messages"].clone();
    served.keep("01-newest-page", &request, reply);
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), cases.len());
    for (row, case) in rows.iter().zip(cases) {
        let runner = recording(case, "display-entry.json")["row"].clone();
        // The runner's words parts: its text parts no host wrote.
        let words: Vec<(Value, Value)> = runner["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "text" && part["host"] != true)
            .map(|part| (part["part"].clone(), part["text"].clone()))
            .collect();
        let listed: Vec<Value> = words
            .iter()
            .map(|(key, _)| json!({"partKey": key, "code": CLAUDE_CODE_MID_TURN}))
            .collect();
        assert_eq!(row["hostFrames"], json!(listed), "{case}");
        for (key, text) in &words {
            let part = row["parts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|part| &part["partKey"] == key)
                .unwrap();
            assert_eq!(
                (&part["type"], &part["text"]),
                (&json!("text"), text),
                "{case}"
            );
        }
        assert_eq!(
            (&row["role"], &row["text"]),
            (&json!("user"), &runner["text"]),
            "{case}"
        );
        for (key, host) in keys(row) {
            assert_eq!(key.contains("#host_frame:"), host, "{case} {key}");
        }
    }
    served.check();
}
