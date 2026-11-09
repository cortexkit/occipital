//! Thalamus's runner serving real Claude Code 2.1.318 sessions, recorded
//! against a stand-in model in one test run and copied unchanged into
//! `tests/data/runner-pages-claude-code-v1/` (its `SHA256SUMS` hashes to
//! `ce3d96cd0658f084…`). Each of the 13 folders is one session: its reads with
//! display facts and its raw history in all three media modes, its display
//! history, its head and its live frames. The set's README says what each
//! case records and what this runner build can't produce yet; most notably,
//! a Claude Code call's `tool_started` reaches readers only once the call has
//! returned, so a watch opened mid-call sees no running call.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/runner-pages-claude-code-v1")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap())
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn cases() -> Vec<String> {
    let mut cases: Vec<String> = std::fs::read_dir(root())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    cases.sort();
    cases
}

/// A recorded read's rows as the reader builds them for a Claude Code session
/// on Thalamus's runner: a page's rows, or with `full` an exact read's.
fn rows_of(name: &str, full: bool) -> Vec<Value> {
    let mut binding = resolved();
    binding.reference.runner = THALAMUS_RUNNER_MODULE_ID.into();
    binding.harness = "claude-code".into();
    let read = sample(name);
    BrocaSource::new(binding, FakeRoute::new())
        .rows(read["messages"].as_array().unwrap(), full, &[])
        .unwrap_or_else(|error| panic!("{name}: {error:?}"))
        .into_iter()
        .map(|row| row.display)
        .collect()
}

fn rows(name: &str) -> Vec<Value> {
    rows_of(name, false)
}

/// Every tool part on a list of rows.
fn calls(rows: &[Value]) -> Vec<Value> {
    rows.iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["type"] == "tool")
        .collect()
}

/// The call whose call id is `id`.
fn call(rows: &[Value], id: &str) -> Value {
    calls(rows)
        .into_iter()
        .find(|part| part["callId"] == id)
        .unwrap_or_else(|| panic!("no call {id}"))
}

#[test]
fn the_claude_code_pages_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 168);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Every read and raw history page in every case, in every media mode, builds
/// one row per message with nothing unreadable. Every display history decodes,
/// every head passes the runner contract's consistency check (`head_meta`),
/// and every live frame maps and keeps the feed.
#[test]
fn every_claude_code_reply_decodes() {
    let mut reads = 0;
    for case in cases() {
        for file in std::fs::read_dir(root().join(&case)).unwrap() {
            let file = file.unwrap().file_name().to_string_lossy().into_owned();
            let name = format!("{case}/{file}");
            let body = sample(&name);
            if body.get("error").is_some() {
                continue;
            }
            if file.starts_with("session-history-display") {
                let page: DisplayPage =
                    serde_json::from_value(body).unwrap_or_else(|error| panic!("{name}: {error}"));
                assert!(!page.messages.is_empty(), "{name}");
            } else if file.starts_with("session-read") || file.starts_with("session-history") {
                let built = rows_of(&name, file.contains("exact"));
                assert_eq!(
                    built.len(),
                    body["messages"].as_array().unwrap().len(),
                    "{name}"
                );
                for row in &built {
                    assert!(row["unreadable"].is_null(), "{name}: {row}");
                    for part in row["parts"].as_array().into_iter().flatten() {
                        assert_ne!(part["type"], "unreadable", "{name}: {row}");
                    }
                }
                reads += 1;
            } else if file.starts_with("session-head") || file.ends_with("-session-head.json") {
                head_meta(&body).unwrap_or_else(|error| panic!("{name}: {error:?}"));
            } else if file.starts_with("subscribe-frames") {
                let source = BrocaSource::new(resolved(), FakeRoute::new());
                for subscription in body["subscriptions"].as_array().unwrap() {
                    for (index, frame) in subscription["frames"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .enumerate()
                    {
                        let frame = decode_subscription_frame(&serde_json::to_vec(frame).unwrap())
                            .unwrap_or_else(|error| panic!("{name}:{index}: {error:?}"));
                        if let Some(normalized) = normalize(frame.clone()) {
                            let _ = display_events::map(&normalized);
                        }
                        assert!(source.observe(frame), "{name}:{index} ended the feed");
                    }
                }
            }
        }
    }
    assert_eq!(reads, 85, "page and history reads checked");
}

/// The describe passes the contract's check, and page reads ask it for media
/// references.
#[test]
fn the_claude_code_describe_is_read() {
    let describe = check_describe(&sample("07-describe-and-head/role-describe.json")).unwrap();
    for group in [
        "read_display_facts",
        "media_refs",
        "notices",
        "message_marks",
    ] {
        assert!(describe.declares(group), "{group}");
    }
    assert!(stored_media(Some(&describe)).is_some());
}

/// Every call names its key and both times, and every answer its finish time.
/// A call that ran `echo`, the one tool the session's plan gives it (served by a
/// stand-in tool provider), carries `echo`'s schema pin: the tool version that
/// ran, which is what lets the apps draw the call with that provider's tool
/// view. A call to a tool the session doesn't offer, or one cut at the step
/// limit before it ran, has no pin and draws as a plain chip.
#[test]
fn calls_and_answers_carry_their_identity_and_times() {
    for case in cases() {
        let built = rows(&format!("{case}/session-read-media-refs.json"));
        for row in built.iter().filter(|row| row["role"] == "assistant") {
            assert!(row["completedAtMs"].is_u64(), "{case}: {row}");
        }
        for part in calls(&built) {
            assert!(part["callKey"].is_string(), "{case}: {part}");
            assert!(part["startedAtMs"].is_u64(), "{case}: {part}");
            assert!(part["finishedAtMs"].is_u64(), "{case}: {part}");
            let ran_echo = part["tool"] == "echo" && part["unanswered"] != "not_executed";
            assert_eq!(part["schemaPin"].is_string(), ran_echo, "{case}: {part}");
        }
    }
}

/// How each call ended, as the row shows it once stored: answered; failed
/// (a tool the session doesn't offer); cut at the step limit; stopped by a
/// caller, which reads as cancelled, the same reason the live frame gave.
#[test]
fn each_call_ending_reads_as_it_happened() {
    let built = rows("02-tool-outcomes/session-read-media-refs.json");
    let shown = |id: &str| {
        let part = call(&built, id);
        (part["isError"].clone(), part["unanswered"].clone())
    };
    assert_eq!(shown("toolu_tools_echo"), (Value::Null, Value::Null));
    assert_eq!(shown("toolu_tools_missing"), (json!(true), Value::Null));
    assert_eq!(
        shown("toolu_tools_step_two"),
        (json!(true), json!("not_executed"))
    );
    assert_eq!(shown("toolu_tools_held"), (json!(true), json!("cancelled")));
    let stopped = call(
        &rows("13-stop-all/session-read-media-refs.json"),
        "toolu_stop_call",
    );
    assert_eq!(stopped["unanswered"], "cancelled", "{stopped}");
}

/// A tool's answer comes back in a user message, as the Messages API carries
/// it, but nobody typed it: every message holding only tool results reads as a
/// tool row, never as the person's.
#[test]
fn tool_answers_are_never_the_persons_words() {
    let read = sample("02-tool-outcomes/session-read-media-refs.json");
    let built = rows("02-tool-outcomes/session-read-media-refs.json");
    let mut answers = 0;
    for (message, row) in read["messages"].as_array().unwrap().iter().zip(&built) {
        let blocks = message["message"]["content"].as_array();
        let only_results =
            blocks.is_some_and(|blocks| blocks.iter().all(|block| block["type"] == "tool_result"));
        if message["role"] == "user" && only_results {
            assert_eq!(row["role"], "tool", "{row}");
            answers += 1;
        }
    }
    assert_eq!(answers, 3);
}

/// Claude's thinking is a reasoning part, kept even when Claude Code stored
/// no text for it, so the apps keep each part's position.
#[test]
fn thinking_stays_a_reasoning_part_even_when_empty() {
    let built = rows("01-text-and-reasoning/session-read-media-refs.json");
    let reasoning: Vec<Value> = built
        .iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["type"] == "reasoning")
        .map(|part| part["text"].clone())
        .collect();
    assert_eq!(
        reasoning,
        [json!("Weighing the question before answering."), json!("")]
    );
}

/// The shared photo and PDF are file parts with their type and digest, and
/// the photo's digest is the one `session.media` serves its bytes under.
#[test]
fn shared_files_are_file_parts_the_media_read_serves() {
    let built = rows("03-photo-and-file/session-read-media-refs.json");
    let files: Vec<Value> = built
        .iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["type"] == "file")
        .collect();
    assert_eq!(
        files
            .iter()
            .map(|file| file["mime"].clone())
            .collect::<Vec<_>>(),
        [json!("image/png"), json!("application/pdf")]
    );
    let photo = sample("03-photo-and-file/session-media-read-photo.json");
    let bytes = crate::base64::decode(photo["data"].as_str().unwrap()).unwrap();
    assert_eq!(files[0]["sha256"], format!("{:x}", Sha256::digest(&bytes)));
}

/// Core's notice reads as a notice, words core passed on read as the agent's,
/// and the phone's message carries the app message id it was sent under.
#[test]
fn marks_and_app_message_ids_reach_the_rows() {
    let built = rows("08-notices-and-marks/session-read-media-refs.json");
    let marked = |text: &str| {
        built
            .iter()
            .find(|row| {
                row["text"]
                    .as_str()
                    .is_some_and(|row| row.starts_with(text))
            })
            .unwrap_or_else(|| panic!("no row {text}"))
            .clone()
    };
    assert_eq!(
        marked("A notice from core")["notice"],
        json!({"sender": "prefrontal-core", "deliveredAs": "idle_start"})
    );
    assert_eq!(
        marked("Words agent-a asked")["speaksFor"],
        json!({"agentId": "agent-a"})
    );
    let phone = rows("09-client-and-send-ids/session-read-media-refs.json")
        .into_iter()
        .find(|row| row["text"] == "Sent from the phone.")
        .unwrap();
    assert_eq!(phone["clientMessageId"], "phone-message-1", "{phone}");
}

/// The upgrade to provider message ids raises the history counter once, from
/// 0 to 1, and keeps the log identity, so each app holding the chat reloads it
/// exactly once; a later restart moves nothing.
#[test]
fn the_message_id_upgrade_reloads_the_chat_once() {
    let head = |file: &str| sample(&format!("06-legacy-id-upgrade/{file}"));
    let versions: Vec<(Value, Value)> = [
        "session-head-before-upgrade.json",
        "session-head-after-upgrade.json",
        "session-head-after-second-restart.json",
    ]
    .iter()
    .map(|file| {
        let head = head(file);
        (head["history_version"].clone(), head["incarnation"].clone())
    })
    .collect();
    assert_eq!(
        versions
            .iter()
            .map(|(version, _)| version.clone())
            .collect::<Vec<_>>(),
        [json!(0), json!(1), json!(1)]
    );
    assert!(versions.iter().all(|(_, log)| *log == versions[0].1));
}

/// An exact read of the long tool answer serves its whole output text.
#[test]
fn an_exact_read_serves_a_long_answer_whole() {
    let built = rows_of("11-long-payloads/session-read-exact-result.json", true);
    let content: usize = built
        .iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .filter_map(|part| part["content"].as_str().map(str::len))
        .sum();
    assert!(content > 59_000, "{content}");
}

/// The Stop everything reply decodes as the contract's reply.
#[test]
fn the_stop_everything_reply_decodes() {
    let reply: cortexkit_role_llm_runner::stop_all::StopAllReply =
        serde_json::from_value(sample("13-stop-all/session-stop-all.json")).unwrap();
    assert_eq!(reply.providers.len(), 1);
}
