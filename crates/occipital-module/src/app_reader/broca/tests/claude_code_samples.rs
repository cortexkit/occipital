//! The sample set the apps pin for Claude Code chats, `phone-reader-claude-code-v1`;
//! its README says what each file shows.
//!
//! Every chat is one of the real Claude Code sessions Thalamus recorded in
//! `tests/data/runner-pages-claude-code-v1/` (see `claude_code_pages`), served
//! through Occipital's own handlers as a phone's route reaches them. The
//! recorded reads predate runner contract 0.8.33, so for the chat with tool
//! endings this test adds the `host_parts` fact a runner now serves for
//! Claude Code's two interrupt markers there: one with its code, one without.
//! `served_samples` says how the set is checked and rewritten.

use super::served_samples::{decoded, Served};
use super::*;
use cortexkit_test_support::ScratchDir;
use std::path::{Path, PathBuf};
use subc_protocol::FrameType;

/// Routes to the test session whatever its harness: here, Claude Code.
struct AnyHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for AnyHarness {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// Entorhinal's answer for the test session: a Claude Code head hosted by
/// Thalamus's runner.
struct ClaudeCodeHead;

#[async_trait]
impl Entorhinal for ClaudeCodeHead {
    async fn request(&self, method: &str, _: Value) -> Result<Value, ReadError> {
        assert!(
            matches!(
                method,
                "agent.session_for_harness" | "agent.session_for_runner"
            ),
            "unexpected Entorhinal call {method}"
        );
        Ok(
            json!({"result": {"agent_id": "agent-test", "ref_kind": "alfonso", "ref": "test-ref",
            "incarnation": "entorhinal-test", "ended_at_ms": null,
            "runner": THALAMUS_RUNNER_MODULE_ID, "session": "ses_head", "harness": "claude-code",
            "project_root": "/workspace/head", "network": "unknown", "network_history": []}}),
        )
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/runner-pages-claude-code-v1")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap())
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// The entries of a recorded case's read with display facts and media
/// references, as a page read asks for them.
fn entries(case: &str) -> Vec<Value> {
    sample(&format!("{case}/session-read-media-refs.json"))["messages"]
        .as_array()
        .unwrap()
        .clone()
}

/// The recorded session's call that a module's note rode: one sent by
/// `magic-context`, the context-management module, riding the call's result.
const RIDDEN_CALL: &str = "msg_notices_call/toolu_notices_echo";

/// The time the set's tool frames name as `asOfMs`, when the computer served them.
const AS_OF_MS: &str = "1794208800000";

/// `body` with every tool frame's `asOfMs` fixed at [`AS_OF_MS`]. The watch
/// stamps it from the computer's clock as it serves each notice, so it is the
/// one member that would change the set's bytes from run to run. Only the
/// digits are replaced, so the rest of the body is the bytes the app receives.
fn fixed_clock(body: Vec<u8>) -> Vec<u8> {
    let text = String::from_utf8(body).unwrap();
    let mut fixed = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find("\"asOfMs\":") {
        let start = at + "\"asOfMs\":".len();
        fixed.push_str(&rest[..start]);
        fixed.push_str(AS_OF_MS);
        let digits = rest[start..].bytes().take_while(u8::is_ascii_digit).count();
        rest = &rest[start + digits..];
    }
    fixed.push_str(rest);
    fixed.into_bytes()
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
        bodies.push(fixed_clock(body));
    }
    bodies
}

#[tokio::test(start_paused = true)]
async fn the_claude_code_samples_are_what_the_reader_serves() {
    let dir = ScratchDir::new("occipital-claude-code-samples");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notice_writer, _task) = crate::notices::Notices::start(file.clone());
    let slot = crate::notices::NoticeSlot::default();
    let _ = slot.set(notice_writer.clone());

    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&[
        "attachments",
        "transcript_reads",
        "media_refs",
    ]));
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(ClaudeCodeHead))),
            Arc::new(AnyHarness(route.clone())),
        )
        .with_notices(slot),
    )));
    app.set_notices(notice_writer);
    let mut served = Served::new("phone-reader-claude-code-v1", app);
    let session = json!({"session": "ses_head", "harness": "claude-code"});
    let mut page_request =
        json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": session.clone()});
    page_request["params"]["limit"] = json!(50);
    let mut corr = 0;
    let mut page = |chat: Vec<Value>| {
        *route.entries.lock().unwrap() = chat;
        corr += 1;
        (corr, page_request.clone())
    };

    // 01: Claude's thinking is a reasoning part, the second one empty.
    let (corr_01, request) = page(entries("01-text-and-reasoning"));
    let reply = served.read(corr_01, &request).await;
    let rows = decoded(&reply)["result"]["messages"].clone();
    let reasoning: Vec<Value> = rows
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["type"] == "reasoning")
        .map(|part| part["text"].clone())
        .collect();
    assert_eq!(reasoning.len(), 2, "{rows}");
    assert_eq!(reasoning[1], "");
    served.keep("01-page-reasoning", &request, reply);

    // 02: how each call ended, and Claude Code's two interrupt markers as
    // host parts kept out of the person's text.
    let mut chat = entries("02-tool-outcomes");
    for entry in &mut chat {
        match entry["ordinal"].as_u64() {
            Some(14) => entry["host_parts"] = json!([{"part": "1", "host_code": "interrupted"}]),
            Some(16) => entry["host_parts"] = json!([{"part": "1"}]),
            _ => {}
        }
    }
    let (corr_02, request) = page(chat);
    let reply = served.read(corr_02, &request).await;
    let rows = decoded(&reply)["result"]["messages"].clone();
    let rows = rows.as_array().unwrap();
    let hosts: Vec<(Value, Value)> = rows
        .iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["host"] == true)
        .map(|part| (part["type"].clone(), part["hostCode"].clone()))
        .collect();
    assert_eq!(
        hosts,
        [
            (json!("injected"), json!("interrupted")),
            (json!("injected"), Value::Null)
        ]
    );
    assert!(rows.iter().all(|row| !row["text"]
        .as_str()
        .unwrap_or("")
        .contains("[Request interrupted")));
    let endings: Vec<Value> = rows
        .iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["type"] == "tool")
        .map(|part| part["unanswered"].clone())
        .collect();
    assert!(endings.contains(&json!("not_executed")), "{endings:?}");
    assert!(endings.contains(&json!("cancelled")), "{endings:?}");
    served.keep("02-page-tool-endings", &request, reply);

    // 03, 04: a shared photo and PDF are file parts, and the photo's bytes
    // come whole through `session.media` under the digest its part names.
    let photo = crate::base64::decode(
        sample("03-photo-and-file/session-media-read-photo.json")["data"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let digest = route.hold_media("image/png", &photo);
    let (corr_03, request) = page(entries("03-photo-and-file"));
    let reply = served.read(corr_03, &request).await;
    let files: Vec<Value> = decoded(&reply)["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["type"] == "file")
        .collect();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["sha256"], digest);
    served.keep("03-page-files", &request, reply);
    let mut media = json!({"method": super::super::super::MEDIA, "params": session.clone()});
    media["params"]["sha256"] = json!(digest);
    media["params"]["size"] = json!("full");
    let full = served.read(50, &media).await;
    assert_eq!(
        crate::base64::decode(decoded(&full)["result"]["data"].as_str().unwrap()).unwrap(),
        photo
    );
    served.keep("04-photo-full", &media, full);

    // 05, 06: core's notice and words it passed on carry their marks, and a
    // note that rode a call is read beside the page.
    let (corr_05, request) = page(entries("08-notices-and-marks"));
    let reply = served.read(corr_05, &request).await;
    let rows = decoded(&reply)["result"]["messages"].clone();
    assert!(rows
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["notice"].is_object()));
    assert!(rows
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["speaksFor"] == json!({"agentId": "agent-a"})));
    served.keep("05-page-marks", &request, reply);
    for _ in 0..1_000 {
        if !file
            .for_call(THALAMUS_RUNNER_MODULE_ID, "ses_head", RIDDEN_CALL)
            .unwrap()
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let mut read = json!({"method": crate::notices::READ, "params": session.clone()});
    read["params"]["callKeys"] = json!([RIDDEN_CALL]);
    let notes = served.read(51, &read).await;
    assert_eq!(
        decoded(&notes)["result"]["calls"][RIDDEN_CALL][0]["sender"],
        "magic-context"
    );
    served.keep("06-notices", &read, notes);

    // 07: the phone's message carries the app message id it was sent under.
    let (corr_07, request) = page(entries("09-client-and-send-ids"));
    let reply = served.read(corr_07, &request).await;
    assert!(decoded(&reply)["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["clientMessageId"] == "phone-message-1"));
    served.keep("07-page-phone-message", &request, reply);

    // 08 on: a display watch on a chat as Claude Code runs a long call. The
    // reply's words stream first; the call's start reaches the watch only once
    // the call has returned, with its end right after.
    *route.entries.lock().unwrap() = entries("10-long-call");
    let mut watch_params = session.clone();
    watch_params["acceptToolFrames"] = json!(true);
    let watch = json!({"method": super::super::super::DISPLAY_WATCH, "params": watch_params});
    served.keep_file(
        "08-session.display_watch.request.json",
        watch.to_string().into_bytes(),
    );
    served.send(1, &watch).await;
    let mut kept = Vec::new();
    let mut next = 9;
    for body in notices(&mut served).await {
        kept.push(decoded(&body));
        served.keep_file(&format!("{next:02}-watch-notice.json"), body);
        next += 1;
    }
    for _ in 0..1_000 {
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
    let recorded = sample("10-long-call/subscribe-frames.json");
    for frame in recorded["subscriptions"][0]["frames"].as_array().unwrap() {
        route.emit(frame.clone()).await;
    }
    for body in notices(&mut served).await {
        kept.push(decoded(&body));
        served.keep_file(&format!("{next:02}-watch-notice.json"), body);
        next += 1;
    }
    let frames: Vec<Value> = kept[1..]
        .iter()
        .flat_map(|notice| notice["frames"].as_array().cloned().unwrap_or_default())
        .map(|sequenced| sequenced["frame"].clone())
        .collect();
    let calls: Vec<(Value, Value)> = frames
        .iter()
        .filter(|frame| frame["kind"] == "tool")
        .map(|frame| (frame["callId"].clone(), frame["phase"].clone()))
        .collect();
    assert_eq!(
        calls,
        [
            (json!("toolu_long_call"), json!("running")),
            (json!("toolu_long_call"), json!("answered"))
        ],
        "{frames:?}"
    );
    assert!(frames.iter().any(|frame| frame["kind"] == "display"));
    served.check();
}
