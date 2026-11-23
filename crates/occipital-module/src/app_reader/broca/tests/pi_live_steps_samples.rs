//! The sample set `phone-reader-live-ids-v2`: a Pi agent's steps with tool
//! calls, as an app's display watch and page reads see them. Its README says
//! what each file shows.
//!
//! Pi's runner streams a step's words under a `live_id` until its host stores
//! the step, then names it by its `mid`. Its calls run only after that, so
//! their events name the `mid`. A frame about a `mid` whose words were shown
//! under a `live_id` in this run carries that `live_id`, so the step's words
//! and calls reach an app under one key until the stored copy replaces them.
//! A step with no words shown (a call alone) keeps its `mid`.
//! The `served_samples` module beside this one says how the set is checked
//! and rewritten.

use super::claude_code_samples::fixed_clock;
use super::served_samples::{decoded, Served};
use super::*;
use subc_protocol::FrameType;

const SESSION: &str = "ses_head";
const SET: &str = "phone-reader-live-ids-v2";
const PIN: &str = "tp1:read:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:1";

/// Routes to the test session, which must be the Pi one.
struct PiHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for PiHarness {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "pi");
        Ok(self.0.clone())
    }
}

fn control(event: Value) -> Value {
    json!({"kind": "control", "event": event})
}

fn display(event: Value) -> Value {
    json!({"kind": "display", "event": event})
}

/// One stored Pi journal entry as the runner serves it.
fn entry(ordinal: u64, message: Value) -> Value {
    json!({"mid": message["id"], "ordinal": ordinal, "written_at": 1_794_400_000_000_u64 + ordinal,
        "message": message})
}

fn user(id: &str, text: &str) -> Value {
    json!({"type": "message", "id": id, "message": {"role": "user",
        "content": [{"type": "text", "text": text}], "timestamp": 1_794_400_000_000_u64}})
}

/// An assistant step: its parts, ending in a call of `tool`.
fn step(id: &str, words: Value, call: &str, tool: &str, arguments: Value) -> Value {
    json!({"type": "message", "id": id, "message": {"role": "assistant",
        "content": [words, {"type": "toolCall", "id": call, "name": tool, "arguments": arguments}],
        "provider": "probe", "model": "probe", "stopReason": "toolUse",
        "timestamp": 1_794_400_000_000_u64}})
}

fn call_only(id: &str, call: &str, tool: &str, arguments: Value) -> Value {
    json!({"type": "message", "id": id, "message": {"role": "assistant",
        "content": [{"type": "toolCall", "id": call, "name": tool, "arguments": arguments}],
        "provider": "probe", "model": "probe", "stopReason": "toolUse",
        "timestamp": 1_794_400_000_000_u64}})
}

fn result(id: &str, call: &str, tool: &str, text: &str) -> Value {
    json!({"type": "message", "id": id, "message": {"role": "toolResult", "toolCallId": call,
        "toolName": tool, "content": [{"type": "text", "text": text}], "isError": false,
        "timestamp": 1_794_400_000_000_u64}})
}

fn started(mid: &str, call: &str, tool: &str, arguments: &Value, at: u64) -> Value {
    control(
        json!({"type": "tool_started", "mid": mid, "tool_call_id": call, "name": tool,
        "arguments": arguments.to_string(), "arguments_truncated": false,
        "call_key": format!("run-session/{mid}/{call}"), "schema_pin": PIN, "started_at": at}),
    )
}

fn finished(mid: &str, call: &str, bytes: u64, at: u64) -> Value {
    control(
        json!({"type": "tool_finished", "mid": mid, "tool_call_id": call,
        "call_key": format!("run-session/{mid}/{call}"), "outcome": "ok", "output_bytes": bytes,
        "finished_at": at, "schema_pin": PIN}),
    )
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
async fn the_pi_live_step_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["live_events", "live_ids"]));
    *route.run_state.lock().unwrap() = json!({"run_id": "r0", "state": "completed"});
    *route.entries.lock().unwrap() =
        vec![entry(1, user("u1", "Check the plan, then run the tests."))];
    route.history.store(1, Ordering::Relaxed);
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                PI_RUNNER_MODULE_ID,
            ))))),
            Arc::new(PiHarness(route.clone())),
        ),
    )));
    let mut served = Served::new(SET, app);
    let session = json!({"session": SESSION, "harness": "pi"});
    let with = |extra: Value| {
        let mut params = session.clone();
        for (name, value) in extra.as_object().unwrap() {
            params[name] = value.clone();
        }
        params
    };
    let watch = json!({"method": super::super::super::DISPLAY_WATCH,
        "params": with(json!({"acceptToolFrames": true}))});
    let page = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": with(json!({"limit": 20}))});
    let baseline_read = json!({"method": super::super::super::DISPLAY_READ,
        "params": with(json!({"acceptToolFrames": true}))});

    let mut watched: Vec<Value> = Vec::new();
    let mut next = 2;
    let mut keep_notices = |served: &mut Served, bodies: Vec<Vec<u8>>, next: &mut usize| {
        for body in bodies {
            watched.push(decoded(&body));
            served.keep_file(&format!("{:02}-watch-notice.json", *next), body);
            *next += 1;
        }
    };

    // 01: the watch opens on the idle chat.
    served.keep_file(
        "01-session.display_watch.request.json",
        watch.to_string().into_bytes(),
    );
    served.send(1, &watch).await;
    let opened = notices(&mut served).await;
    keep_notices(&mut served, opened, &mut next);
    for _ in 0..1000 {
        if route.attaches.load(Ordering::Relaxed) == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(*route.live_ids.lock().unwrap(), [true]);

    // The first step: reasoning under its `live_id`, stored as `a1`, then its
    // call runs under the `mid`.
    *route.run_state.lock().unwrap() = json!({"run_id": "r1", "state": "active"});
    let read_args = json!({"path": "plan.md"});
    for event in [
        control(json!({"type": "run_started", "run_id": "r1"})),
        display(
            json!({"type": "reasoning_delta", "live_id": "live-step-1", "delta": "Reading the plan "}),
        ),
        display(json!({"type": "reasoning_delta", "live_id": "live-step-1", "delta": "first."})),
        control(json!({"type": "message_started", "mid": "a1", "live_id": "live-step-1"})),
        control(json!({"type": "message_finished", "mid": "a1"})),
        started("a1", "call_read", "read", &read_args, 1_794_400_001_000),
    ] {
        route.emit(event).await;
    }
    let first = notices(&mut served).await;
    keep_notices(&mut served, first, &mut next);

    // The host stored the step before its call ran; a page and a baseline read
    // taken while the call runs.
    {
        let mut entries = route.entries.lock().unwrap();
        entries.push(entry(
            2,
            step(
                "a1",
                json!({"type": "thinking", "thinking": "Reading the plan first."}),
                "call_read",
                "read",
                read_args.clone(),
            ),
        ));
    }
    route.history.store(2, Ordering::Relaxed);
    let running_page = served.read(2, &page).await;
    served.keep(
        &format!("{next:02}-page-step-stored-call-running"),
        &page,
        running_page,
    );
    next += 1;
    let baseline = fixed_clock(served.read(3, &baseline_read).await);
    let running: Vec<Value> = decoded(&baseline)["result"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(running.len(), 1, "{}", String::from_utf8_lossy(&baseline));
    assert_eq!(running[0]["messageID"], "live-step-1");
    assert_eq!(running[0]["callKey"], "run-session/a1/call_read");
    served.keep(
        &format!("{next:02}-session.display_read-baseline-call-running"),
        &baseline_read,
        baseline,
    );
    next += 1;

    // The call is answered and its result stored.
    route
        .emit(finished("a1", "call_read", 12, 1_794_400_001_400))
        .await;
    let answered = notices(&mut served).await;
    keep_notices(&mut served, answered, &mut next);
    route
        .entries
        .lock()
        .unwrap()
        .push(entry(3, result("t1", "call_read", "read", "Plan: 3 steps")));
    route.history.store(3, Ordering::Relaxed);
    let answered_page = served.read(4, &page).await;
    served.keep(
        &format!("{next:02}-page-step-call-answered"),
        &page,
        answered_page,
    );
    next += 1;

    // The second step: text before its call. The third: a call alone, whose
    // arguments stream under a `live_id` but show no words, so it keeps its `mid`.
    let test_args = json!({"command": "cargo test"});
    let bash_args = json!({"command": "git status"});
    for event in [
        display(json!({"type": "text_delta", "live_id": "live-step-2", "delta": "Now the tests."})),
        control(json!({"type": "message_started", "mid": "a3", "live_id": "live-step-2"})),
        control(json!({"type": "message_finished", "mid": "a3"})),
        started("a3", "call_test", "bash", &test_args, 1_794_400_002_000),
        finished("a3", "call_test", 40, 1_794_400_002_500),
        display(json!({"type": "tool_input_delta", "live_id": "live-step-3",
            "tool_call_id": "call_status", "tool_name": "bash", "part": "0",
            "delta": bash_args.to_string()})),
        control(json!({"type": "message_started", "mid": "a5", "live_id": "live-step-3"})),
        control(json!({"type": "message_finished", "mid": "a5"})),
        started("a5", "call_status", "bash", &bash_args, 1_794_400_003_000),
        finished("a5", "call_status", 9, 1_794_400_003_200),
    ] {
        route.emit(event).await;
    }
    *route.run_state.lock().unwrap() = json!({"run_id": "r1", "state": "completed"});
    route
        .emit(control(json!({"type": "run_finished", "run_id": "r1"})))
        .await;
    let rest = notices(&mut served).await;
    keep_notices(&mut served, rest, &mut next);
    {
        let mut entries = route.entries.lock().unwrap();
        entries.push(entry(
            4,
            step(
                "a3",
                json!({"type": "text", "text": "Now the tests."}),
                "call_test",
                "bash",
                test_args,
            ),
        ));
        entries.push(entry(
            5,
            result("t3", "call_test", "bash", "test result: ok"),
        ));
        entries.push(entry(6, call_only("a5", "call_status", "bash", bash_args)));
        entries.push(entry(7, result("t5", "call_status", "bash", "clean")));
    }
    route.history.store(7, Ordering::Relaxed);
    let final_page = served.read(5, &page).await;
    served.keep(&format!("{next:02}-page-run-ended"), &page, final_page);

    // Every watched frame about a step carries one key: a step whose words
    // were shown keeps their `live_id`, and the call alone keeps its `mid`.
    let frames: Vec<Value> = watched
        .iter()
        .filter_map(|notice| notice["frames"].as_array())
        .flatten()
        .map(|sequenced| sequenced["frame"].clone())
        .collect();
    let keys: Vec<(String, String)> = frames
        .iter()
        .filter_map(|frame| {
            let id = frame["messageID"].as_str()?;
            let what = frame["callId"]
                .as_str()
                .map_or_else(|| frame["kind"].as_str().unwrap().to_owned(), str::to_owned);
            Some((what, id.to_owned()))
        })
        .collect();
    for (what, id) in &keys {
        let expected = match what.as_str() {
            "call_read" => "live-step-1",
            "call_test" => "live-step-2",
            "call_status" => "a5",
            _ => continue,
        };
        assert_eq!(id, expected, "{what}: {keys:?}");
    }
    for mid in ["a1", "a3"] {
        assert!(!keys.iter().any(|(_, id)| id == mid), "{mid}: {keys:?}");
    }
    assert_eq!(
        keys.iter()
            .filter(|(what, _)| what.starts_with("call_"))
            .count(),
        6,
        "{keys:?}"
    );
    served.check();
}
