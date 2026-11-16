//! A row that changes in place under one page version. Broca stores a step before
//! its tool calls run and each result as a later entry, so a call's row gains its
//! result when that entry lands, while the runner's history counter, and so the
//! page version, stays the same. Every app following the chat must be sent the
//! changed row, however many apps read it and in whatever order. Every reply here
//! goes through the module's own handlers over a fake Broca route, and the replies
//! are the sample set the apps pin, `phone-reader-in-place-v1`; `served_samples`
//! says how the set is checked and rewritten.

use super::served_samples::{decoded, Served};
use super::*;
use crate::app_reader::resolve::Entorhinal;
use crate::app_reader::{AppReader, TRANSCRIPT_PAGE};

struct AnySession;

#[async_trait]
impl Entorhinal for AnySession {
    async fn request(&self, _: &str, params: Value) -> Result<Value, ReadError> {
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref", "incarnation":"entorhinal-test", "ended_at_ms":null,
            "runner":"broca", "session":params["session"], "harness":"broca",
            "project_root":"/workspace/project", "network":"yes", "network_history":[]}}),
        )
    }
}

struct ReadRoutes(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for ReadRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// The step that asks for `ls`, stored before the call runs.
fn running() -> Vec<Value> {
    vec![
        json!({"mid":"m40", "ordinal":40, "message":{"role":"user", "content":[
            {"type":"text", "text":"What is in the worktree?"}]}}),
        json!({"mid":"m41", "ordinal":41, "message":{"role":"assistant", "content":[
            {"type":"text", "text":"Listing it."},
            {"type":"tool_call", "tool_call_id":"call_ls", "tool_name":"bash", "input":{"command":"ls"}}]}}),
    ]
}

/// Then the call's result and the answer land, under the same history counter.
fn answered() -> Vec<Value> {
    let mut entries = running();
    entries.push(
        json!({"mid":"m42", "ordinal":42, "message":{"role":"tool", "content":[
        {"type":"tool_result", "tool_call_id":"call_ls", "output":"README.md\nsrc\n"}]}}),
    );
    entries.push(
        json!({"mid":"m43", "ordinal":43, "message":{"role":"assistant", "content":[
        {"type":"text", "text":"It holds README.md and src."}]}}),
    );
    entries
}

fn page(limit: u64) -> Value {
    json!({"method": TRANSCRIPT_PAGE,
        "params": {"session":"alfonso:s1", "harness":"broca", "limit":limit}})
}

fn claim(limit: u64, page: &Value) -> Value {
    let messages = page["messages"].as_array().unwrap();
    json!({"method": TRANSCRIPT_PAGE,
        "params": {"session":"alfonso:s1", "harness":"broca", "limit":limit, "claim":{
            "heldVersion":page["version"],
            "heldFloorMessageId":messages.first().unwrap()["messageId"],
            "sinceMessageId":messages.last().unwrap()["messageId"]}}})
}

/// Serve `request` as sample `name`, and return its result.
async fn read(served: &mut Served, corr: u64, name: &str, request: Value) -> Value {
    let reply = served.read(corr, &request).await;
    let result = decoded(&reply)["result"].clone();
    served.keep(name, &request, reply);
    result
}

/// Two apps hold the page with the call running, at different page sizes. When the
/// result lands, the first app's claim records the new page; the second's must
/// still answer `diverged` from `m41` with the call's new row, not `matched` with
/// only the rows after it. An app that has the new rows then matches.
#[tokio::test]
async fn a_row_changed_in_place_reaches_every_app_holding_it() {
    let route = FakeRoute::new();
    route
        .head_extra
        .lock()
        .unwrap()
        .insert("first_ordinal".into(), json!(40));
    *route.entries.lock().unwrap() = running();
    let app = Arc::new(AppReader::new(Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(AnySession))),
        Arc::new(ReadRoutes(route.clone())),
    ))));
    let mut served = Served::new("phone-reader-in-place-v1", app);
    let first = read(&mut served, 1, "01-page-small", page(1)).await;
    let second = read(&mut served, 2, "02-page-large", page(4)).await;
    let call_row = |messages: &Value| {
        messages
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["messageId"] == "m41")
            .cloned()
            .unwrap()
    };
    let before = call_row(&second["messages"]);

    *route.entries.lock().unwrap() = answered();
    let fresh = read(&mut served, 3, "03-page-after-result", page(4)).await;
    let after = call_row(&fresh["messages"]);
    assert_eq!(
        fresh["version"], second["version"],
        "the version must not move"
    );
    assert_ne!(
        before, after,
        "the call's row must change when its result lands"
    );

    for (corr, name, held) in [
        (4, "04-claim-small", &first),
        (5, "05-claim-large", &second),
    ] {
        let reply = read(&mut served, corr, name, claim(4, held)).await;
        assert_eq!(reply["claim"], "diverged", "{name}: {reply}");
        let resync = reply["resync"]["messages"].as_array().unwrap();
        assert_eq!(resync[0]["messageId"], "m41", "{name}: {reply}");
        assert_eq!(resync[0], after, "{name}: {reply}");
    }
    let caught_up = read(&mut served, 6, "06-claim-caught-up", claim(4, &fresh)).await;
    assert_eq!(caught_up["claim"], "matched", "{caught_up}");
    assert_eq!(caught_up["newTurns"], json!([]), "{caught_up}");
    served.check();
}
