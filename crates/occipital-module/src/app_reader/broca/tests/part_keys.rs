//! Part identities on the apps' transcript rows: `partKey`, `partIndex` and
//! `callId`. Magic Context keys its drop markers and summary ranges by them, so
//! they must be the stored message's own values, the same on every surface that
//! serves a row, and the same after the module restarts. Every reply here goes
//! through the module's own handlers over a fake Broca route.

use super::*;
use crate::app_reader::tests::serve;
use crate::app_reader::{resolve::Entorhinal, AppReader};
use crate::app_reader::{TRANSCRIPT_PAGE, TRANSCRIPT_WATCH};
use occipital_app_reader::broca_entries::{without_part_identity, CALL_ID, PART_INDEX, PART_KEY};

const KEYS: &str = "../occipital-app-reader/test-vectors/phone-reader-broca-v3-keys";
const V1: &str = "../occipital-app-reader/test-vectors/phone-reader-broca-v1";

/// Resolves whichever session it is asked for, with no network history, so every
/// row without a write time reads `unknown`.
struct AnySession;

#[async_trait]
impl Entorhinal for AnySession {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        assert_eq!(method, "agent.session_for_runner");
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref", "incarnation":"entorhinal-test", "ended_at_ms":null,
            "runner":"broca", "session":params["session"], "harness":"broca",
            "project_root":"/workspace/project", "network":"yes", "network_history":[]}}),
        )
    }
}

/// Routes with no send path, as for a session this module only reads.
struct ReadRoutes(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for ReadRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// A fresh module reader over `route`: nothing cached from an earlier reader.
fn reader(route: &Arc<FakeRoute>) -> Arc<AppReader> {
    Arc::new(AppReader::new(Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(AnySession))),
        Arc::new(ReadRoutes(route.clone())),
    ))))
}

fn parse(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

/// The session behind the key samples. `m41` holds encrypted reasoning, readable
/// reasoning, text, a tool call with its result in the same message, more text and
/// a second call whose result is stored in `m42`. The encrypted reasoning is never
/// shown, and a page read folds the first result into its call (a full read shows
/// it as its own part), so on a page the shown parts' `partIndex` values
/// (1, 2, 3, 5, 6) differ from their row positions (0 to 4).
fn keys_route() -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    route
        .head_extra
        .lock()
        .unwrap()
        .insert("first_ordinal".into(), json!(40));
    *route.entries.lock().unwrap() = vec![
        json!({"mid":"m40", "ordinal":40, "message":{"role":"user", "content":[
            {"type":"text", "text":"What year is it, and what is in the worktree?"}]}}),
        json!({"mid":"m41", "ordinal":41, "message":{"role":"assistant", "content":[
            {"type":"redacted_reasoning", "data":"ZW5jcnlwdGVkIHJlYXNvbmluZw=="},
            {"type":"reasoning", "text":"The user wants the current year. Check the clock instead of guessing."},
            {"type":"text", "text":"Checking the clock."},
            {"type":"tool_call", "tool_call_id":"call_date_1", "tool_name":"bash", "input":{"command":"date -u +%Y"}},
            {"type":"tool_result", "tool_call_id":"call_date_1", "output":"2026\n"},
            {"type":"text", "text":"It is 2026. Listing the worktree next."},
            {"type":"tool_call", "tool_call_id":"call_ls_2", "tool_name":"bash", "input":{"command":"ls"}}]}}),
        json!({"mid":"m42", "ordinal":42, "message":{"role":"tool", "content":[
            {"type":"tool_result", "tool_call_id":"call_ls_2", "output":"README.md\nsrc\n"}]}}),
        json!({"mid":"m43", "ordinal":43, "message":{"role":"assistant", "content":[
            {"type":"text", "text":"It is 2026, and the worktree holds README.md and src."}]}}),
    ];
    route
}

fn keys_params() -> Value {
    json!({"session":"alfonso:s1", "harness":"broca", "limit":4})
}

/// A claim on the newest page that holds only `m40`, so the reply's `newTurns`
/// carry `m41` to `m43`.
fn behind(version: &Value) -> Value {
    let mut request = json!({"session":"alfonso:s1", "harness":"broca"});
    request["claim"] =
        json!({"heldFloorMessageId":"m40", "sinceMessageId":"m40", "heldVersion":version});
    request
}

/// Each sample, by file name: the request the phone sends and the bytes the
/// module answers.
async fn key_samples() -> Vec<(String, Vec<u8>)> {
    let route = keys_route();
    let app = reader(&route);
    let newest_request = keys_params();
    let newest = serve(&app, TRANSCRIPT_PAGE, &newest_request).await;
    let version = parse(&newest)["result"]["version"].clone();
    let mut claim_request = behind(&version);
    claim_request["limit"] = json!(4);
    let claim = serve(&app, TRANSCRIPT_PAGE, &claim_request).await;
    let watch_request = behind(&version);
    let watch = serve(&app, TRANSCRIPT_WATCH, &watch_request).await;
    let full_request = json!({"session":"alfonso:s1", "harness":"broca", "messageOrdinal":41});
    let full = serve(&app, TRANSCRIPT_PAGE, &full_request).await;
    let mut samples = Vec::new();
    for (name, request, reply, kind) in [
        ("01-page-newest", newest_request, newest, "reply"),
        ("02-claim-behind", claim_request, claim, "reply"),
        ("03-watch-behind", watch_request, watch, "notice-1"),
        ("04-message-full", full_request, full, "reply"),
    ] {
        samples.push((
            format!("{name}.request.json"),
            serde_json::to_vec(&request).unwrap(),
        ));
        samples.push((format!("{name}.{kind}.json"), reply));
    }
    samples
}

fn sums(samples: &[(String, Vec<u8>)]) -> String {
    let mut lines: Vec<_> = samples
        .iter()
        .map(|(name, bytes)| format!("{:x}  {name}\n", Sha256::digest(bytes)))
        .collect();
    lines.sort_by(|left, right| left[66..].cmp(&right[66..]));
    lines.concat()
}

/// The key samples must match the handlers even when a recording flag leaks
/// into the test environment; recording must not turn this check into a pass.
#[tokio::test]
async fn the_part_key_samples_are_what_the_handlers_answer() {
    assert!(
        std::env::var_os("THALAMUS_RECORD_KEY_SAMPLES").is_none(),
        "unset THALAMUS_RECORD_KEY_SAMPLES: correctness tests must compare the published samples"
    );
    let samples = key_samples().await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(KEYS);
    let mut recorded_samples = Vec::new();
    for (name, bytes) in &samples {
        let recorded = std::fs::read(root.join(name)).unwrap();
        let mut expected = parse(&recorded);
        add_recorded_completion(&mut expected);
        // May-not-send replies carry neither sender member, so handler paths name these samples.
        if [
            "01-page-newest.reply.json",
            "02-claim-behind.reply.json",
            "03-watch-behind.notice-1.json",
        ]
        .contains(&name.as_str())
        {
            add_recorded_last_run(&mut expected, &json!({"run_id":"r1", "state":"active"}));
        }
        assert_eq!(
            expected.to_string(),
            String::from_utf8_lossy(bytes),
            "{name}"
        );
        recorded_samples.push((name.clone(), recorded));
    }
    assert_eq!(
        std::fs::read_to_string(root.join("SHA256SUMS")).unwrap(),
        sums(&recorded_samples)
    );
    let mut present: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".json"))
        .collect();
    present.sort();
    let mut named: Vec<_> = samples.iter().map(|(name, _)| name.clone()).collect();
    named.sort();
    assert_eq!(present, named, "every sample file is recorded here");
}

/// The row for `mid` among a reply's `messages` or `newTurns`.
fn row<'a>(reply: &'a Value, mid: &str) -> &'a Value {
    let body = reply.get("result").unwrap_or(reply);
    body["messages"]
        .as_array()
        .or_else(|| body["newTurns"].as_array())
        .unwrap()
        .iter()
        .find(|row| row["messageId"] == mid)
        .unwrap_or_else(|| panic!("{mid} in {reply}"))
}

/// (`partKey`, `partIndex`, `callId`) of each part, `null` where absent.
fn identities(row: &Value) -> Vec<(Value, Value, Value)> {
    row["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| {
            (
                part[PART_KEY].clone(),
                part[PART_INDEX].clone(),
                part[CALL_ID].clone(),
            )
        })
        .collect()
}

#[tokio::test]
async fn page_claim_and_watch_serve_the_same_part_identities() {
    let samples: BTreeMap<_, _> = key_samples().await.into_iter().collect();
    let page = parse(&samples["01-page-newest.reply.json"]);
    let claim = parse(&samples["02-claim-behind.reply.json"]);
    let notice = parse(&samples["03-watch-behind.notice-1.json"]);
    let shown = identities(row(&page, "m41"));
    assert_eq!(
        shown,
        [
            (json!("1"), json!(1), Value::Null),
            (json!("2"), json!(2), Value::Null),
            (json!("3"), json!(3), json!("call_date_1")),
            (json!("5"), json!(5), Value::Null),
            (json!("6"), json!(6), json!("call_ls_2")),
        ]
    );
    for (surface, reply) in [("claim", &claim), ("watch", &notice)] {
        for mid in ["m41", "m42", "m43"] {
            assert_eq!(row(reply, mid), row(&page, mid), "{surface} {mid}");
        }
    }
    let calls: Vec<_> = row(&page, "m41")["toolCalls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|call| call[CALL_ID].clone())
        .collect();
    assert_eq!(calls, [json!("call_date_1"), json!("call_ls_2")]);
    // The full read shows the folded result as its own part, on its call's id.
    let full = parse(&samples["04-message-full.reply.json"]);
    assert_eq!(
        identities(row(&full, "m41")),
        [
            (json!("1"), json!(1), Value::Null),
            (json!("2"), json!(2), Value::Null),
            (json!("3"), json!(3), json!("call_date_1")),
            (json!("4"), json!(4), json!("call_date_1")),
            (json!("5"), json!(5), Value::Null),
            (json!("6"), json!(6), json!("call_ls_2")),
        ]
    );
}

#[tokio::test]
async fn a_message_keeps_its_part_identities_across_rereads_and_a_fresh_reader() {
    let first = reader(&keys_route());
    let before = parse(&serve(&first, TRANSCRIPT_PAGE, &keys_params()).await);
    let again = parse(&serve(&first, TRANSCRIPT_PAGE, &keys_params()).await);
    // A restarted module: a new reader and a new route over the same stored session.
    let restarted = reader(&keys_route());
    let after = parse(&serve(&restarted, TRANSCRIPT_PAGE, &keys_params()).await);
    for mid in ["m40", "m41", "m42", "m43"] {
        assert_eq!(row(&again, mid), row(&before, mid), "re-read {mid}");
        assert_eq!(row(&after, mid), row(&before, mid), "restart {mid}");
    }
    assert_eq!(identities(row(&after, "m41"))[3].1, json!(5));
}

/// The stored session behind the `phone-reader-broca-v1` recording, rebuilt from
/// the rows it served: each shown text and tool call becomes the stored block it
/// was decoded from, and each call's result, of the recorded `outputBytes`, is
/// stored in the tool message that follows it. `m0` and `m1` are older than both
/// recorded pages, so their content is never shown.
fn recorded_session() -> Vec<Value> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(V1);
    let page = |name: &str| parse(&std::fs::read(root.join(name)).unwrap());
    let mut rows = page("02-page-older.reply.json")["result"]["messages"]
        .as_array()
        .unwrap()
        .clone();
    rows.extend(
        page("01-page-newest.reply.json")["result"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .cloned(),
    );
    let mut entries = vec![
        json!({"mid":"m0", "ordinal":0, "message":{"role":"user", "content":[
            {"type":"text", "text":"Throwaway session for transcript-reader samples."}]}}),
        json!({"mid":"m1", "ordinal":1, "message":{"role":"assistant", "content":[
            {"type":"text", "text":"Starting."}]}}),
    ];
    entries.extend(stored_entries(rows));
    entries
}

fn stored_entries(rows: Vec<Value>) -> Vec<Value> {
    let mut entries = Vec::new();
    let mut results = Vec::new();
    for row in rows {
        let mid = row["messageId"].as_str().unwrap();
        let content: Vec<Value> = if row["role"] == "tool" {
            std::mem::take(&mut results)
        } else {
            row["parts"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(at, part)| match part["type"].as_str().unwrap() {
                    "text" => json!({"type":"text", "text":part["text"]}),
                    "tool" => {
                        let id = format!("call_{mid}_{at}");
                        let bytes = part["outputBytes"].as_u64().unwrap() as usize;
                        results.push(json!({"type":"tool_result", "tool_call_id":id,
                            "output":"r".repeat(bytes)}));
                        let input: Value =
                            serde_json::from_str(part["arguments"].as_str().unwrap()).unwrap();
                        json!({"type":"tool_call", "tool_call_id":id, "tool_name":part["tool"], "input":input})
                    }
                    other => panic!("v1 rows hold no {other} part"),
                })
                .collect()
        };
        let mut entry = json!({"mid":mid, "ordinal":row["ordinal"],
            "message":{"role":row["role"], "content":content}});
        // The phone recording preserves Broca's written_at as createdAtMs.
        if let Some(at) = row.get("createdAtMs") {
            entry["written_at"] = at.clone();
        }
        entries.push(entry);
    }
    assert!(results.is_empty(), "every recorded call has its result");
    entries
}

/// The recorded session's head: history counter 0 on the recorded lineage, idle
/// since the recorded activity time, and no attachments.
fn recorded_route() -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = recorded_session();
    route.history.store(0, Ordering::Relaxed);
    *route.lineage.lock().unwrap() = "b1e99ee9-edce-4f82-9b7d-0f8d60beb737".into();
    route.updated_at.store(1_792_756_846_572, Ordering::Relaxed);
    *route.run_state.lock().unwrap() = json!({"run_id":"r1", "state":"completed"});
    let mut head = route.head_extra.lock().unwrap();
    head.insert("first_ordinal".into(), json!(0));
    head.insert("input".into(), json!({"attachments":"no"}));
    drop(head);
    route
}

#[tokio::test]
async fn recorded_assistant_write_time_is_its_completion_time_on_the_page() {
    let recorded = parse(include_bytes!(
        "../../../../../occipital-app-reader/test-vectors/phone-reader-broca-sends-v1/11-older-page.json"
    ));
    // The last call's result is outside the recorded page. Replay the complete
    // call/result pairs and the user steer before that call.
    let rows = recorded["result"]["messages"].as_array().unwrap()[..5].to_vec();
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = stored_entries(rows.clone());
    route
        .head_extra
        .lock()
        .unwrap()
        .insert("first_ordinal".into(), json!(8));
    let reply = parse(
        &serve(
            &reader(&route),
            TRANSCRIPT_PAGE,
            &json!({"session":"alfonso:s1", "harness":"broca", "limit":5}),
        )
        .await,
    );
    assert_eq!(reply["result"]["messages"].as_array().unwrap().len(), 5);
    let assistant = row(&reply, "m10");
    assert_eq!(assistant["completedAtMs"], 1_792_938_364_557_i64);
    assert_eq!(assistant["createdAtMs"], 1_792_938_364_557_i64);
    for recorded in rows {
        let served = row(&reply, recorded["messageId"].as_str().unwrap());
        assert_eq!(served["createdAtMs"], recorded["createdAtMs"]);
        assert!(served.get("completion").is_none());
        if served["role"] == "assistant" {
            assert_eq!(served["completedAtMs"], recorded["createdAtMs"]);
        } else {
            assert!(served.get("completedAtMs").is_none());
        }
    }
    assert_eq!(row(&reply, "m12")["role"], "user");
    assert_eq!(row(&reply, "m9")["role"], "tool");
}

#[tokio::test]
async fn recorded_assistants_without_write_time_have_unknown_completion_on_the_page() {
    let route = recorded_route();
    let reply = parse(
        &serve(
            &reader(&route),
            TRANSCRIPT_PAGE,
            &json!({"session":"alfonso:s1", "harness":"broca", "limit":12}),
        )
        .await,
    );
    let rows = reply["result"]["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 10);
    assert_eq!(row(&reply, "m4")["completion"], "unknown");
    for row in rows {
        assert!(row.get("createdAtMs").is_none());
        assert!(row.get("completedAtMs").is_none());
        if row["role"] == "assistant" {
            assert_eq!(row["completion"], "unknown");
        } else {
            assert!(row.get("completion").is_none());
        }
    }
    assert_eq!(row(&reply, "m0")["role"], "user");
    assert_eq!(row(&reply, "m5")["role"], "tool");
}

// The published snapshots predate completion metadata. Their timed rows were
// generated from written_at; untimed assistants now carry unknown completion.
// Enrich only the test expectation, leaving the published bytes and hashes intact.
pub(super) fn add_recorded_completion(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if object.get("role").and_then(Value::as_str) == Some("assistant") {
                if let Some(at) = object.get("createdAtMs").cloned() {
                    object.insert("completedAtMs".into(), at);
                } else {
                    object.insert("completion".into(), json!("unknown"));
                }
            }
            for value in object.values_mut() {
                add_recorded_completion(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                add_recorded_completion(value);
            }
        }
        _ => {}
    }
}

/// The recordings predate this optional session fact; only the expectation is enriched.
pub(super) fn add_recorded_last_run(value: &mut Value, run: &Value) {
    let run = serde_json::from_value(run.clone()).unwrap();
    let body = if value.get("result").is_some() {
        &mut value["result"]
    } else {
        value
    };
    body["lastRun"] = occipital_app_reader::source::last_run_value(&run);
}

/// Replaying the v1 recording's requests through today's handlers gives the
/// recorded bytes once part identities are removed and completion metadata is
/// added to the expectation. No existing member changed its value or order, except
/// `activityToken`, a digest of the live head that the rebuilt head can't
/// reproduce, so the recorded token is put back before comparing.
#[tokio::test]
async fn without_part_identities_the_v1_replies_are_the_recorded_bytes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(V1);
    let app = reader(&recorded_route());
    let mut carried = 0;
    for (name, method, kind) in [
        ("01-page-newest", TRANSCRIPT_PAGE, "reply"),
        ("02-page-older", TRANSCRIPT_PAGE, "reply"),
        ("03-claim-current", TRANSCRIPT_PAGE, "reply"),
        ("04-claim-behind", TRANSCRIPT_PAGE, "reply"),
        ("05-watch-current", TRANSCRIPT_WATCH, "notice-1"),
        ("06-watch-behind", TRANSCRIPT_WATCH, "notice-1"),
    ] {
        let request = parse(&std::fs::read(root.join(format!("{name}.request.json"))).unwrap());
        let recorded = std::fs::read(root.join(format!("{name}.{kind}.json"))).unwrap();
        let mut reply = parse(&serve(&app, method, &request).await);
        let text = reply.to_string();
        carried += text.matches(PART_KEY).count();
        if let Some(token) = reply
            .get_mut("result")
            .and_then(|body| body.get_mut("activityToken"))
        {
            assert!(token.as_str().unwrap().starts_with("broca-activity-"));
            *token = parse(&recorded)["result"]["activityToken"].clone();
        }
        without_part_identity(&mut reply);
        let mut expected = parse(&recorded);
        add_recorded_completion(&mut expected);
        // May-not-send replies carry neither sender member; the older-page path is excluded.
        if [
            "01-page-newest",
            "03-claim-current",
            "04-claim-behind",
            "05-watch-current",
            "06-watch-behind",
        ]
        .contains(&name)
        {
            add_recorded_last_run(&mut expected, &json!({"run_id":"r1", "state":"completed"}));
        }
        assert_eq!(
            String::from_utf8_lossy(&serde_json::to_vec(&reply).unwrap()),
            expected.to_string(),
            "{name}"
        );
    }
    // The replies did carry the members before they were removed.
    assert!(carried > 0);
}
