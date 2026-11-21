//! Replay Broca's golden subscription bytes through the app watch handlers.

use super::*;
use crate::app_reader::{tests::Connection, DISPLAY_WATCH, TRANSCRIPT_WATCH};
use subc_protocol::{Frame, FrameType};

const VECTORS: &str = "../occipital-app-reader/test-vectors/broca-live-8b931f56";
const SESSION: &str = "alfonso:s1";

struct WireRoute {
    durable: Arc<FakeRoute>,
    sender: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    consumed: Arc<AtomicUsize>,
}

struct WireEvents {
    receiver: mpsc::Receiver<Vec<u8>>,
    consumed: Arc<AtomicUsize>,
    delivered: usize,
}

#[async_trait]
impl EventStream for WireEvents {
    async fn next(&mut self) -> Result<Option<Value>, ReadError> {
        // The pump asks for another frame only after observing the previous one.
        self.consumed.store(self.delivered, Ordering::SeqCst);
        let Some(bytes) = self.receiver.recv().await else {
            return Ok(None);
        };
        self.delivered += 1;
        decode_subscription_frame(&bytes).map(Some)
    }
}

#[async_trait]
impl BrocaRoute for WireRoute {
    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        self.durable.read(op, params).await
    }

    async fn subscribe(&self, _live_ids: bool) -> Result<Box<dyn EventStream>, ReadError> {
        let (sender, receiver) = mpsc::channel(32);
        *self.sender.lock().unwrap() = Some(sender);
        Ok(Box::new(WireEvents {
            receiver,
            consumed: self.consumed.clone(),
            delivered: 0,
        }))
    }
}

struct WireRoutes(Arc<WireRoute>);

#[async_trait]
impl BrocaRoutes for WireRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

async fn settle() {
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
}

fn notices(rx: &mut mpsc::Receiver<Frame>, sample: &str, line: usize) -> Vec<Value> {
    let mut display = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        assert_eq!(
            frame.header.ty,
            FrameType::StreamData,
            "{sample}:{line}: watch errored or ended: {}",
            String::from_utf8_lossy(&frame.body)
        );
        let body: Value = serde_json::from_slice(&frame.body).unwrap();
        if frame.header.corr == 2 {
            display.push(body);
        }
    }
    display
}

fn head_reads(route: &FakeRoute) -> usize {
    route
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == "session.head")
        .count()
}

fn ring(app: &super::super::super::AppReader) -> Value {
    serde_json::to_value(
        app.display
            .runner("broca")
            .projection(SESSION)
            .unwrap()
            .display_read(Some(0)),
    )
    .unwrap()
}

/// Durable reads deliberately omit the in-flight mids: live deltas can arrive
/// before their assistant message is stored.
async fn replay(sample: &str, expected_text: &[(&str, &str)], durable_lines: &[usize]) {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(VECTORS)
            .join(sample),
    )
    .unwrap();
    replay_bytes(sample, &bytes, expected_text, durable_lines, 1).await;
}

async fn replay_bytes(
    sample: &str,
    bytes: &[u8],
    expected_text: &[(&str, &str)],
    durable_lines: &[usize],
    first_seq: usize,
) {
    let durable = FakeRoute::new();
    let route = Arc::new(WireRoute {
        durable: durable.clone(),
        sender: Mutex::new(None),
        consumed: Arc::new(AtomicUsize::new(0)),
    });
    let sources = Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(Lookup))),
        Arc::new(WireRoutes(route.clone())),
    ));
    let app = Arc::new(super::super::super::AppReader::new(sources.clone()));
    let source = sources.source(&resolved().reference).await.unwrap();
    let mut connection = Connection::new(app.clone());
    let (tx, mut rx) = mpsc::channel(256);
    open_watch(&mut connection, &tx, 1, TRANSCRIPT_WATCH).await;
    open_watch(&mut connection, &tx, 2, DISPLAY_WATCH).await;
    users(&source, 2).await;
    settle().await;
    let mut emitted = notices(&mut rx, sample, 0);
    let mut changes = source.changes();
    let mut found = Vec::new();
    let mut refreshed = Vec::new();

    for (index, line) in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .enumerate()
    {
        let line_number = index + 1;
        let frame = decode_subscription_frame(line).unwrap();
        assert_eq!(
            frame["seq"],
            json!(first_seq + index),
            "{sample}:{line_number}"
        );
        let before = *changes.borrow_and_update();
        let reads = head_reads(&durable);
        let previous = ring(&app);
        let sender = route.sender.lock().unwrap().clone().unwrap();
        sender.send(line.to_vec()).await.unwrap();
        settle().await;
        assert_eq!(
            route.consumed.load(Ordering::SeqCst),
            line_number,
            "{sample}:{line_number}: frame rejected by pump"
        );
        assert!(
            source.activity_feed_live(),
            "{sample}:{line_number}: feed ended"
        );
        let is_control = frame["kind"] == "control";
        assert_eq!(
            *changes.borrow_and_update(),
            before + u64::from(is_control),
            "{sample}:{line_number}: transcript change notification"
        );

        let event = frame["event"]["type"].as_str().unwrap_or_default();
        if matches!(
            event,
            "message_started"
                | "message_finished"
                | "tool_started"
                | "tool_finished"
                | "tool_changed"
        ) {
            // A finished message or a call sends the words still held for batching,
            // and a finished message may end them with `display_end`; no lifecycle
            // event, nor a notice for the session's owner such as `tool_changed`,
            // ever drops or replaces what is on the live display.
            let now = ring(&app);
            let before = previous["frames"].as_array().cloned().unwrap_or_default();
            let after = now["frames"].as_array().cloned().unwrap_or_default();
            assert!(
                after.starts_with(&before)
                    && after[before.len()..].iter().all(|frame| {
                        match frame["frame"]["kind"].as_str() {
                            Some("display") => event != "message_started",
                            Some("display_end") => event == "message_finished",
                            _ => false,
                        }
                    }),
                "{sample}:{line_number}: lifecycle event wiped live display: {previous} -> {now}"
            );
        }
        if event == "display_reset" {
            let value = ring(&app);
            assert_eq!(
                value["frames"].as_array().unwrap().last().unwrap()["frame"]["kind"],
                "display_gap",
                "{sample}:{line_number}: reset did not reset"
            );
        }
        // Leave deltas pending until the next lifecycle event, exercising the real
        // batch timer as well as preservation of already published chunks.
        if !matches!(event, "text_delta" | "reasoning_delta") || sample.contains("attach") {
            tokio::time::advance(Duration::from_millis(400)).await;
            settle().await;
        }
        if durable_lines.contains(&line_number) {
            assert!(
                head_reads(&durable) > reads,
                "{sample}:{line_number}: durable control failed to refresh transcript watch"
            );
            refreshed.push(line_number);
        }
        emitted.extend(notices(&mut rx, sample, line_number));
    }
    assert_eq!(refreshed, durable_lines);
    for sequenced in ring(&app)["frames"].as_array().unwrap() {
        let frame = &sequenced["frame"];
        if frame["kind"] == "display" {
            assert_eq!(
                frame["messageID"], "m1",
                "explicit runner mid must beat provider message_id=m"
            );
            assert!(
                frame.get("part").is_none(),
                "display protocol does not yet carry part"
            );
            found.push((
                frame["partType"].as_str().unwrap().to_owned(),
                frame["chunk"].as_str().unwrap().to_owned(),
            ));
        }
    }
    let expected: Vec<_> = expected_text
        .iter()
        .map(|(kind, text)| (kind.to_string(), text.to_string()))
        .collect();
    assert_eq!(found, expected, "{sample}: live text or attach replay lost");
    for (_, text) in expected_text {
        assert!(
            emitted
                .iter()
                .any(|notice| notice.to_string().contains(text)),
            "{sample}: text never reached session.display_watch: {emitted:?}"
        );
    }
    connection.end_route(1, 1);
    connection.shutdown().await;
}

/// Broca's control lane for one step that calls a tool, as its own serializer writes
/// it, before and after it stopped pairing the step's `tool_call` and `step_finished`
/// units with a `display_reset` (Broca's recording, in
/// `tests/data/broca-control-no-reset-1116`). Those resets came
/// right after the step's `message_finished`, so every app dropped words it had just
/// been told were final, at each tool call. Without them those units carry no live
/// event: they never reach the display, keep the feed, and still wake the
/// transcript. Every other frame reads the same in both streams.
#[tokio::test(start_paused = true)]
async fn broca_control_units_without_an_event_drop_no_live_words() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/broca-control-no-reset-1116");
    let frames = |side: &str| {
        let mut files: Vec<_> = std::fs::read_dir(folder.join(side))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        files
            .iter()
            .map(|path| {
                let bytes = std::fs::read(path).unwrap();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                (
                    name,
                    String::from_utf8(bytes).unwrap().trim_end().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let (before, after) = (frames("before"), frames("after"));
    assert_eq!(before.len(), 15);
    // Every control frame wakes the transcript and keeps the feed, in both streams.
    let every_line: Vec<usize> = (1..=15).collect();
    for (side, stream) in [("before", &before), ("after", &after)] {
        let bytes = stream
            .iter()
            .map(|(_, line)| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        replay_bytes(side, bytes.as_bytes(), &[], &every_line, 1).await;
    }
    let live = |line: &str| {
        normalize(serde_json::from_str(line).unwrap())
            .map(|frame| frame["event"]["type"].as_str().unwrap().to_owned())
    };
    for ((name, old), (_, new)) in before.iter().zip(&after) {
        let (old, new) = (live(old), live(new));
        if matches!(
            name.as_str(),
            "05-tool_call.json" | "06-step_finished.json" | "13-step_finished.json"
        ) {
            assert_eq!(old.as_deref(), Some("display_reset"), "{name}");
            assert_eq!(new, None, "{name}");
        } else {
            assert!(old.is_some() && old == new, "{name}: {old:?} {new:?}");
        }
    }
}

/// A call refused because its tool's major version changed, as Broca's serve loop
/// writes it (`tests/data/broca-tool-changed-1121`): the call's `tool_finished` under
/// the `tp2` pin it carried (the tool's schema digest and major version), then
/// `tool_changed` on a `message_boundary` unit. Both are durable
/// control frames that wake the transcript, and neither touches the live words of a
/// reply still streaming: `tool_changed` is a notice for the session's owner.
#[tokio::test(start_paused = true)]
async fn a_tool_changed_frame_keeps_the_live_words() {
    let folder =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-tool-changed-1121");
    let sample = |name: &str| {
        String::from_utf8(std::fs::read(folder.join(name)).unwrap())
            .unwrap()
            .trim_end()
            .to_owned()
    };
    let frames = [
        r#"{"kind":"control","seq":7,"event":{"type":"run_started","run_id":"r1"}}"#.to_owned(),
        r#"{"kind":"control","seq":8,"event":{"type":"message_started","mid":"m1"}}"#.to_owned(),
        r#"{"kind":"display","event":{"type":"text_delta","mid":"m1","part":"0","message_id":"m","delta":"still writing"},"seq":9}"#.to_owned(),
        sample("tool_finished.tp2.frame.json"),
        sample("tool_changed.frame.json"),
    ];
    assert!(frames[4].contains(r#""type":"tool_changed""#));
    replay_bytes(
        "broca-tool-changed",
        frames.join("\n").as_bytes(),
        &[("text", "still writing")],
        &[4, 5],
        7,
    )
    .await;
}

/// The OpenCode runner also streams control frames that carry only a `runner_event`
/// string, such as a send's `send_accepted`, with no `event` the display maps. One of
/// those arriving as a run starts must not end the live feed: the reply's text that
/// follows still reaches session.display_watch.
#[tokio::test(start_paused = true)]
async fn a_runner_status_frame_keeps_the_live_feed() {
    let frames = [
        r#"{"kind":"control","cursor":{"wal_seq":1,"sub_index":0},"runner_event":"send_accepted","seq":1}"#,
        r#"{"kind":"control","cursor":{"wal_seq":1,"sub_index":1},"seq":2,"event":{"type":"run_started","run_id":"r1"}}"#,
        r#"{"kind":"control","cursor":{"wal_seq":2,"sub_index":0},"seq":3,"event":{"type":"message_started","mid":"m1"}}"#,
        r#"{"kind":"display","event":{"type":"display_reset"},"seq":4}"#,
        r#"{"kind":"display","event":{"type":"text_delta","mid":"m1","part":"0","message_id":"m","delta":"answer"},"seq":5}"#,
        r#"{"kind":"control","cursor":{"wal_seq":3,"sub_index":0},"runner_event":"delivered","seq":6}"#,
        r#"{"kind":"control","cursor":{"wal_seq":3,"sub_index":1},"seq":7,"event":{"type":"message_finished","mid":"m1"}}"#,
    ]
    .join("\n");
    replay_bytes(
        "opencode-runner-status-frames",
        frames.as_bytes(),
        &[("text", "answer")],
        &[7],
        1,
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn live_text() {
    replay(
        "session_subscribe.live_text.jsonl",
        &[("text", "answer")],
        &[7],
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn live_tool() {
    replay("session_subscribe.live_tool.jsonl", &[], &[7, 12, 15]).await;
}

#[tokio::test(start_paused = true)]
async fn live_reasoning() {
    replay(
        "session_subscribe.live_reasoning.jsonl",
        &[("reasoning", "thinking"), ("text", "answer")],
        &[8],
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn live_steer() {
    replay("session_subscribe.live_steer.jsonl", &[], &[7, 12, 17]).await;
}

#[tokio::test(start_paused = true)]
async fn live_attach() {
    replay(
        "session_subscribe.live_attach.jsonl",
        &[("text", "answer")],
        &[],
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn broca_0_7_tool_frames_preserve_live_text_and_wake_transcript_watch() {
    for prefix in ["", "approved."] {
        let starts = samples_0_7::samples(&format!("{prefix}tool_started.frame.json"));
        let finishes = samples_0_7::samples(&format!("{prefix}tool_finished.frame.json"));
        let bare_starts = samples_0_7::samples(&format!("{prefix}tool_started.json"));
        let bare_finishes = samples_0_7::samples(&format!("{prefix}tool_finished.json"));
        for (((name, start), (_, finish)), ((_, bare_start), (_, bare_finish))) in starts
            .into_iter()
            .zip(finishes)
            .zip(bare_starts.into_iter().zip(bare_finishes))
        {
            for (frame, bare) in [(&start, &bare_start), (&finish, &bare_finish)] {
                let wire = serde_json::to_vec(frame).unwrap();
                let decoded = decode_subscription_frame(&wire).unwrap();
                assert_eq!(&decoded["event"], bare, "{name}: exact nested event");
                assert_eq!(
                    normalize(decoded).unwrap()["event"],
                    *bare,
                    "{name}: metadata must not be rewritten"
                );
                assert!(bare["call_key"].as_str().unwrap().ends_with(":5"), "{name}");
            }
            if prefix == "approved." {
                assert!(
                    start["event"].get("schema_pin").is_none(),
                    "{name}: preparation precedes dispatch"
                );
            } else {
                assert!(
                    start["event"]["schema_pin"]
                        .as_str()
                        .unwrap()
                        .starts_with("tp1:work:"),
                    "{name}"
                );
            }
            assert!(
                finish["event"]["schema_pin"]
                    .as_str()
                    .unwrap()
                    .starts_with("tp1:work:"),
                "{name}"
            );
            let frames = [
                json!({"kind":"control", "seq":7, "cursor":{"wal_seq":2,"sub_index":0}, "unit":{"type":"message_boundary","mid":"m1"}, "event":{"type":"message_started","mid":"m1"}}),
                json!({"kind":"display", "seq":8, "event":{"type":"text_delta","mid":"m1","part":"0","delta":"answer"}}),
                start,
                finish,
            ];
            let bytes = frames
                .iter()
                .map(|frame| serde_json::to_string(frame).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            replay_bytes(&name, bytes.as_bytes(), &[("text", "answer")], &[3, 4], 7).await;
        }
    }
}
