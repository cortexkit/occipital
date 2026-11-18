//! Message frames on the display lane: a call of the chat's message tool shows
//! its text as the model writes it, to readers that accept message frames, and
//! changes nothing for readers that don't.

use super::test_support::FakeSession;
use serde_json::{json, Value};

const NOW: u64 = 1_793_500_100_000;
const PIN: &str = "tp1:say:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1";
const STAMP: &str = "4e256f56bf6a6f69d85f0bd9a101fd53637784fd76f7e8c53447069fd1dd77f5";

fn control(event: Value) -> Value {
    json!({"kind":"control", "event":event})
}

/// A chunk of `call`'s arguments; `tool` names it on the chunk that begins them.
fn chunk(call: &str, tool: Option<&str>, text: &str) -> Value {
    let mut event = json!({"type":"tool_input_delta", "tool_call_id":call, "delta":text,
        "mid":"m1", "part":"1"});
    if let Some(tool) = tool {
        event["tool_name"] = json!(tool);
    }
    json!({"kind":"display", "event":event})
}

fn started(call: &str, tool: &str) -> Value {
    control(
        json!({"type":"tool_started", "mid":"m1", "tool_call_id":call, "name":tool,
        "arguments":"{}", "started_at":1_793_500_001_000_u64, "call_key":format!("ck_{call}"),
        "schema_pin":PIN}),
    )
}

fn finished(call: &str, outcome: &str) -> Value {
    control(
        json!({"type":"tool_finished", "mid":"m1", "tool_call_id":call, "outcome":outcome,
        "output_bytes":4, "finished_at":1_793_500_002_000_u64}),
    )
}

fn run_started() -> Value {
    control(json!({"type":"run_started", "run_id":"r1",
        "rich_ui":{"dialect":"1.0", "catalog_digest":STAMP}}))
}

/// A chat whose agent talks through `say`.
fn say_chat() -> FakeSession {
    let mut session = FakeSession::new("ses", 64);
    session.message_tool = Some("say");
    session
}

fn run(session: &FakeSession, events: &[Value]) {
    for event in events {
        session.ingest(event.clone());
    }
    session.flush();
}

fn with_messages(session: &FakeSession, since: Option<u64>, tools: bool) -> Value {
    serde_json::to_value(
        session
            .projection
            .display_read_with_messages(since, NOW, tools),
    )
    .unwrap()
}

/// Each frame of an incremental reply, as `kind` plus what tells it apart.
fn frames(reply: &Value) -> Vec<String> {
    reply["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sequenced| {
            let frame = &sequenced["frame"];
            match (frame["kind"].as_str().unwrap(), frame["partType"].as_str()) {
                ("display", Some("message")) => format!(
                    "message {} {}",
                    frame["callId"].as_str().unwrap(),
                    frame["chunk"].as_str().unwrap()
                ),
                ("message", _) if frame["frozen"] == true => {
                    format!("frozen {}", frame["callId"].as_str().unwrap())
                }
                ("message", _) => format!("withdrawn {}", frame["callId"].as_str().unwrap()),
                ("tool", _) => format!(
                    "tool {} {}",
                    frame["callId"].as_str().unwrap(),
                    frame["phase"].as_str().unwrap()
                ),
                (kind, _) => kind.to_owned(),
            }
        })
        .collect()
}

/// The text of a `say` streams as decoded from its arguments, stamped with the
/// run's rich UI stamp; the call itself is no tool call to these readers.
#[test]
fn a_message_streams_its_text_as_the_model_writes_it() {
    let session = say_chat();
    run(
        &session,
        &[
            run_started(),
            chunk("c1", Some("say"), r#"{"reply_to":"m0","te"#),
            chunk("c1", None, r#"xt":"Done: <Badge>ok</Badge> \u00"#),
            chunk("c1", None, r#"e9","level":"chat"}"#),
            started("c1", "say"),
            finished("c1", "ok"),
            chunk("c2", Some("bash"), r#"{"command":"ls"}"#),
            started("c2", "bash"),
        ],
    );
    let reply = with_messages(&session, Some(0), true);
    assert_eq!(
        frames(&reply),
        [
            "message c1 Done: <Badge>ok</Badge> ",
            "message c1 é",
            "tool c2 running",
        ],
        "{reply}"
    );
    let first = &reply["frames"][0]["frame"];
    assert_eq!(
        *first,
        json!({"kind":"display", "sessionID":"ses", "messageID":"m1", "partType":"message",
            "partKey":"1", "callId":"c1", "chunk":"Done: <Badge>ok</Badge> ",
            "richUi":{"dialect":"1.0", "catalogDigest":STAMP}})
    );
    // An answered message stays until the stored one replaces it, so its
    // call's end sends nothing; nor does a reader without tool frames see c2.
    assert_eq!(
        frames(&with_messages(&session, Some(0), false)),
        ["message c1 Done: <Badge>ok</Badge> ", "message c1 é"]
    );
}

/// A reader that didn't ask for message frames sees what it saw before they
/// existed: the `say` as a tool call to a reader of tool frames, nothing more.
#[test]
fn readers_without_message_frames_see_the_call_as_before() {
    let events = [
        run_started(),
        chunk("c1", Some("say"), r#"{"text":"Hi"#),
        chunk("c1", None, r#" there"}"#),
        started("c1", "say"),
        finished("c1", "ok"),
    ];
    let with = say_chat();
    run(&with, &events);
    let without = FakeSession::new("ses", 64);
    run(&without, &events);
    for since in [None, Some(0)] {
        assert_eq!(
            serde_json::to_value(with.projection.display_read_with_tools(since, NOW)).unwrap(),
            serde_json::to_value(without.projection.display_read_with_tools(since, NOW)).unwrap()
        );
        assert_eq!(
            serde_json::to_value(with.projection.display_read(since)).unwrap(),
            serde_json::to_value(without.projection.display_read(since)).unwrap()
        );
    }
    assert_eq!(
        frames(
            &serde_json::to_value(with.projection.display_read_with_tools(Some(0), NOW)).unwrap()
        ),
        ["tool c1 running", "tool c1 answered"]
    );
}

/// On a chat whose agent has no message tool, a call named `say` is a tool
/// call to every reader, and its arguments show nothing.
#[test]
fn without_the_message_tool_a_say_is_a_tool_call() {
    let session = FakeSession::new("ses", 64);
    run(
        &session,
        &[
            run_started(),
            chunk("c1", Some("say"), r#"{"text":"Hi"}"#),
            started("c1", "say"),
        ],
    );
    assert_eq!(
        frames(&with_messages(&session, Some(0), true)),
        ["tool c1 running"]
    );
}

/// A baseline carries each message still being written, with its text so far.
#[test]
fn a_baseline_carries_the_messages_being_written() {
    let session = say_chat();
    run(
        &session,
        &[
            run_started(),
            chunk("c1", Some("say"), r#"{"text":"Checking the"#),
        ],
    );
    let baseline = with_messages(&session, None, true);
    assert_eq!(
        baseline["messages"],
        json!([{"messageID":"m1", "partKey":"1", "callId":"c1", "text":"Checking the",
            "richUi":{"dialect":"1.0", "catalogDigest":STAMP}}]),
        "{baseline}"
    );
    // A reader of tool frames alone has no `messages`.
    assert!(
        serde_json::to_value(session.projection.display_read_with_tools(None, NOW)).unwrap()
            ["messages"]
            .is_null()
    );
}

/// A gap freezes a message being written, since chunks were lost: it keeps
/// showing and grows no more. A reset withdraws it, and its call streams again
/// only from a chunk that begins its arguments.
#[test]
fn a_gap_freezes_a_message_and_a_reset_withdraws_it() {
    let session = say_chat();
    run(
        &session,
        &[
            run_started(),
            chunk("c1", Some("say"), r#"{"text":"Checking the"#),
            json!({"kind":"display", "event":{"type":"display_gap", "dropped":3}}),
            chunk("c1", None, r#" logs"#),
        ],
    );
    let baseline = with_messages(&session, None, true);
    assert_eq!(baseline["messages"][0]["frozen"], true, "{baseline}");
    assert_eq!(baseline["messages"][0]["text"], "Checking the");
    run(
        &session,
        &[
            json!({"kind":"display", "event":{"type":"display_reset"}}),
            chunk("c1", None, r#" ignored"#),
            chunk("c1", Some("say"), r#"{"text":"Fresh"#),
        ],
    );
    assert_eq!(
        frames(&with_messages(&session, Some(0), true)),
        [
            "message c1 Checking the",
            "display_gap",
            "frozen c1",
            "display_gap",
            "withdrawn c1",
            "message c1 Fresh",
        ]
    );
}

/// A message whose call failed or went unanswered sent nothing: its live text
/// is withdrawn. So is one still being written when its run ends.
#[test]
fn a_message_that_was_not_sent_is_withdrawn() {
    let session = say_chat();
    run(
        &session,
        &[
            run_started(),
            chunk("c1", Some("say"), r#"{"text":"One"}"#),
            started("c1", "say"),
            finished("c1", "error"),
            chunk("c2", Some("say"), r#"{"text":"Two"#),
            control(json!({"type":"run_finished", "run_id":"r1"})),
        ],
    );
    assert_eq!(
        frames(&with_messages(&session, Some(0), true)),
        [
            "message c1 One",
            "withdrawn c1",
            "message c2 Two",
            "withdrawn c2",
            "display_end",
        ]
    );
}

/// A chunk that begins a call's arguments again, as a runner re-streaming a
/// call sends it, withdraws what was shown and starts afresh.
#[test]
fn arguments_begun_again_withdraw_and_stream_afresh() {
    let session = say_chat();
    run(
        &session,
        &[
            run_started(),
            chunk("c1", Some("say"), r#"{"text":"Draft"#),
            chunk("c1", Some("say"), r#"{"text":"Final"#),
        ],
    );
    assert_eq!(
        frames(&with_messages(&session, Some(0), true)),
        ["message c1 Draft", "withdrawn c1", "message c1 Final"]
    );
}

/// A message's chunks send the reply words held for batching first, so a
/// reader sees the words and the message in the order the model wrote them.
#[test]
fn reply_words_held_for_batching_go_out_before_a_message() {
    let session = say_chat();
    session.ingest(run_started());
    session.ingest(json!({"kind":"display",
        "event":{"type":"text_delta", "mid":"m1", "delta":"Working."}}));
    session.ingest(chunk("c1", Some("say"), r#"{"text":"Hi"#));
    let reply = with_messages(&session, Some(0), true);
    assert_eq!(frames(&reply), ["display", "message c1 Hi"], "{reply}");
}

/// Broca's recorded frames for llm-runner 0.8.65's `tool_name`
/// (`tool_name_1118/`, from broca's run loop and serve path).
///
/// The serve path's frames are a run that calls `work`: its one chunk names
/// the tool and carries the call's message and part. On a chat whose agent
/// talks through `say`, that chunk shows nothing to any reader: every reply is
/// what the same frames give without it.
#[test]
fn brocas_named_chunks_decode_and_other_tools_show_nothing() {
    use super::{map, DisplayInput};
    let lines = |text: &str| -> Vec<Value> {
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    for (name, text) in [
        (
            "live_tool",
            include_str!("tool_name_1118/session_subscribe.live_tool.jsonl"),
        ),
        (
            "live_steer",
            include_str!("tool_name_1118/session_subscribe.live_steer.jsonl"),
        ),
    ] {
        let frames = lines(text);
        let is_chunk = |frame: &Value| frame["event"]["type"] == "tool_input_delta";
        let chunk = frames.iter().find(|frame| is_chunk(frame)).unwrap();
        let Some(DisplayInput::ToolInput(input)) = map(chunk) else {
            panic!("{name}: {chunk}");
        };
        assert_eq!(input.tool_name.as_deref(), Some("work"), "{name}");
        assert_eq!(input.part.as_deref(), Some("0"), "{name}");
        assert_eq!(input.mid, chunk["event"]["mid"].as_str().unwrap(), "{name}");

        let with = say_chat();
        run(&with, &frames);
        let without = say_chat();
        let rest: Vec<Value> = frames.iter().filter(|f| !is_chunk(f)).cloned().collect();
        run(&without, &rest);
        for since in [None, Some(0)] {
            for tools in [false, true] {
                assert_eq!(
                    with_messages(&with, since, tools),
                    with_messages(&without, since, tools),
                    "{name} {since:?} {tools}"
                );
            }
            assert_eq!(
                serde_json::to_value(with.projection.display_read_with_tools(since, NOW)).unwrap(),
                serde_json::to_value(without.projection.display_read_with_tools(since, NOW))
                    .unwrap(),
                "{name} {since:?}"
            );
        }
    }

    // The run loop's own events, before the serve path adds each chunk's
    // message and part: only the first of the call's joined chunks names it.
    let named: Vec<Option<String>> = lines(include_str!(
        "tool_name_1118/tool_input_delta.fragments.jsonl"
    ))
    .into_iter()
    .map(|mut event| {
        event["mid"] = json!("m1");
        match map(&json!({"kind": "display", "event": event})) {
            Some(DisplayInput::ToolInput(input)) => input.tool_name,
            other => panic!("{other:?}"),
        }
    })
    .collect();
    assert_eq!(named, [Some("read".to_owned()), None, None]);
}
