//! The sample set `phone-reader-exact-reads-v1`: exact reads
//! (`session.transcript_page {messageOrdinal}`), which serve a message's tool
//! calls whole for a step log, on an OpenCode 2 head, a Pi head and a Broca
//! worker. Its README says what each file shows. Occipital cuts nothing in an
//! exact read: a call carries its arguments as an object and its result's text
//! as `content`, whatever their size. A call whose result the runner hasn't
//! recorded makes the read a summary with `fullContentUnavailable` instead.

use super::served_samples::{decoded, Served};
use super::*;
use std::collections::HashMap;

const OPENCODE2: &str = "ses_head_oc2";
const PI: &str = "ses_head_pi";
const BROCA: &str = "alfonso:worker";

/// Entorhinal's record of each of the three chats.
struct ThreeChats;

#[async_trait]
impl Entorhinal for ThreeChats {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        let session = params["session"].as_str().unwrap_or_default();
        let (runner, harness) = match session {
            OPENCODE2 => (THALAMUS_RUNNER_MODULE_ID, "opencode2"),
            PI => (THALAMUS_RUNNER_MODULE_ID, "pi"),
            BROCA => ("broca", "broca"),
            other => panic!("unexpected session {other}"),
        };
        let known = match method {
            "agent.session_for_harness" => params["harness"] == harness,
            "agent.session_for_runner" => params["runner"] == runner,
            other => panic!("unexpected Entorhinal call {other}"),
        };
        if !known {
            return Ok(json!({"code":"not_found", "detail":null, "message":"no agent session"}));
        }
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref",
            "incarnation":"entorhinal-test", "ended_at_ms":null, "runner":runner, "session":session,
            "harness":harness, "project_root":"/workspace/head", "network":"unknown", "network_history":[]}}),
        )
    }
}

/// Each chat's runner, by session.
struct PerSession(HashMap<&'static str, Arc<FakeRoute>>);

#[async_trait]
impl BrocaRoutes for PerSession {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0[resolved.reference.session.as_str()].clone())
    }
}

fn route(entries: Vec<Value>) -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries;
    *route.run_state.lock().unwrap() = Value::Null;
    route
}

/// A long tool output: 2,000 lines, 96,000 bytes.
fn long_output() -> String {
    (0..2_000)
        .map(|line| format!("test result {line:05}: ok, 0 failed, 12 passed ...\n"))
        .collect()
}

const ERROR_TEXT: &str = "error[E0425]: cannot find value `limit` in this scope";

fn opencode2() -> Vec<Value> {
    let tool = |id: &str, name: &str, input: Value, state: Value| {
        let mut state = state;
        state["input"] = input;
        json!({"type": "tool", "id": id, "name": name, "state": state})
    };
    let assistant = |ordinal: u64, content: Vec<Value>| {
        let id = format!("msg_{ordinal}");
        json!({"mid": id, "ordinal": ordinal, "written_at": 1_000 * ordinal,
            "message": {"id": id, "type": "assistant",
                "time": {"created": 1_000 * ordinal - 500, "completed": 1_000 * ordinal},
                "content": content}})
    };
    vec![
        assistant(
            1,
            vec![tool(
                "call_long",
                "bash",
                json!({"command": "cargo test --workspace", "timeout": 600000}),
                json!({"status": "completed",
                    "content": [{"type": "text", "text": long_output()}]}),
            )],
        ),
        assistant(
            2,
            vec![tool(
                "call_error",
                "bash",
                json!({"command": "cargo build"}),
                json!({"status": "error", "error": {"type": "unknown", "message": ERROR_TEXT}}),
            )],
        ),
    ]
}

fn pi() -> Vec<Value> {
    let message = |ordinal: u64, message: Value| {
        json!({"mid": format!("entry-{ordinal}"), "ordinal": ordinal, "written_at": 1_000 * ordinal,
            "message": {"type": "message", "id": format!("entry-{ordinal}"),
                "timestamp": "2026-11-07T20:00:00.000Z", "message": message}})
    };
    let result = |call: &str, text: &str, is_error: bool| {
        json!({"role": "toolResult", "toolCallId": call, "toolName": "bash",
            "content": [{"type": "text", "text": text}], "isError": is_error,
            "timestamp": 1_794_000_000_000_u64})
    };
    vec![
        message(
            1,
            json!({"role": "assistant", "stopReason": "toolUse", "timestamp": 1_794_000_000_000_u64,
                "content": [{"type": "toolCall", "id": "call_long", "name": "bash",
                    "arguments": {"command": "cargo test --workspace"}}]}),
        ),
        message(2, result("call_long", &long_output(), false)),
        message(
            3,
            json!({"role": "assistant", "stopReason": "toolUse", "timestamp": 1_794_000_000_000_u64,
                "content": [{"type": "toolCall", "id": "call_error", "name": "bash",
                    "arguments": {"command": "cargo build"}}]}),
        ),
        message(4, result("call_error", ERROR_TEXT, true)),
    ]
}

fn broca() -> Vec<Value> {
    let entry = |ordinal: u64, message: Value| {
        json!({"mid": format!("m{ordinal}"), "ordinal": ordinal,
            "written_at": 1_000 * ordinal, "message": message})
    };
    let call = |id: &str, command: &str| {
        json!({"role": "assistant", "content": [{"type": "tool_call", "tool_call_id": id,
            "tool_name": "bash", "input": {"command": command}}]})
    };
    let result = |id: &str, output: &str, is_error: bool| {
        json!({"role": "tool", "content": [{"type": "tool_result", "tool_call_id": id,
            "output": output, "is_error": is_error}]})
    };
    vec![
        entry(1, call("call_long", "cargo test --workspace")),
        entry(2, result("call_long", &long_output(), false)),
        entry(3, call("call_error", "cargo build")),
        entry(4, result("call_error", ERROR_TEXT, true)),
        // Still running: the runner has stored the call, not its result.
        entry(5, call("call_running", "cargo test --release")),
    ]
}

/// The one tool part of an exact read's message.
fn tool(reply: &[u8]) -> Value {
    let result = decoded(reply)["result"].clone();
    let parts = result["messages"][0]["parts"]
        .as_array()
        .unwrap_or_else(|| panic!("no message parts: {result}"))
        .clone();
    let tools: Vec<&Value> = parts.iter().filter(|part| part["type"] == "tool").collect();
    assert_eq!(tools.len(), 1, "{result}");
    tools[0].clone()
}

#[tokio::test]
async fn the_exact_read_samples_are_what_the_reader_serves() {
    let routes = PerSession(HashMap::from([
        (OPENCODE2, route(opencode2())),
        (PI, route(pi())),
        (BROCA, route(broca())),
    ]));
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(ThreeChats))),
            Arc::new(routes),
        ),
    )));
    let mut served = Served::new("phone-reader-exact-reads-v1", app);
    let exact = |session: &str, harness: Option<&str>, ordinal: u64| {
        let mut params = json!({"session": session, "messageOrdinal": ordinal});
        if let Some(harness) = harness {
            params["harness"] = json!(harness);
        }
        json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": params})
    };

    let cases: [(&str, &str, Option<&str>, u64); 6] = [
        ("01-opencode2-long-output", OPENCODE2, Some("opencode2"), 1),
        ("02-opencode2-error", OPENCODE2, Some("opencode2"), 2),
        ("03-pi-long-output", PI, Some("pi"), 1),
        ("04-pi-error", PI, Some("pi"), 3),
        ("05-broca-long-output", BROCA, None, 1),
        ("06-broca-error", BROCA, None, 3),
    ];
    for (corr, (name, session, harness, ordinal)) in (1..).zip(cases) {
        let request = exact(session, harness, ordinal);
        let reply = served.read(corr, &request).await;
        let result = decoded(&reply)["result"].clone();
        assert_eq!(result["fullContent"], true, "{name}: {result}");
        let call = tool(&reply);
        if name.ends_with("long-output") {
            // Whole, never cut.
            assert_eq!(call["content"], long_output(), "{name}");
            assert_eq!(call["isError"], false, "{name}");
            assert_eq!(
                call["arguments"]["command"], "cargo test --workspace",
                "{name}"
            );
        } else {
            assert_eq!(call["content"], ERROR_TEXT, "{name}");
            assert_eq!(call["isError"], true, "{name}");
            assert!(call["arguments"].is_object(), "{name}");
        }
        served.keep(name, &request, reply);
    }

    // 07: a Broca call whose result isn't recorded yet. The read can't be full,
    // so it is the message's summary row with the reason, and no
    // `fullContent`.
    let request = exact(BROCA, None, 5);
    let reply = served.read(7, &request).await;
    let result = decoded(&reply)["result"].clone();
    assert!(result.get("fullContent").is_none(), "{result}");
    assert!(
        result["fullContentUnavailable"]
            .as_str()
            .is_some_and(|reason| reason.contains("bash")),
        "{result}"
    );
    let call = tool(&reply);
    assert!(
        call.get("content").is_none() && call.get("outputBytes").is_none(),
        "{call}"
    );
    served.keep("07-broca-no-output-yet", &request, reply);

    served.check();
}
