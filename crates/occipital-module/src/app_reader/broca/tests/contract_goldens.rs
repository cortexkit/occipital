//! Broca's recorded runner replies, decoded through the route and phone handlers.
//! They were recorded under runner contract 0.7 and are read as a runner on the
//! current contract serves them (`current_contract`).

use super::*;
use crate::app_reader::{resolve::Entorhinal, AppReader, DETAILS, TRANSCRIPT_PAGE};
use std::path::{Path, PathBuf};

const GOLDENS: &str = "tests/data/broca-goldens-0.6.4-head";
const LOG_ID: &str = "00000000-0000-4000-8000-000000000064";
const FILES: &[&str] = &[
    "role_describe.head_additions.json",
    "session_head.before_head_additions.full.json",
    "session_head.before_head_additions.json",
    "session_head.named.json",
    "session_head.owner.json",
    "session_head.reasoning_usage.json",
    "session_head.starter.json",
    "session_head.ungranted.json",
    "session_read.before_head_additions.json",
    "session_read.written_at.json",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDENS)
}

#[test]
fn broca_golden_checksums_match_the_supplied_bytes() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    let mut listed = Vec::new();
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        assert!(FILES.contains(&name), "unexpected checksum entry: {name}");
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
        listed.push(name);
    }
    listed.sort_unstable();
    let mut expected = FILES.to_vec();
    expected.sort_unstable();
    assert_eq!(listed, expected, "every golden has one checksum");
    let mut on_disk: Vec<_> = std::fs::read_dir(root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".json"))
        .collect();
    on_disk.sort();
    assert_eq!(on_disk, expected, "every JSON file is tested");
}

fn golden(name: &str) -> Value {
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap();
    // Broca redacts unstable values with these exact placeholders. Only replace
    // existing values; never supply a member that the recorded reply omitted.
    fn substitute(value: &mut Value) {
        match value {
            Value::String(text) => match text.as_str() {
                "@LOG_ID@" => *value = json!(LOG_ID),
                "@UPDATED_AT@" => *value = json!(506),
                "@RUN_ID@" => *value = json!("broca-golden-run"),
                _ => {}
            },
            Value::Array(values) => values.iter_mut().for_each(substitute),
            Value::Object(values) => values.values_mut().for_each(substitute),
            _ => {}
        }
    }
    substitute(&mut value);
    current_contract(value)
}

/// The recorded 0.7 samples named `name`, as a runner on the current contract
/// serves them.
fn samples(name: &str) -> Vec<(String, Value)> {
    samples_0_7::samples(name)
        .into_iter()
        .map(|(name, value)| (name, current_contract(value)))
        .collect()
}

struct GoldenRoute {
    head: Mutex<Value>,
    read: Value,
    describe: Value,
    display_history: Option<Value>,
    calls: Mutex<Vec<String>>,
}

struct QuietEvents;

#[async_trait]
impl EventStream for QuietEvents {
    async fn next(&mut self) -> Result<Option<Value>, ReadError> {
        std::future::pending().await
    }
}

#[async_trait]
impl BrocaRoute for GoldenRoute {
    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        let request: Value = serde_json::from_slice(&body(op, params)).unwrap();
        let method = request["method"].as_str().unwrap();
        self.calls.lock().unwrap().push(method.into());
        let reply = match method {
            "session.head" => self.head.lock().unwrap().clone(),
            "session.read" => self.read.clone(),
            "session.history" => match &self.display_history {
                Some(history) if request["params"]["form"] == "display" => history.clone(),
                _ => raw_history(&result(self.read.clone())?, &request["params"])?,
            },
            "role.describe" => self.describe.clone(),
            _ => panic!("golden read unexpectedly called {method}"),
        };
        // Match DaemonRoute::read's JSON decoding and result-envelope handling.
        result(serde_json::from_slice(&serde_json::to_vec(&reply).unwrap()).unwrap())
    }

    async fn subscribe(&self, _live_ids: bool) -> Result<Box<dyn EventStream>, ReadError> {
        Ok(Box::new(QuietEvents))
    }
}

struct GoldenRoutes(Arc<GoldenRoute>);
struct UnusedSender(SessionRef);

#[async_trait]
impl SendTarget for UnusedSender {
    fn reference(&self) -> &SessionRef {
        &self.0
    }
    async fn call(&self, method: &'static str, _: Value) -> Result<Value, RunnerError> {
        panic!("a golden read unexpectedly sent {method}")
    }
}

#[async_trait]
impl BrocaRoutes for GoldenRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
    fn sender(&self, resolved: &Resolved) -> Option<Arc<dyn SendTarget>> {
        Some(Arc::new(UnusedSender(resolved.reference.clone())))
    }
}

struct GoldenLookup;

#[async_trait]
impl Entorhinal for GoldenLookup {
    async fn request(&self, _: &str, _: Value) -> Result<Value, ReadError> {
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref", "incarnation":"entorhinal-test", "ended_at_ms":null,
            "runner":"broca", "session":"alfonso:s1", "harness":"broca",
            "project_root":"/workspace/project", "network":"no", "network_history":[
                {"network":"yes", "since_ms":500}, {"network":"no", "since_ms":505}]}}),
        )
    }
}

fn app(head: &str, read: &str) -> (Arc<GoldenRoute>, AppReader) {
    app_with_values(
        golden(head),
        golden(read),
        golden("role_describe.head_additions.json"),
    )
}

fn app_with_values(head: Value, read: Value, describe: Value) -> (Arc<GoldenRoute>, AppReader) {
    app_with_history(head, read, describe, None)
}

fn app_with_history(
    head: Value,
    read: Value,
    describe: Value,
    display_history: Option<Value>,
) -> (Arc<GoldenRoute>, AppReader) {
    let route = Arc::new(GoldenRoute {
        head: Mutex::new(head),
        read,
        describe,
        display_history,
        calls: Mutex::new(Vec::new()),
    });
    let app = AppReader::new(Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(GoldenLookup))),
        Arc::new(GoldenRoutes(route.clone())),
    )));
    (route, app)
}

async fn page(app: &AppReader) -> Value {
    app.read(TRANSCRIPT_PAGE, params()).await.unwrap()
}

async fn details(app: &AppReader) -> Value {
    app.read(DETAILS, json!({"session":"alfonso:s1", "harness":"broca"}))
        .await
        .unwrap()
}

fn claim(page: &Value) -> Value {
    let mut request = params();
    request["claim"] = json!({"heldFloorMessageId":"m0", "sinceMessageId":"m1",
        "heldVersion":page["version"]});
    request
}

fn normal_usage() -> Value {
    json!({"since":"r1", "steps":1, "unreported_steps":0, "input_tokens":7,
        "output_tokens":3, "by_model":[{"provider":"p", "model":"m", "steps":1,
            "unreported_steps":0, "input_tokens":7, "output_tokens":3}]})
}

fn reasoning_usage() -> Value {
    json!({"since":"broca-golden-run", "steps":2, "unreported_steps":0, "input_tokens":16,
        "output_tokens":22, "reasoning_tokens":7, "by_model":[{"provider":"fake", "model":"fake",
            "steps":2, "unreported_steps":0, "input_tokens":16, "output_tokens":22,
            "reasoning_tokens":7}]})
}

fn assert_sender(reply: &Value, granted: bool, modes: &[&str], interrupt: bool) {
    assert!(reply.get("senderUnknown").is_none(), "{reply}");
    if granted {
        assert!(reply["sender"].is_object(), "{reply}");
        assert_eq!(reply["sender"]["modes"], json!(modes));
        assert_eq!(reply["sender"]["interrupt"], interrupt);
    } else {
        assert!(reply.get("sender").is_none(), "{reply}");
    }
}

async fn assert_head(name: &str, model: Value, usage: Value, modes: &[&str], interrupt: bool) {
    let (_, app) = app(name, "session_read.written_at.json");
    let newest = page(&app).await;
    assert_eq!(newest["model"], model, "{name}: page model");
    assert_eq!(newest["historyVersion"], 0);
    assert_eq!(newest["version"], format!("history:0:incarnation:{LOG_ID}"));
    assert_eq!(newest["acceptsAttachments"], false);
    assert_sender(&newest, !modes.is_empty(), modes, interrupt);

    let matched = app.read(TRANSCRIPT_PAGE, claim(&newest)).await.unwrap();
    assert_eq!(matched["claim"], "matched", "{name}: {matched}");
    assert_eq!(matched["model"], model, "{name}: claim model");
    assert_eq!(matched["version"], newest["version"]);
    assert_sender(&matched, !modes.is_empty(), modes, interrupt);

    let facts = details(&app).await;
    assert_eq!(facts["model"], model, "{name}: details model");
    assert_eq!(facts["usage"], usage, "{name}: details usage");
    assert_eq!(facts["incarnation"], LOG_ID);
    assert_sender(&facts, !modes.is_empty(), modes, interrupt);
}

#[tokio::test]
async fn owner_head_reaches_pages_claims_and_details() {
    assert_head(
        "session_head.owner.json",
        json!({"provider":"p", "model":"m"}),
        normal_usage(),
        &["tool_result", "steer", "idle_start"],
        true,
    )
    .await;
}

#[tokio::test]
async fn named_head_reaches_pages_claims_and_details() {
    assert_head(
        "session_head.named.json",
        json!({"provider":"p", "model":"m"}),
        normal_usage(),
        &["tool_result", "steer"],
        false,
    )
    .await;
}

#[tokio::test]
async fn starter_head_reaches_pages_claims_and_details() {
    assert_head(
        "session_head.starter.json",
        json!({"provider":"p", "model":"m"}),
        normal_usage(),
        &["tool_result", "steer", "idle_start"],
        true,
    )
    .await;
}

#[tokio::test]
async fn ungranted_head_hides_the_message_bar_on_pages_claims_and_details() {
    assert_head(
        "session_head.ungranted.json",
        json!({"provider":"p", "model":"m"}),
        normal_usage(),
        &[],
        false,
    )
    .await;
}

#[tokio::test]
async fn reasoning_usage_is_a_breakdown_not_extra_output_tokens() {
    assert_head(
        "session_head.reasoning_usage.json",
        json!({"provider":"fake", "model":"fake"}),
        reasoning_usage(),
        &["tool_result", "steer", "idle_start"],
        true,
    )
    .await;
}

#[tokio::test]
async fn role_describe_head_sender_group_makes_an_absent_grant_known() {
    let (route, app) = app(
        "session_head.ungranted.json",
        "session_read.written_at.json",
    );
    assert_sender(&page(&app).await, false, &[], false);
    assert_sender(&details(&app).await, false, &[], false);
    assert_eq!(
        route
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|method| method.as_str() == "role.describe")
            .count(),
        1
    );
}

fn assert_no_source_facts(reply: &Value) {
    for field in [
        "activity",
        "activityAtMs",
        "model",
        "usage",
        "sender",
        "senderUnknown",
        "acceptsAttachments",
        "acceptedModalities",
        "acceptedMimeTypes",
    ] {
        assert!(reply.get(field).is_none(), "invented {field}: {reply}");
    }
}

/// A never-written session's head: only the history counter and the log's
/// identity, which every head carries.
fn never_written_head() -> Value {
    json!({"result":{"history_version":0, "incarnation":LOG_ID}})
}

#[tokio::test]
async fn never_written_goldens_map_empty_page_details_and_stale_caller_claim_watch() {
    let (_, app) = app_with_values(
        never_written_head(),
        golden("session_read.before_head_additions.json"),
        golden("role_describe.head_additions.json"),
    );
    let app = Arc::new(app);
    let newest = page(&app).await;
    assert_eq!(newest["messages"], json!([]));
    assert_eq!(newest["reachedStart"], true);
    assert_eq!(newest["version"], format!("history:0:incarnation:{LOG_ID}"));
    assert!(newest["activityToken"].is_string());
    assert_no_source_facts(&newest);
    assert_eq!(details(&app).await, json!({"incarnation":LOG_ID}));

    // An empty transcript supplies no claim anchors. These ids belong to a
    // stale caller cache, not to messages fabricated for the empty session.
    let mut stale = params();
    stale["claim"] = json!({"heldFloorMessageId":"stale-floor", "sinceMessageId":"stale-since",
        "heldVersion":newest["version"]});
    let direct = app.read(TRANSCRIPT_PAGE, stale.clone()).await.unwrap();
    let notice: Value = serde_json::from_slice(
        &crate::app_reader::tests::serve(&app, crate::app_reader::TRANSCRIPT_WATCH, &stale).await,
    )
    .unwrap();
    assert_eq!(notice["answers"], stale["claim"]);
    for reply in [direct, notice] {
        assert_eq!(reply["claim"], "cursor_expired", "{reply}");
        assert_eq!(reply["version"], newest["version"]);
        assert_eq!(reply["resync"]["messages"], json!([]));
        assert_eq!(reply["resync"]["reachedStart"], true);
        assert_no_source_facts(&reply);
        assert_no_source_facts(&reply["resync"]);
        assert!(reply.get("vouchedFromMessageId").is_none(), "{reply}");
        assert!(reply.get("validThroughMessageId").is_none(), "{reply}");
        assert!(reply.get("newTurns").is_none(), "{reply}");
    }
}

/// A written head missing a member the contract requires, or with ordinals out
/// of bounds, is malformed, never an empty session.
#[tokio::test]
async fn partial_heads_are_malformed_not_empty_sessions() {
    let owner = golden("session_head.owner.json");
    let mut cases = Vec::new();
    for field in [
        "updated_at",
        "first_ordinal",
        "history_version",
        "incarnation",
    ] {
        let mut head = owner.clone();
        head["result"].as_object_mut().unwrap().remove(field);
        cases.push((format!("without {field}"), head));
    }
    let mut no_cut = owner.clone();
    no_cut["result"]["first_ordinal"] = json!(2);
    cases.push(("first_ordinal above 1 without a cut".into(), no_cut));
    let mut out_of_bounds = owner;
    out_of_bounds["result"]["first_ordinal"] = json!(4);
    cases.push((
        "first_ordinal beyond last_ordinal + 1".into(),
        out_of_bounds,
    ));
    for (name, head) in cases {
        let (_, app) = app_with_values(
            head,
            golden("session_read.written_at.json"),
            golden("role_describe.head_additions.json"),
        );
        let error = app.read(TRANSCRIPT_PAGE, params()).await.unwrap_err();
        assert_eq!(error.code, "invalid_provider_reply", "{name}");
    }
}

#[tokio::test]
async fn written_at_reaches_phone_rows_and_selects_network_at_each_write() {
    let (_, app) = app("session_head.owner.json", "session_read.written_at.json");
    let reply = page(&app).await;
    let rows = reply["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for (mid, at, network, role, text) in [
        ("m0", 500, "yes", "user", "hello"),
        ("m1", 505, "no", "assistant", "answer"),
    ] {
        let row = rows.iter().find(|row| row["messageId"] == mid).unwrap();
        assert_eq!(row["createdAtMs"], at, "{row}");
        assert_eq!(row["network"], network, "{row}");
        assert_eq!(row["role"], role);
        assert_eq!(row["text"], text);
    }
}

#[tokio::test]
async fn entries_without_written_at_do_not_invent_a_phone_time_or_network() {
    let mut read = golden("session_read.written_at.json");
    for entry in read["result"]["messages"].as_array_mut().unwrap() {
        entry.as_object_mut().unwrap().remove("written_at");
    }
    let (_, app) = app_with_values(
        golden("session_head.owner.json"),
        read,
        golden("role_describe.head_additions.json"),
    );
    let reply = page(&app).await;
    let rows = reply["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert!(row.get("createdAtMs").is_none(), "{row}");
        assert_eq!(row["network"], "unknown", "{row}");
    }
}

#[tokio::test]
async fn an_omitted_sender_group_leaves_the_grant_unknown() {
    let mut head = golden("session_head.owner.json");
    head["result"].as_object_mut().unwrap().remove("sender");
    let mut describe = golden("role_describe.head_additions.json");
    describe["result"]["capabilities"]
        .as_array_mut()
        .unwrap()
        .retain(|group| group != "head_sender");
    let (_, app) = app_with_values(head, golden("session_read.written_at.json"), describe);
    let newest = page(&app).await;
    let matched = app.read(TRANSCRIPT_PAGE, claim(&newest)).await.unwrap();
    assert_eq!(matched["claim"], "matched");
    assert_eq!(matched["version"], newest["version"]);
    let facts = details(&app).await;
    for reply in [newest, matched, facts] {
        assert_eq!(reply["senderUnknown"], true, "{reply}");
        assert!(reply.get("sender").is_none(), "{reply}");
    }
}

#[tokio::test]
async fn populated_head_from_before_the_head_additions_reads_with_every_new_fact_unknown() {
    // Broca's head and describe as they were served before the head additions: two
    // messages and a finished run, with no model, usage, sender or incarnation, and
    // no `session_usage` or `head_sender` group. The reader must still serve it and
    // leave every new fact absent or unknown rather than inventing one.
    let mut read = golden("session_read.written_at.json");
    for entry in read["result"]["messages"].as_array_mut().unwrap() {
        entry.as_object_mut().unwrap().remove("written_at");
    }
    let mut describe = golden("role_describe.head_additions.json");
    describe["result"]["capabilities"]
        .as_array_mut()
        .unwrap()
        .retain(|group| group != "head_sender" && group != "session_usage");
    let (_, app) = app_with_values(
        golden("session_head.before_head_additions.full.json"),
        read,
        describe,
    );
    let newest = page(&app).await;
    assert_eq!(newest["messages"].as_array().unwrap().len(), 2, "{newest}");
    assert_eq!(newest["historyVersion"], 0);
    assert_eq!(newest["version"], format!("history:0:incarnation:{LOG_ID}"));
    let matched = app.read(TRANSCRIPT_PAGE, claim(&newest)).await.unwrap();
    assert_eq!(matched["claim"], "matched", "{matched}");
    assert_eq!(matched["version"], newest["version"]);
    let facts = details(&app).await;
    assert!(facts.get("usage").is_none(), "{facts}");
    for reply in [newest, matched, facts] {
        assert!(reply.get("model").is_none(), "{reply}");
        assert_eq!(reply["senderUnknown"], true, "{reply}");
        assert!(reply.get("sender").is_none(), "{reply}");
    }
}

// Head-only captures have no matching raw-read capture. Supply a small raw page
// with the same log and counters rather than joining independent sessions. Its
// messages are `m0` and up, as the recorded sessions name them.
fn read_for_head(head: &Value) -> Value {
    let meta = &head["result"];
    let messages: Vec<_> = (1..=meta["last_ordinal"].as_u64().unwrap())
        .map(|ordinal| {
            json!({
                "mid":format!("m{}", ordinal - 1), "ordinal":ordinal, "written_at":500 + ordinal,
                "message":{"role":"user", "content":[{"type":"text","text":"raw prompt"}]}
            })
        })
        .collect();
    json!({"result":{"incarnation":meta["incarnation"], "history_version":meta["history_version"],
        "messages":messages, "head":{"wal_seq":2,"sub_index":0}}})
}

#[tokio::test]
async fn broca_0_7_heads_reach_last_run_on_pages_claims_and_details() {
    for file in [
        "session.head.completed.json",
        "session.head.error.fractional.json",
        "session.head.error.missing-delay.json",
    ] {
        for (name, head) in samples(file) {
            let read = read_for_head(&head);
            let (_, app) =
                app_with_values(head.clone(), read, json!({"result":describe_with(&[])}));
            let newest = page(&app).await;
            let mut request = params();
            let last = head["result"]["last_ordinal"].as_u64().unwrap();
            request["claim"] = json!({"heldFloorMessageId":newest["messages"][0]["messageId"], "sinceMessageId":format!("m{}", last - 1), "heldVersion":newest["version"]});
            let matched = app.read(TRANSCRIPT_PAGE, request).await.unwrap();
            assert_eq!(matched["claim"], "matched", "{name}: {matched}");
            for reply in [newest, matched, details(&app).await] {
                let last = &reply["lastRun"];
                assert_eq!(
                    last["runId"], head["result"]["last_run_state"]["run_id"],
                    "{name}: {reply}"
                );
                assert_eq!(
                    last["state"],
                    if file.contains("completed") {
                        "completed"
                    } else {
                        "error"
                    },
                    "{name}"
                );
                assert_eq!(last["reason"], last["state"], "{name}");
                if file.contains("completed") {
                    assert!(last.get("error").is_none(), "{name}");
                } else {
                    assert_eq!(last["error"]["class"], "transient", "{name}");
                    assert_eq!(last["error"]["resetsAtMs"], 1790000003000_u64, "{name}");
                    if file.contains("fractional") {
                        assert_eq!(last["error"]["retryAfterSecs"], 1.5, "{name}");
                    } else {
                        assert!(
                            last["error"].get("retryAfterSecs").is_none(),
                            "{name}: absent must not become zero or null"
                        );
                    }
                }
                assert_sender(&reply, true, &["tool_result", "steer", "idle_start"], true);
            }
        }
    }
}

#[tokio::test]
async fn broca_0_7_display_history_decodes_on_the_client_message_id_echo_path() {
    for file in ["session.history.json", "approved.session.history.json"] {
        for (name, history) in samples(file) {
            let decoded: DisplayPage = serde_json::from_value(result(history.clone()).unwrap())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let projected = serde_json::to_value(decoded).unwrap();
            let part = &projected["messages"][1]["row"]["parts"][0];
            for field in ["call_key", "schema_pin"] {
                if let Some(value) = part.get(field) {
                    assert_eq!(
                        value, &history["result"]["messages"][1]["row"]["parts"][0][field],
                        "{name}: {field}"
                    );
                }
            }
            let head = json!({"result":{"incarnation":history["result"]["incarnation"], "first_ordinal":1,
                "last_ordinal":2, "history_version":0, "updated_at":506, "last_run_state":{"run_id":"run","state":"completed"}}});
            let mut raw = read_for_head(&head);
            raw["result"]["messages"]
                .as_array_mut()
                .unwrap()
                .truncate(2);
            let (route, app) = app_with_history(
                head.clone(),
                raw.clone(),
                json!({"result":describe_with(&["history_display"])}),
                Some(history.clone()),
            );
            let reply = page(&app).await;
            assert!(
                route
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|op| op == "session.history"),
                "{name}: echo path was not reached"
            );
            assert_eq!(reply["messages"][0]["text"], "raw prompt", "{name}");
            assert!(
                reply["messages"][0].get("clientMessageId").is_none(),
                "{name}"
            );
            // Captures have no phone receipt. Adding one to the user row proves
            // that decode failures cannot pass by silently skipping the echo.
            let mut with_echo = history;
            with_echo["result"]["messages"][0]["row"]["client_message_id"] = json!("phone-fixture");
            let (_, app) = app_with_history(
                head,
                raw,
                json!({"result":describe_with(&["history_display"])}),
                Some(with_echo),
            );
            let reply = page(&app).await;
            assert_eq!(
                reply["messages"][0]["clientMessageId"], "phone-fixture",
                "{name}"
            );
            assert!(
                reply["messages"][1].get("clientMessageId").is_none(),
                "{name}"
            );
        }
    }
}

#[tokio::test]
async fn broca_0_7_approved_read_reaches_source_windows_and_run_state() {
    for (name, read) in samples("approved.session.read.json") {
        let head = json!({"result":{"incarnation":read["result"]["incarnation"], "first_ordinal":1,
            "last_ordinal":4, "history_version":0, "updated_at":506,
            "last_run_state":{"run_id":read["result"]["lineage_state"]["last_run_id"],"state":"completed","reason":"completed"}}});
        let (route, app) = app_with_values(
            head.clone(),
            read.clone(),
            json!({"result":describe_with(&[])}),
        );
        let mut request = params();
        request["limit"] = json!(4);
        let reply = app.read(TRANSCRIPT_PAGE, request).await.unwrap();
        assert_eq!(reply["messages"].as_array().unwrap().len(), 4, "{name}");
        assert_eq!(reply["lastRun"]["state"], "completed", "{name}");
        assert!(reply["lastRun"].get("error").is_none(), "{name}");
        assert!(
            reply["messages"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r.get("unreadable").is_none()),
            "{name}"
        );
        assert!(
            route
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|op| op == "session.read"),
            "{name}"
        );
        let source = BrocaSource::new(resolved(), route);
        let full = source.exact(&Anchor::Ordinal(2)).await.unwrap().unwrap();
        assert_eq!(full.mid, "m1", "{name}");
        assert_eq!(
            full.display["parts"][0]["callId"], "approved-call",
            "{name}"
        );
        assert_eq!(full.display["parts"][0]["isError"], false, "{name}");
    }
}
