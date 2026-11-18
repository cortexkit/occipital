//! The sample set `phone-reader-say-v1`: an agent's `say` calls as the apps
//! get them. On a chat whose plan gives the agent prefrontal's `say`, an
//! answered call is the agent's message, a `message` part in the call's place;
//! a call that failed stays a tool card; one still running is left out. On any
//! other chat the same calls are tool cards. `session.details` names the
//! chat's message tool to an app that asks. Its README says what each file shows.

use super::served_samples::{decoded, Served};
use super::*;

const SESSION: &str = "alfonso:head";
const TEXT: &str = "Build passed. <Badge tone=\"success\">1,651 tests</Badge>";

/// The reply catalog the runner stamped the agent's turns with, so the apps
/// draw a message's tags as they draw a reply's.
const CATALOG: &str = "4e256f56bf6a6f69d85f0bd9a101fd53637784fd76f7e8c53447069fd1dd77f5";

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

/// The chat: the person's question, a reply that reads a file and answers by
/// `say`, a `say` that failed, and a `say` the model is still writing.
fn entries() -> Vec<Value> {
    let entry = |ordinal: u64, message: Value| {
        let mut entry = json!({"mid": format!("m{ordinal}"), "ordinal": ordinal,
            "written_at": 1_000 * ordinal, "message": message});
        if entry["message"]["role"] == "assistant" {
            entry["rich_ui"] = json!({"dialect": "1.0", "catalog_digest": CATALOG});
        }
        entry
    };
    let call = |id: &str, tool: &str, input: Value| json!({"type": "tool_call", "tool_call_id": id, "tool_name": tool, "input": input});
    let result = |id: &str, text: &str, is_error: bool| {
        json!({"role": "tool", "content": [{"type": "tool_result", "tool_call_id": id,
            "output": {"kind": "text", "text": text}, "is_error": is_error}]})
    };
    vec![
        entry(
            1,
            json!({"role": "user", "content": [{"type": "text", "text": "Did the build pass?"}]}),
        ),
        entry(
            2,
            json!({"role": "assistant", "content": [
                {"type": "text", "text": "Reading the gate log."},
                call("call_read", "read", json!({"path": "gate.log"})),
                call("call_say", "say", json!({"reply_to": "1", "quote": "Did the build pass?",
                    "text": TEXT, "level": "chat"})),
            ]}),
        ),
        entry(
            3,
            result("call_read", "test result: ok. 1651 passed", false),
        ),
        entry(4, result("call_say", "sent", false)),
        entry(
            5,
            json!({"role": "assistant", "content": [
                call("call_failed", "say", json!({"text": "The release is out.", "level": "attention",
                    "key": "release", "done": true})),
            ]}),
        ),
        entry(
            6,
            result("call_failed", "invalid level for this agent", true),
        ),
        entry(
            7,
            json!({"role": "assistant", "content": [
                call("call_running", "say", json!({"text": "Starting the next step."})),
            ]}),
        ),
    ]
}

/// The plan in force: prefrontal's `say` beside AFT's `read`, or `read` alone.
fn plan(with_say: bool) -> Value {
    let mut tools = vec![json!({"name": "read", "description": "Read a file.",
        "input_schema": {"type": "object"}, "provider": "aft"})];
    if with_say {
        tools.push(
            json!({"name": "say", "description": "Send the person a message.",
            "input_schema": {"type": "object", "properties": {"text": {"type": "string"}}},
            "provider": "prefrontal-core"}),
        );
    }
    json!({"generation": 9, "applied_at_ms": 1_794_400_000_000_u64,
        "system": {"text": "Be brief."}, "tools": tools, "as_planned": true})
}

/// An app reading the chat, on a runner whose plan is `plan(with_say)`.
fn chat(with_say: bool) -> (Arc<super::super::super::AppReader>, Arc<FakeRoute>) {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries();
    *route.run_state.lock().unwrap() = Value::Null;
    *route.describe.lock().unwrap() = Some(describe_with(&[
        "attachments",
        "transcript_reads",
        "plan_view",
    ]));
    *route.plan_view.lock().unwrap() = Some(Ok(plan(with_say)));
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(OneChat))),
            Arc::new(OneRoute(route.clone())),
        ),
    )));
    (app, route)
}

/// The parts of the row with message id `mid` on a newest page.
fn parts(page: &Value, mid: &str) -> Vec<Value> {
    page["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["messageId"] == mid)
        .unwrap_or_else(|| panic!("{page}"))["parts"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn the_say_samples_are_what_the_reader_serves() {
    let (app, route) = chat(true);
    let mut served = Served::new("phone-reader-say-v1", app);
    let newest = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": SESSION}});

    let reply = served.read(1, &newest).await;
    let page = decoded(&reply);
    let answer = parts(&page, "m2");
    assert_eq!(answer.len(), 3, "{page}");
    let stamped = page["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["messageId"] == "m2")
        .unwrap();
    assert_eq!(stamped["richUi"]["catalogDigest"], CATALOG, "{stamped}");
    assert_eq!(answer[1]["type"], "tool");
    assert_eq!(
        answer[2],
        json!({"type": "message", "partIndex": 2, "partKey": "2", "callId": "call_say",
            "text": TEXT, "level": "chat"})
    );
    let failed = parts(&page, "m5");
    assert_eq!(failed[0]["type"], "tool", "{page}");
    assert!(failed[0].get(occipital_app_reader::say::SAY_TEXT).is_none());
    assert_eq!(parts(&page, "m7"), Vec::<Value>::new(), "{page}");
    assert!(!reply
        .windows(occipital_app_reader::say::SAY_TEXT.len())
        .any(|window| window == occipital_app_reader::say::SAY_TEXT.as_bytes()));
    served.keep("01-newest-page", &newest, reply);

    let exact = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": SESSION, "messageOrdinal": 2}});
    let reply = served.read(2, &exact).await;
    let message = decoded(&reply)["result"]["messages"][0]["parts"][2].clone();
    assert_eq!(message["type"], "message", "{message}");
    assert_eq!(message["text"], TEXT);
    served.keep("02-exact-read", &exact, reply);

    let details = json!({"method": super::super::super::DETAILS,
        "params": {"session": SESSION, "harness": "broca", "messageTool": true}});
    let reply = served.read(3, &details).await;
    assert_eq!(decoded(&reply)["result"]["messageTool"], "say");
    served.keep("03-details", &details, reply);

    // The plan is read once for both page reads and the details.
    let plan_reads = route
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == "session.plan_view")
        .count();
    assert!(plan_reads <= 2, "{plan_reads}");

    let (other, _) = chat(false);
    served.serve_with(other);
    let reply = served.read(4, &newest).await;
    let page = decoded(&reply);
    let answer = parts(&page, "m2");
    assert_eq!(answer[2]["type"], "tool", "{page}");
    assert_eq!(answer[2]["tool"], "say");
    assert_eq!(parts(&page, "m7")[0]["type"], "tool", "{page}");
    served.keep("04-newest-page-without-say", &newest, reply);

    let reply = served.read(5, &details).await;
    assert!(decoded(&reply)["result"].get("messageTool").is_none());
    served.keep("05-details-without-say", &details, reply);

    served.check();
}
