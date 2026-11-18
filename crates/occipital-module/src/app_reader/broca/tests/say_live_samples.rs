//! The sample set `phone-reader-say-live-v1`: an agent's `say` message shown
//! as the model writes it. Its README says what each file shows.
//!
//! An app's display watch, with `acceptToolFrames` and `acceptMessageFrames`,
//! is opened on the chat of `say_samples`, whose plan gives the agent
//! prefrontal's `say`. A run then writes one message whole, answered; begins
//! a second that loses chunks to a gap and is withdrawn by a reset; and ends.
//! A reader without `acceptMessageFrames` sees the first `say` as a tool call.
//! `served_samples` says how the set is checked and rewritten.

use super::claude_code_samples::fixed_clock;
use super::say_samples::{chat, CATALOG, SESSION};
use super::served_samples::{decoded, Served};
use super::*;
use subc_protocol::FrameType;

/// The message's text, as the model writes its arguments: split inside the
/// `text` key, and inside a `\u` escape.
const PIECES: [&str; 4] = [
    r#"{"reply_to":"m1","quote":"Did the build pass?","te"#,
    r#"xt":"Build passed. <Badge tone=\"success\">1,651 tests</Badge> \u00"#,
    r#"e9"#,
    r#"","level":"chat"}"#,
];

fn control(event: Value) -> Value {
    json!({"kind": "control", "event": event})
}

/// A chunk of `call`'s arguments in message `m8`; `tool` names it on the
/// chunk that begins them.
fn chunk(call: &str, tool: Option<&str>, text: &str) -> Value {
    let mut event = json!({"type": "tool_input_delta", "tool_call_id": call, "delta": text,
        "mid": "m8", "part": "0"});
    if let Some(tool) = tool {
        event["tool_name"] = json!(tool);
    }
    json!({"kind": "display", "event": event})
}

/// Every notice the watch sends until it has been quiet for a second.
async fn notices(served: &mut Served) -> Vec<Vec<u8>> {
    let mut bodies = Vec::new();
    while let Some((ty, body)) = served.next(1, Duration::from_secs(1)).await {
        assert_eq!(
            ty,
            FrameType::StreamData,
            "{}",
            String::from_utf8_lossy(&body)
        );
        bodies.push(body);
    }
    bodies
}

/// The frames of every incremental notice, in order.
fn frames(notices: &[Value]) -> Vec<Value> {
    notices
        .iter()
        .filter_map(|notice| notice["frames"].as_array())
        .flatten()
        .map(|sequenced| sequenced["frame"].clone())
        .collect()
}

#[tokio::test(start_paused = true)]
async fn the_say_live_samples_are_what_the_reader_serves() {
    let (app, route) = chat(true);
    *route.run_state.lock().unwrap() = json!({"run_id": "r0", "state": "completed"});
    let mut served = Served::new("phone-reader-say-live-v1", app);
    let session = json!({"session": SESSION, "harness": "broca"});
    let mut watch_params = session.clone();
    watch_params["acceptToolFrames"] = json!(true);
    watch_params["acceptMessageFrames"] = json!(true);
    let watch = json!({"method": super::super::super::DISPLAY_WATCH, "params": watch_params});

    let mut kept: Vec<Value> = Vec::new();
    let mut keep_notices = |served: &mut Served, bodies: Vec<Vec<u8>>, from: &mut usize| {
        for body in bodies {
            kept.push(decoded(&body));
            served.keep_file(&format!("{:02}-watch-notice.json", *from), body);
            *from += 1;
        }
    };
    let mut next = 2;

    // 01, 02: the watch opens on the idle chat and sends its first notice. The
    // page read that opens the live feed holds `say` calls, so it reads the
    // chat's plan and learns that its agent talks through `say`.
    served.keep_file(
        "01-session.display_watch.request.json",
        watch.to_string().into_bytes(),
    );
    served.send(1, &watch).await;
    let opened = notices(&mut served).await;
    assert_eq!(opened.len(), 1, "the first notice");
    keep_notices(&mut served, opened, &mut next);
    for _ in 0..1000 {
        if route.attaches.load(Ordering::Relaxed) == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(route.attaches.load(Ordering::Relaxed), 1);

    // The run starts, and the model writes the first message's arguments.
    *route.run_state.lock().unwrap() = json!({"run_id": "r1", "state": "active"});
    route
        .emit(control(json!({"type": "run_started", "run_id": "r1",
            "rich_ui": {"dialect": "1.0", "catalog_digest": CATALOG}})))
        .await;
    route.emit(chunk("call_live", Some("say"), PIECES[0])).await;
    for piece in &PIECES[1..] {
        route.emit(chunk("call_live", None, piece)).await;
    }
    // The call goes to its tool and is answered; another call's arguments
    // stream meanwhile and show nothing.
    route
        .emit(control(
            json!({"type": "tool_started", "mid": "m8", "tool_call_id": "call_live",
            "name": "say", "arguments": PIECES.concat(), "started_at": 1_794_400_001_000_u64,
            "call_key": "broca:call_live"}),
        ))
        .await;
    route
        .emit(control(
            json!({"type": "tool_finished", "mid": "m8", "tool_call_id": "call_live",
            "outcome": "ok", "output_bytes": 4, "finished_at": 1_794_400_001_200_u64}),
        ))
        .await;
    route
        .emit(chunk("call_read", Some("read"), r#"{"path":"gate.log"}"#))
        .await;
    let streamed = notices(&mut served).await;
    keep_notices(&mut served, streamed, &mut next);

    // A second message begins; a read without a cursor shows it so far.
    route
        .emit(chunk("call_gap", Some("say"), r#"{"text":"Checking the"#))
        .await;
    let streamed = notices(&mut served).await;
    keep_notices(&mut served, streamed, &mut next);
    let mut read_params = session.clone();
    read_params["acceptToolFrames"] = json!(true);
    read_params["acceptMessageFrames"] = json!(true);
    let baseline_read = json!({"method": super::super::super::DISPLAY_READ, "params": read_params});
    let baseline = served.read(2, &baseline_read).await;
    assert_eq!(
        decoded(&baseline)["result"]["messages"],
        json!([{"messageID": "m8", "partKey": "0", "callId": "call_gap", "text": "Checking the",
            "richUi": {"dialect": "1.0", "catalogDigest": CATALOG}}]),
        "{}",
        String::from_utf8_lossy(&baseline)
    );
    served.keep(
        &format!("{next:02}-session.display_read-baseline"),
        &baseline_read,
        baseline,
    );
    next += 1;

    // Chunks are lost: the message freezes, and a reset withdraws it.
    route
        .emit(json!({"kind": "display", "event": {"type": "display_gap", "dropped": 2}}))
        .await;
    route.emit(chunk("call_gap", None, " logs")).await;
    route
        .emit(json!({"kind": "display", "event": {"type": "display_reset"}}))
        .await;
    *route.run_state.lock().unwrap() = json!({"run_id": "r1", "state": "completed"});
    route
        .emit(control(json!({"type": "run_finished", "run_id": "r1"})))
        .await;
    let ended = notices(&mut served).await;
    keep_notices(&mut served, ended, &mut next);

    // A reader of tool frames alone sees the first `say` as a tool call, and
    // none of the messages' text.
    let mut tools_params = session.clone();
    tools_params["acceptToolFrames"] = json!(true);
    tools_params["sinceSeq"] = json!(0);
    let tools_read = json!({"method": super::super::super::DISPLAY_READ, "params": tools_params});
    let tools = served.read(3, &tools_read).await;
    let tool_frames: Vec<Value> = decoded(&tools)["result"]["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sequenced| sequenced["frame"].clone())
        .collect();
    assert!(
        tool_frames
            .iter()
            .all(|frame| frame["partType"] != "message" && frame["kind"] != "message"),
        "{tool_frames:?}"
    );
    assert_eq!(
        tool_frames
            .iter()
            .filter(|frame| frame["kind"] == "tool" && frame["callId"] == "call_live")
            .count(),
        2,
        "{tool_frames:?}"
    );
    // Its tool frames carry the computer's clock as `asOfMs`, fixed here so
    // the set's bytes don't change from run to run.
    served.keep(
        &format!("{next:02}-session.display_read-tools-only"),
        &tools_read,
        fixed_clock(tools),
    );

    // The watch's frames: the first message whole, as decoded; no tool frame
    // for a `say` and nothing for the other call's arguments; the second
    // message's start, frozen by the gap and withdrawn by the reset.
    let frames = frames(&kept);
    let message: String = frames
        .iter()
        .filter(|frame| frame["partType"] == "message" && frame["callId"] == "call_live")
        .map(|frame| {
            assert_eq!(frame["messageID"], "m8", "{frame}");
            assert_eq!(frame["partKey"], "0", "{frame}");
            assert_eq!(frame["richUi"]["catalogDigest"], CATALOG, "{frame}");
            frame["chunk"].as_str().unwrap()
        })
        .collect();
    assert_eq!(
        message,
        "Build passed. <Badge tone=\"success\">1,651 tests</Badge> é"
    );
    assert!(
        frames
            .iter()
            .all(|frame| frame["kind"] != "tool" && frame["callId"] != "call_read"),
        "{frames:?}"
    );
    let rest: Vec<String> = frames
        .iter()
        .filter(|frame| frame["callId"] != "call_live")
        .map(|frame| match frame["kind"].as_str().unwrap() {
            "message" if frame["frozen"] == true => "frozen".to_owned(),
            "message" => "withdrawn".to_owned(),
            "display" => format!("text {}", frame["chunk"].as_str().unwrap()),
            kind => kind.to_owned(),
        })
        .collect();
    assert_eq!(
        rest,
        [
            "text Checking the",
            "display_gap",
            "frozen",
            "display_gap",
            "withdrawn",
            "display_end",
        ],
        "{frames:?}"
    );
    served.check();
}
