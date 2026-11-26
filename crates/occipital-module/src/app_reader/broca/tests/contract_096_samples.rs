//! What runner contract 0.9.6 adds to the rows the apps read, and the sample
//! sets the apps pin for it, each served through Occipital's own handlers for
//! a Claude Code chat as a phone's route reaches it. `served_samples` says how
//! a set is checked and rewritten.
//!
//! - Messages the person sent while a Claude Code turn ran: Claude Code
//!   stores each inside two fixed sentences of its own as a `system` message,
//!   and the read's `host_frame` fact names that wrapper (llm-runner
//!   §9.7.1.1). The six messages are the contract's own recordings
//!   (`steer-frame-claude-code-v1`), which predate the fact, so these tests add
//!   the fact a runner now serves for each. Their set,
//!   `phone-reader-host-frame-v1`, is frozen: its rows predate `hostFrames`,
//!   and `contract_097_samples` serves its messages in the set that replaces it.
//! - `phone-reader-runner-entry-v1`: the empty reply Agent Runner writes when
//!   Claude Code saved no reply for a finished step, marked with the read's
//!   `runner_entry` fact (llm-runner §4.7), beside a model's own empty reply,
//!   which carries no mark.
//! - `phone-reader-fallback-reason-v1`: replies another model wrote after the
//!   one asked for refused, with the reason `model_fallback` now carries.

use super::claude_code_samples::{AnyHarness, ClaudeCodeHead};
use super::served_samples::{decoded, Served};
use super::*;
use cortexkit_role_llm_runner::attribution::host_frames;
use std::path::Path;

/// The wrapper fact a runner serves for a message Claude Code framed.
fn frame() -> Value {
    json!({"part": 0, "code": host_frames::CLAUDE_CODE_MID_TURN,
        "head_bytes": host_frames::CLAUDE_CODE_MID_TURN_HEAD.len(),
        "tail_bytes": host_frames::CLAUDE_CODE_MID_TURN_TAIL.len()})
}

/// Recorded mid-turn message `n` (1 to 6) from the contract's samples.
fn steer(n: usize) -> Value {
    let name = format!("steer-frame-claude-code-v1/steer-{n}/read-entry.json");
    serde_json::from_str(cortexkit_role_llm_runner::vectors::file(&name).unwrap()).unwrap()
}

/// The person's words inside a framed message's stored text.
fn words(entry: &Value) -> &str {
    entry["message"]["content"]
        .as_str()
        .unwrap()
        .strip_prefix(host_frames::CLAUDE_CODE_MID_TURN_HEAD)
        .unwrap()
        .strip_suffix(host_frames::CLAUDE_CODE_MID_TURN_TAIL)
        .unwrap()
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

/// With the fact, a framed message is the person's: their words are its text,
/// and each of the two sentences is a host part with the frame's code, keyed
/// from the stored part so the keys hold across reads. Every recorded message
/// splits, including the two whose words copy a sentence of the wrapper.
#[test]
fn a_framed_message_is_the_persons_words_between_two_host_parts() {
    for n in 1..=6 {
        let mut entry = steer(n);
        let said = words(&entry).to_owned();
        entry["host_frame"] = frame();
        let row = row(entry);
        assert_eq!(
            (&row["role"], &row["text"]),
            (&json!("user"), &json!(said)),
            "steer {n}"
        );
        assert_eq!(row["clientMessageId"], json!(format!("client-steer-{n}")));
        assert_eq!(
            row["parts"],
            json!([
                {"type": "injected", "text": host_frames::CLAUDE_CODE_MID_TURN_HEAD,
                    "host": true, "hostCode": "claude_code_mid_turn",
                    "partIndex": 0, "partKey": "0#host_frame:head"},
                {"type": "text", "text": said, "partIndex": 0, "partKey": "0"},
                {"type": "injected", "text": host_frames::CLAUDE_CODE_MID_TURN_TAIL,
                    "host": true, "hostCode": "claude_code_mid_turn",
                    "partIndex": 0, "partKey": "0#host_frame:tail"},
            ]),
            "steer {n}"
        );
    }
}

/// Without the fact, or with one the stored text doesn't match exactly, the
/// message is shown as stored, as before the fact existed: the wrapper is
/// never found from the text alone.
#[test]
fn an_unframed_or_mismatched_message_stays_as_stored() {
    let stored = steer(1)["message"]["content"].clone();
    let mut wrong = frame();
    wrong["tail_bytes"] = json!(host_frames::CLAUDE_CODE_MID_TURN_TAIL.len() - 1);
    for fact in [None, Some(wrong)] {
        let mut entry = steer(1);
        if let Some(fact) = fact {
            entry["host_frame"] = fact;
        }
        let row = row(entry);
        assert_eq!((&row["role"], &row["text"]), (&json!("system"), &stored));
    }
}

/// A `runner_entry` fact reaches the app row as `runnerEntry`, whatever its
/// code, and a fact the contract refuses drops only itself.
#[test]
fn a_runner_entry_reaches_the_row() {
    let empty = |fact: Option<Value>| {
        let mut entry = json!({"mid": "thalamus-final-test", "ordinal": 1,
            "message": {"role": "assistant", "content": []}});
        if let Some(fact) = fact {
            entry["runner_entry"] = fact;
        }
        row(entry)["runnerEntry"].clone()
    };
    assert_eq!(
        empty(Some(json!({"code": "host_stored_no_reply"}))),
        json!({"code": "host_stored_no_reply"})
    );
    assert_eq!(
        empty(Some(json!({"code": "a_later_code"}))),
        json!({"code": "a_later_code"})
    );
    assert_eq!(empty(Some(json!({"code": "Not A Code"}))), Value::Null);
    assert_eq!(empty(None), Value::Null);
}

/// A reply's fallback reason reaches the app row beside the two models, and a
/// fallback recorded without one keeps the note's earlier shape.
#[test]
fn a_fallback_reason_reaches_the_row() {
    let reply = |fallback: Value| {
        row(
            json!({"mid": "m-reply", "ordinal": 1, "model_fallback": fallback,
            "message": {"role": "assistant", "content": [{"type": "text", "text": "Done."}]}}),
        )["modelFallback"]
            .clone()
    };
    let reason = json!({"requested": "claude-opus-5-5", "answered": "claude-opus-4-8",
        "by": "client", "category": "cyber", "cause": "refused_here"});
    assert_eq!(reply(reason.clone()), reason);
    let bare = json!({"requested": "claude-opus-5-5", "answered": "claude-opus-4-8"});
    assert_eq!(reply(bare.clone()), bare);
    // From runner contract 0.9.9 a provider that names no refusal category
    // leaves it out; the reason still reaches the row.
    let uncategorized = json!({"requested": "claude-opus-5-5", "answered": "claude-opus-4-8",
        "by": "provider", "cause": "refused_here"});
    assert_eq!(reply(uncategorized.clone()), uncategorized);
}

/// The app's newest page of the chat holding `entries`, kept in `set`.
async fn serve(set: &'static str, entries: Vec<Value>) -> (Served, Value) {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries;
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(ClaudeCodeHead))),
            Arc::new(AnyHarness(route.clone())),
        ),
    )));
    let mut served = Served::new(set, app);
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": "ses_head", "harness": "claude-code", "limit": 50}});
    let reply = served.read(1, &request).await;
    let rows = decoded(&reply)["result"]["messages"].clone();
    served.keep("01-newest-page", &request, reply);
    (served, rows)
}

/// `phone-reader-runner-entry-v1`: Agent Runner's recorded Claude Code chat
/// (`thalamus-host-parity-wave1`), whose last entry is the empty reply the
/// runner wrote, now with the `runner_entry` fact it serves. Two copies of that
/// entry follow, invented to show the other cases: one with a code from a
/// later contract, which still marks a runner-written entry, and one with no
/// fact, as a model's own empty reply reads.
#[tokio::test]
async fn the_runner_entry_samples_are_what_the_reader_serves() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-host-parity-wave1/claude-code-session-read-display-empty.json");
    let recorded: Value = serde_json::from_slice(&std::fs::read(root).unwrap()).unwrap();
    let mut chat = recorded["messages"].as_array().unwrap().clone();
    let written = chat.last_mut().unwrap();
    assert_eq!(written["message"]["cortexkit_runner_entry"], json!(true));
    written["runner_entry"] = json!({"code": "host_stored_no_reply"});
    let mut later = written.clone();
    later["mid"] = json!("thalamus-final-a-later-code");
    later["ordinal"] = json!(4);
    later["runner_entry"] = json!({"code": "a_later_code"});
    let mut unmarked = written.clone();
    unmarked["mid"] = json!("a-models-empty-reply");
    unmarked["ordinal"] = json!(5);
    unmarked.as_object_mut().unwrap().remove("runner_entry");
    chat.extend([later, unmarked]);
    let (served, rows) = serve("phone-reader-runner-entry-v1", chat).await;
    let marks: Vec<Value> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["runnerEntry"].clone())
        .collect();
    assert_eq!(
        marks,
        [
            Value::Null,
            Value::Null,
            json!({"code": "host_stored_no_reply"}),
            json!({"code": "a_later_code"}),
            Value::Null,
        ]
    );
    served.check();
}

/// `phone-reader-fallback-reason-v1`: three replies another model wrote in
/// place of the one asked for, invented in the shape a runner serves them:
/// the sign-in plugin retried after a refusal, the provider's own fallback
/// answered later in its round, and a fallback recorded with no reason.
#[tokio::test]
async fn the_fallback_reason_samples_are_what_the_reader_serves() {
    fallback_samples("phone-reader-fallback-reason-v1", &fallbacks()).await;
}

/// `phone-reader-fallback-reason-v2`: the v1 replies, and a fourth whose
/// provider named no refusal category, which runner contract 0.9.9 leaves out.
#[tokio::test]
async fn the_fallback_reason_v2_samples_are_what_the_reader_serves() {
    let mut fallbacks = fallbacks().to_vec();
    fallbacks.push(
        json!({"requested": "claude-opus-5-5", "answered": "claude-opus-4-8",
        "by": "provider", "cause": "refused_here"}),
    );
    fallback_samples("phone-reader-fallback-reason-v2", &fallbacks).await;
}

/// The fallback facts of the v1 samples, one per reply.
fn fallbacks() -> [Value; 3] {
    [
        json!({"requested": "claude-opus-5-5", "answered": "claude-opus-4-8",
            "by": "client", "category": "cyber", "cause": "refused_here"}),
        json!({"requested": "claude-opus-5-5", "answered": "claude-opus-4-8",
            "by": "provider", "category": "cyber", "cause": "round"}),
        json!({"requested": "claude-opus-5-5", "answered": "claude-opus-4-8"}),
    ]
}

/// Serve one reply per fact in `fallbacks` as `set`, and check each row's note
/// is that fact.
async fn fallback_samples(set: &'static str, fallbacks: &[Value]) {
    let reply = |ordinal: u64, fallback: Value| {
        json!({"mid": format!("m-reply-{ordinal}"), "ordinal": ordinal,
            "written_at": 1_795_515_400_000_u64 + ordinal, "model_fallback": fallback,
            "message": {"role": "assistant",
                "content": [{"type": "text", "text": format!("Reply {ordinal}.")}]}})
    };
    let chat = (1..)
        .zip(fallbacks.iter().cloned())
        .map(|(ordinal, fallback)| reply(ordinal, fallback))
        .collect();
    let (served, rows) = serve(set, chat).await;
    let notes: Vec<Value> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["modelFallback"].clone())
        .collect();
    assert_eq!(notes, fallbacks);
    served.check();
}
