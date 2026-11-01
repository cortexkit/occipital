//! Tool frames on the display lane: what a reader that accepts them sees, and
//! that a reader that doesn't sees exactly what it saw before they existed.

use super::test_support::FakeSession;
use serde_json::{json, Value};

const NOW: u64 = 1_793_500_100_000;
const PIN: &str = "tp1:bash:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:1";

fn delta(mid: &str, text: &str) -> Value {
    json!({"kind":"display", "event":{"type":"text_delta", "mid":mid, "delta":text}})
}

fn control(event: Value) -> Value {
    json!({"kind":"control", "event":event})
}

fn started(call: &str, arguments: &str) -> Value {
    started_tool(call, "bash", PIN, arguments)
}

fn started_tool(call: &str, tool: &str, pin: &str, arguments: &str) -> Value {
    control(
        json!({"type":"tool_started", "mid":"m1", "tool_call_id":call, "name":tool,
        "arguments":arguments, "started_at":1_793_500_001_000_u64, "call_key":format!("ck_{call}"),
        "schema_pin":pin}),
    )
}

fn finished(call: &str, outcome: &str) -> Value {
    control(
        json!({"type":"tool_finished", "mid":"m1", "tool_call_id":call, "outcome":outcome,
        "output_bytes":12, "finished_at":1_793_500_002_000_u64}),
    )
}

/// Feed the runner's events in order, then send any words held for batching.
fn run(session: &FakeSession, events: &[Value]) {
    for event in events {
        session.ingest(event.clone());
    }
    session.flush();
}

fn with_tools(session: &FakeSession, since: Option<u64>) -> Value {
    serde_json::to_value(session.projection.display_read_with_tools(since, NOW)).unwrap()
}

fn plain(session: &FakeSession, since: Option<u64>) -> Value {
    serde_json::to_value(session.projection.display_read(since)).unwrap()
}

/// The kinds and phases of an incremental reply's frames, in order.
fn kinds(reply: &Value) -> Vec<String> {
    reply["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sequenced| {
            let frame = &sequenced["frame"];
            match frame["phase"].as_str() {
                Some(phase) => format!("tool {} {phase}", frame["callId"].as_str().unwrap()),
                None => frame["kind"].as_str().unwrap().to_owned(),
            }
        })
        .collect()
}

/// A reader that didn't opt in gets byte for byte what it got when tool events
/// were ignored: the same frames and the same sequence numbers.
#[test]
fn a_reader_without_tool_frames_sees_no_change() {
    let text_only = FakeSession::new("ses", 64);
    let with_calls = FakeSession::new("ses", 64);
    let words = [delta("m1", "Listing."), delta("m2", "Done.")];
    run(&text_only, &[words[0].clone()]);
    run(
        &with_calls,
        &[
            words[0].clone(),
            started("c1", r#"{"command":"ls"}"#),
            finished("c1", "ok"),
        ],
    );
    run(&text_only, &[words[1].clone()]);
    run(&with_calls, &[words[1].clone()]);
    for since in [None, Some(0), Some(1)] {
        assert_eq!(
            plain(&text_only, since),
            plain(&with_calls, since),
            "since {since:?}"
        );
    }
}

/// Text and calls keep the runner's order, with words held for batching sent
/// before the call that follows them. Each frame carries the stored part's
/// members, and `asOfMs` is the read's time.
#[test]
fn a_reader_with_tool_frames_sees_each_call_start_and_end_in_order() {
    let session = FakeSession::new("ses", 64);
    run(
        &session,
        &[
            delta("m1", "Listing."),
            started("c1", r#"{"command":"ls"}"#),
            finished("c1", "ok"),
            started("c2", r#"{"command":"false"}"#),
            finished("c2", "error"),
            delta("m2", "Done."),
        ],
    );
    let reply = with_tools(&session, Some(0));
    assert_eq!(
        kinds(&reply),
        [
            "display",
            "tool c1 running",
            "tool c1 answered",
            "tool c2 running",
            "tool c2 answered",
            "display"
        ],
        "{reply}"
    );
    let running = &reply["frames"][1]["frame"];
    assert_eq!(
        running,
        &json!({"kind":"tool", "sessionID":"ses", "messageID":"m1", "callId":"c1",
            "tool":"bash", "phase":"running", "callKey":"ck_c1", "schemaPin":PIN,
            "input":{"command":"ls"}, "inputTruncatedFields":[],
            "startedAtMs":1_793_500_001_000_u64, "asOfMs":NOW})
    );
    let answered = &reply["frames"][2]["frame"];
    assert_eq!(answered["finishedAtMs"], 1_793_500_002_000_u64);
    assert_eq!(answered["outputBytes"], 12);
    assert!(answered.get("isError").is_none());
    assert_eq!(reply["frames"][4]["frame"]["isError"], true);
}

/// Any outcome but `ok` and `error` is why the call went unanswered.
#[test]
fn a_cancelled_or_denied_call_is_unanswered_with_its_reason() {
    let session = FakeSession::new("ses", 64);
    run(
        &session,
        &[
            started("c1", "{}"),
            finished("c1", "denied"),
            started("c2", "{}"),
            finished("c2", "skipped_by_policy"),
        ],
    );
    let reply = with_tools(&session, Some(0));
    assert_eq!(reply["frames"][1]["frame"]["phase"], "unanswered");
    assert_eq!(reply["frames"][1]["frame"]["unanswered"], "denied");
    assert_eq!(
        reply["frames"][3]["frame"]["unanswered"],
        "skipped_by_policy"
    );
}

/// A reader that starts mid-call gets each running call in its baseline, with its
/// original start, even when no text has streamed yet.
#[test]
fn a_baseline_carries_the_calls_still_running() {
    let session = FakeSession::new("ses", 64);
    run(&session, &[started("c1", "{}")]);
    let baseline = with_tools(&session, None);
    assert_eq!(
        baseline["frame"],
        json!({"kind":"display_gap"}),
        "{baseline}"
    );
    assert_eq!(baseline["tools"][0]["callId"], "c1");
    assert_eq!(baseline["tools"][0]["startedAtMs"], 1_793_500_001_000_u64);
    assert_eq!(baseline["tools"][0]["asOfMs"], NOW);
    // A reader without tool frames still sees a quiet ring.
    assert_eq!(plain(&session, None), json!({"absent":"ring_quiet"}));

    run(
        &session,
        &[
            delta("m1", "Waiting."),
            started("c2", "{}"),
            finished("c1", "ok"),
        ],
    );
    let baseline = with_tools(&session, None);
    assert_eq!(baseline["frame"]["chunk"], "Waiting.");
    let running: Vec<&Value> = baseline["tools"].as_array().unwrap().iter().collect();
    assert_eq!(running.len(), 1, "{baseline}");
    assert_eq!(running[0]["callId"], "c2");
}

/// A display gap loses text, never calls: each running call is sent again after
/// it. A reset forgets them, since the stored page then shows each call's phase.
#[test]
fn a_gap_resends_running_calls_and_a_reset_forgets_them() {
    let session = FakeSession::new("ses", 64);
    run(&session, &[started("c1", "{}")]);
    let before = with_tools(&session, None)["seq"].as_u64().unwrap();
    run(
        &session,
        &[json!({"kind":"display", "event":{"type":"display_gap"}})],
    );
    let after_gap = with_tools(&session, Some(before));
    assert_eq!(
        kinds(&after_gap),
        ["display_gap", "tool c1 running"],
        "{after_gap}"
    );

    let seq = after_gap["seq"].as_u64().unwrap();
    run(
        &session,
        &[json!({"kind":"display", "event":{"type":"display_reset"}})],
    );
    assert_eq!(kinds(&with_tools(&session, Some(seq))), ["display_gap"]);
    assert!(with_tools(&session, None).get("tools").is_none());
    // An end for a call no longer known sends nothing.
    let seq = with_tools(&session, None)["seq"].as_u64().unwrap();
    run(&session, &[finished("c1", "ok")]);
    assert_eq!(
        kinds(&with_tools(&session, Some(seq))),
        Vec::<String>::new()
    );
}

/// A run that ends with a call still running ends it, so no reader shows it
/// running for good.
#[test]
fn a_run_end_ends_each_call_still_running() {
    let session = FakeSession::new("ses", 64);
    run(
        &session,
        &[
            control(json!({"type":"run_started", "run_id":"r1"})),
            delta("m1", "Working."),
            started("c1", "{}"),
            control(json!({"type":"run_finished", "run_id":"r1", "reason":"cancelled"})),
        ],
    );
    let reply = with_tools(&session, Some(0));
    assert_eq!(
        kinds(&reply),
        [
            "display",
            "tool c1 running",
            "tool c1 unanswered",
            "display_end"
        ],
        "{reply}"
    );
    let ended = &reply["frames"][2]["frame"];
    assert_eq!(ended["unanswered"], "interrupted");
    assert!(ended.get("finishedAtMs").is_none() && ended.get("outputBytes").is_none());
    assert!(with_tools(&session, None).get("tools").is_none());
}

/// A call without a tool version has no row input, as its stored part has none;
/// arguments the runner cut can't be parsed, so they give none either.
#[test]
fn input_follows_the_stored_parts_rules() {
    let session = FakeSession::new("ses", 64);
    run(
        &session,
        &[
            control(
                json!({"type":"tool_started", "mid":"m1", "tool_call_id":"plain",
                "name":"bash", "arguments":r#"{"command":"ls"}"#}),
            ),
            control(
                json!({"type":"tool_started", "mid":"m1", "tool_call_id":"cut",
                "name":"bash", "arguments":r#"{"command":"l"#, "arguments_truncated":true,
                "schema_pin":PIN}),
            ),
            started("long", &json!({"command": "x".repeat(4_000)}).to_string()),
        ],
    );
    let reply = with_tools(&session, Some(0));
    let frame = |n: usize| reply["frames"][n]["frame"].clone();
    assert!(frame(0).get("input").is_none() && frame(0).get("schemaPin").is_none());
    assert!(frame(1).get("input").is_none() && frame(1)["schemaPin"] == PIN);
    assert_eq!(frame(2)["inputTruncatedFields"], json!(["/command"]));
    assert!(frame(2)["input"].to_string().len() <= crate::tool_input::INPUT_BYTES);
}

/// The sample set the apps pin, `phone-reader-tool-frames-v1`; its README says
/// what each file shows. The replies come from the display read at a fixed time,
/// the same call a served `session.display_read` and each display watch notice
/// make. This test serves every request again and fails if a byte changes; with
/// `OCCIPITAL_WRITE_SAMPLES` set to a folder it writes the set there instead.
#[test]
fn the_tool_frame_samples_are_what_the_reader_serves() {
    let session = FakeSession::new("ses_head", 64);
    let read = |since: Option<u64>, tools: bool| session.read(since, tools, NOW);
    let mut files: Vec<(String, Value)> = Vec::new();
    let mut keep = |name: &str, (request, reply): (Value, Value)| {
        files.push((format!("{name}.request.json"), request));
        files.push((format!("{name}.json"), reply));
    };

    run(
        &session,
        &[
            control(json!({"type":"run_started", "run_id":"r1"})),
            delta("m1", "I'll list the folder first."),
            started("c1", r#"{"command":"ls -la"}"#),
        ],
    );
    let baseline = read(None, true);
    let seq = baseline.1["result"]["seq"].as_u64().unwrap();
    keep("01-baseline-mid-call", baseline);
    run(
        &session,
        &[
            finished("c1", "ok"),
            started_tool(
                "c2",
                "read",
                &PIN.replace("bash", "read"),
                &json!({"path": format!("notes/{}.md", "very-long-name-".repeat(200))}).to_string(),
            ),
            finished("c2", "error"),
            control(
                json!({"type":"tool_started", "mid":"m1", "tool_call_id":"c3",
                "name":"bash", "arguments":r#"{"command":"sleep 100"}"#,
                "started_at":1_793_500_003_000_u64}),
            ),
            json!({"kind":"display", "event":{"type":"display_gap"}}),
        ],
    );
    let incremental = read(Some(seq), true);
    let seq = incremental.1["result"]["seq"].as_u64().unwrap();
    keep("02-calls-answered-and-a-gap", incremental);
    run(
        &session,
        &[control(
            json!({"type":"run_finished", "run_id":"r1", "reason":"cancelled"}),
        )],
    );
    keep("03-run-end-ends-running-call", read(Some(seq), true));
    keep("04-same-run-without-tool-frames", read(Some(0), false));
    super::keep_sample_set("phone-reader-tool-frames-v1", &files);
}
