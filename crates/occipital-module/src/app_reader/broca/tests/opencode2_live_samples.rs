//! The sample set the apps pin for an OpenCode 2 reply streamed live,
//! `phone-reader-opencode2-live-v1`; its README says what each file shows.
//!
//! An app's display watch is opened on an idle OpenCode 2 chat, as the phone
//! opens one while the chat is on screen, before the run starts. The runner
//! (Thalamus, module `thalamus-harness`) then starts a run carrying a rich UI
//! stamp, streams one reply in pieces, stores it and ends the run.
//! `session.details` is read while the run is active and after it ends. The
//! text is invented. `served_samples` says how the set is checked and
//! rewritten.

use super::served_samples::{decoded, Served};
use super::*;
use subc_protocol::FrameType;

/// Routes to the test session whatever its harness: here, OpenCode 2.
struct AnyHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for AnyHarness {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// The session the test resolver knows (`ByHarness`).
const SESSION: &str = "ses_head";
const REPLY: &str = "msg_reply";
/// The stamp runners put on a run's replies: rich UI dialect 1.0 under catalog
/// 1.0's original digest, which every app build accepts.
const CATALOG: &str = "beb0904e8b402916d84d7609a8b06db30ac308f692dcc048db2dc1138cd0c986";
/// The reply's words, in the pieces the runner streams them in.
const PIECES: [&str; 5] = [
    "# Repotting a fern\n\n",
    "Water it the day before, ",
    "so the roots slide out whole.",
    " Choose a pot one size up, ",
    "and keep it out of direct sun for a week.",
];

fn question() -> Value {
    json!({"mid": "msg_question", "ordinal": 1, "written_at": 1_793_600_000_000_u64,
        "message": {"id": "msg_question", "type": "user",
            "time": {"created": 1_793_600_000_000_u64},
            "text": "How do I repot a fern?"}})
}

fn reply() -> Value {
    json!({"mid": REPLY, "ordinal": 2, "written_at": 1_793_600_009_000_u64,
        "message": {"id": REPLY, "type": "assistant",
            "time": {"created": 1_793_600_000_400_u64, "completed": 1_793_600_008_900_u64},
            "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
            "content": [{"type": "text", "text": PIECES.concat()}]}})
}

fn control(event: Value) -> Value {
    json!({"kind": "control", "event": event})
}

fn piece(text: &str) -> Value {
    json!({"kind": "display", "event": {"type": "text_delta", "mid": REPLY, "part": "0", "delta": text}})
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

#[tokio::test(start_paused = true)]
async fn the_opencode2_live_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = vec![question()];
    *route.run_state.lock().unwrap() = json!({"run_id": "r0", "state": "completed"});
    // The head's model, which the details show. The test runner has no send
    // path, so the details carry no `sender`.
    route.head_extra.lock().unwrap().insert(
        "model".into(),
        json!({"provider": "anthropic", "model": "claude-opus-5-5"}),
    );
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                THALAMUS_RUNNER_MODULE_ID,
            ))))),
            Arc::new(AnyHarness(route.clone())),
        ),
    )));
    let mut served = Served::new("phone-reader-opencode2-live-v1", app);
    let session = json!({"session": SESSION, "harness": "opencode2"});
    let mut watch_params = session.clone();
    watch_params["acceptToolFrames"] = json!(true);
    let watch = json!({"method": super::super::super::DISPLAY_WATCH, "params": watch_params});
    let details = json!({"method": super::super::super::DETAILS, "params": session});

    let mut kept: Vec<Value> = Vec::new();
    let mut keep_notices = |served: &mut Served, bodies: Vec<Vec<u8>>, from: &mut usize| {
        for body in bodies {
            kept.push(decoded(&body));
            served.keep_file(&format!("{:02}-watch-notice.json", *from), body);
            *from += 1;
        }
    };
    let mut next = 2;

    // 01, 02: the watch opens on the idle chat and sends its first notice.
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
    assert_eq!(
        route.attaches.load(Ordering::Relaxed),
        1,
        "the runner is followed"
    );

    // The run starts, stamped, and the reply's first pieces stream.
    *route.run_state.lock().unwrap() = json!({"run_id": "r1", "state": "active"});
    route
        .emit(control(json!({"type": "run_started", "run_id": "r1",
            "rich_ui": {"dialect": "1.0", "catalog_digest": CATALOG}})))
        .await;
    route
        .emit(control(json!({"type": "message_started", "mid": REPLY})))
        .await;
    for text in &PIECES[..3] {
        route.emit(piece(text)).await;
    }
    let streamed = notices(&mut served).await;
    keep_notices(&mut served, streamed, &mut next);

    // The details while the run is active.
    let active = served.read(2, &details).await;
    assert_eq!(decoded(&active)["result"]["lastRun"]["state"], "active");
    served.keep(
        &format!("{next:02}-session.details-active"),
        &details,
        active,
    );
    next += 1;

    // The rest of the reply, then it's stored and the run ends.
    for text in &PIECES[3..] {
        route.emit(piece(text)).await;
    }
    let streamed = notices(&mut served).await;
    keep_notices(&mut served, streamed, &mut next);
    route.entries.lock().unwrap().push(reply());
    route
        .emit(control(json!({"type": "message_finished", "mid": REPLY})))
        .await;
    *route.run_state.lock().unwrap() = json!({"run_id": "r1", "state": "completed"});
    route
        .emit(control(json!({"type": "run_finished", "run_id": "r1"})))
        .await;
    let ended = notices(&mut served).await;
    keep_notices(&mut served, ended, &mut next);

    let after = served.read(3, &details).await;
    assert_eq!(decoded(&after)["result"]["lastRun"]["state"], "completed");
    served.keep(&format!("{next:02}-session.details-after"), &details, after);

    // The first notice is one frame; every later one is a batch. The reply's
    // words arrive whole, under its id and the run's stamp, and the message's
    // end and the run's end follow.
    assert!(kept[0].get("frame").is_some(), "{}", kept[0]);
    let frames: Vec<&Value> = kept[1..]
        .iter()
        .flat_map(|notice| notice["frames"].as_array().unwrap())
        .map(|sequenced| &sequenced["frame"])
        .collect();
    let text: String = frames
        .iter()
        .filter(|frame| frame["kind"] == "display")
        .map(|frame| {
            assert_eq!(frame["messageID"], REPLY, "{frame}");
            assert_eq!(frame["richUi"]["catalogDigest"], CATALOG, "{frame}");
            frame["chunk"].as_str().unwrap()
        })
        .collect();
    assert_eq!(text, PIECES.concat());
    let ends: Vec<&str> = frames
        .iter()
        .skip_while(|frame| frame["kind"] == "display")
        .map(|frame| frame["kind"].as_str().unwrap())
        .collect();
    assert_eq!(ends, ["display_end", "display_end"], "{frames:?}");
    served.check();
}
