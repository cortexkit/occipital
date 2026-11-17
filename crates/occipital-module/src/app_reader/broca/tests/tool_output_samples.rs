//! The sample set `phone-reader-tool-output-v1`: exact reads of a Broca chat
//! whose tool results Broca stores in its own `output` envelope. Each call's
//! `content` is the text the model was given, through the runner contract's
//! conversion of that envelope (`owned_broca::tool_output`), and a picture or
//! document a tool returned is listed apart under `media`, which the apps fetch
//! by digest with `session.media`. Its README says what each file shows.

use super::served_samples::{decoded, Served};
use super::*;

const SESSION: &str = "alfonso:worker";

/// Entorhinal's record of the one chat.
struct OneChat;

#[async_trait]
impl Entorhinal for OneChat {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        assert_eq!(params["session"], SESSION, "{method}");
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref",
            "incarnation":"entorhinal-test", "ended_at_ms":null, "runner":"broca", "session":SESSION,
            "harness":"broca", "project_root":"/workspace/head", "network":"unknown", "network_history":[]}}),
        )
    }
}

struct OneRoute(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for OneRoute {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// Broca's recording of a tool that returned a text part, a PNG and a PDF
/// (`tests/data/broca-tool-image-reads-1117/`, copied unchanged): the tool
/// result as a `media: "blocks"` read serves it, which is the shape a read
/// asking `tool_media: "refs"` gets, with references and never bytes.
fn recorded_parts_result() -> Value {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-tool-image-reads-1117");
    let sums = std::fs::read_to_string(data.join("SHA256SUMS")).unwrap();
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(data.join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
    let read: Value = serde_json::from_slice(
        &std::fs::read(data.join("tool-image-session-read-blocks.jsonl")).unwrap(),
    )
    .unwrap();
    let message = read["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["message"]["role"] == "tool")
        .unwrap();
    message["message"]["content"][0]["output"].clone()
}

fn entries() -> Vec<Value> {
    let entry = |ordinal: u64, message: Value| {
        json!({"mid": format!("m{ordinal}"), "ordinal": ordinal,
            "written_at": 1_000 * ordinal, "message": message})
    };
    let call = |id: &str, tool: &str| {
        json!({"role": "assistant", "content": [{"type": "tool_call", "tool_call_id": id,
            "tool_name": tool, "input": {"path": "src/lib.rs"}}]})
    };
    let result = |id: &str, output: Value, is_error: bool| {
        json!({"role": "tool", "content": [{"type": "tool_result", "tool_call_id": id,
            "output": output, "is_error": is_error}]})
    };
    vec![
        entry(1, call("call_text", "read")),
        entry(
            2,
            result(
                "call_text",
                json!({"kind": "text", "text": "1: pub fn limit() -> u32 {\n2:     40\n3: }\n"}),
                false,
            ),
        ),
        entry(3, call("call_json", "aft_outline")),
        entry(
            4,
            result(
                "call_json",
                json!({"kind": "json", "value": {"symbols": [{"name": "limit", "line": 1}]}}),
                false,
            ),
        ),
        entry(5, call("call_denied", "write")),
        entry(
            6,
            result(
                "call_denied",
                json!({"kind": "denied", "reason": "the person declined"}),
                false,
            ),
        ),
        entry(7, call("call_error", "bash")),
        entry(
            8,
            result(
                "call_error",
                json!({"kind": "error", "message": "tool provider restarting",
                    "module": "aft", "code": "module_reloading", "nothing_sent": true}),
                true,
            ),
        ),
        entry(9, call("call_media", "browser_use")),
        entry(10, result("call_media", recorded_parts_result(), false)),
    ]
}

/// The one tool part of an exact read's message.
fn tool(reply: &[u8]) -> Value {
    let result = decoded(reply)["result"].clone();
    assert_eq!(result["fullContent"], true, "{result}");
    let parts = result["messages"][0]["parts"].as_array().unwrap().clone();
    let tools: Vec<&Value> = parts.iter().filter(|part| part["type"] == "tool").collect();
    assert_eq!(tools.len(), 1, "{result}");
    tools[0].clone()
}

#[tokio::test]
async fn the_tool_output_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries();
    *route.run_state.lock().unwrap() = Value::Null;
    *route.describe.lock().unwrap() = Some(describe_with(&[
        "attachments",
        "transcript_reads",
        "media_refs",
        "tool_media_refs",
    ]));
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(OneChat))),
            Arc::new(OneRoute(route.clone())),
        ),
    )));
    let mut served = Served::new("phone-reader-tool-output-v1", app);
    let png = "2bb3f6f7eb60006453f46d1a1f6fb72cb30be54fbe604d29a31a8a41cc0a5fe7";
    let pdf = "b7442ba422acb7aa7660b99e95858e049e8fac32dbc1b34a19e285b1adc0ae2f";
    let cases: [(&str, u64, &str, bool, Value); 5] = [
        (
            "01-text",
            1,
            "1: pub fn limit() -> u32 {\n2:     40\n3: }\n",
            false,
            Value::Null,
        ),
        (
            "02-json",
            3,
            r#"{"symbols":[{"line":1,"name":"limit"}]}"#,
            false,
            Value::Null,
        ),
        (
            "03-denied",
            5,
            "denied: the person declined",
            true,
            Value::Null,
        ),
        (
            "04-error",
            7,
            "tool provider restarting\nmodule: aft\ncode: module_reloading\nnothing was sent",
            true,
            Value::Null,
        ),
        (
            "05-picture-and-document",
            9,
            "before-media\nafter-media",
            false,
            json!([
                {"type": "file", "mime": "image/png", "sha256": png, "byteCount": 4749,
                    "width": 640, "height": 160},
                {"type": "file", "mime": "application/pdf", "sha256": pdf, "byteCount": 584},
            ]),
        ),
    ];
    for (corr, (name, ordinal, content, error, media)) in (1..).zip(cases) {
        let request = json!({"method": super::super::super::TRANSCRIPT_PAGE,
            "params": {"session": SESSION, "messageOrdinal": ordinal}});
        let reply = served.read(corr, &request).await;
        let call = tool(&reply);
        assert_eq!(call["content"], content, "{name}: {call}");
        assert_eq!(call["isError"], error, "{name}: {call}");
        assert_eq!(
            call.get("media").cloned().unwrap_or(Value::Null),
            media,
            "{name}: {call}"
        );
        served.keep(name, &request, reply);
    }

    // The newest page: each call's row counts the text the model read, and
    // carries neither the text nor the media.
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": SESSION, "limit": 10}});
    let reply = served.read(6, &request).await;
    let rows = decoded(&reply)["result"]["messages"].clone();
    let calls: Vec<Value> = rows
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .filter(|part| part["type"] == "tool")
        .collect();
    assert_eq!(calls.len(), 5, "{rows}");
    assert_eq!(calls[4]["outputBytes"], "before-media\nafter-media".len());
    for call in &calls {
        assert!(
            call.get("content").is_none() && call.get("media").is_none(),
            "{call}"
        );
    }
    served.keep("06-newest-page", &request, reply);

    // Every read asked for tool media as references, since the runner declares
    // `tool_media_refs`.
    let reads: Vec<Value> = route
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == "session.read")
        .cloned()
        .collect();
    assert!(!reads.is_empty());
    for read in &reads {
        assert_eq!(
            (&read["params"]["media"], &read["params"]["tool_media"]),
            (&json!("media_refs"), &json!("refs")),
            "{read}"
        );
    }

    served.check();
}

/// A runner that declares `media_refs` but not `tool_media_refs` is never asked
/// for tool media, so its reads stay valid; one declaring neither is asked for
/// no media form at all.
#[test]
fn tool_media_is_asked_for_only_where_declared() {
    use cortexkit_role_llm_runner::read::{MediaMode, ToolMedia};
    for (groups, expected) in [
        (
            &[
                "attachments",
                "transcript_reads",
                "media_refs",
                "tool_media_refs",
            ][..],
            (Some(MediaMode::MediaRefs), Some(ToolMedia::Refs)),
        ),
        (
            &["attachments", "transcript_reads", "media_refs"][..],
            (Some(MediaMode::MediaRefs), None),
        ),
        (&[][..], (None, None)),
    ] {
        let describe = check_describe(&describe_with(groups)).unwrap();
        assert_eq!(
            super::super::stored_media(Some(&describe)),
            expected,
            "{groups:?}"
        );
    }
}
