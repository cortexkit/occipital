//! An OpenCode 2 head's runner replies, recorded from a real OpenCode 2.2.3
//! server. Thalamus is the runner module that hosts OpenCode heads (module id
//! `thalamus-harness`, harness `opencode2`, runner contract llm-runner 0.8.2),
//! and it recorded these replies. The set is copied byte for byte into
//! `tests/data` from the thalamus repository's tag `runner-pages-opencode2-v1`
//! (object `fd0cf027d001…`, commit `f5b4a87f9687…`), with its README, which
//! says what each file is and stays as published so the checksums hold. The
//! session ran two turns: one with a `bash` call, then a plain reply, each sent
//! with an app's `client_message_id`.

use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SET: &str = "runner-pages-opencode2-v1";
/// The digest of the set's `SHA256SUMS` file as published with the tag.
const SUMS_SHA256: &str = "a3673922b3d5f5a195c0d42c5ce531d908d8f71d007aded64d9b0f11943832bf";
const INCARNATION: &str = "inc_45b86dec0f8d4c2599e3c11c52a8f445";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(SET)
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

fn opencode2_binding() -> Resolved {
    let mut binding = resolved();
    binding.reference.runner = OPENCODE_RUNNER_MODULE_ID.into();
    binding.harness = "opencode2".into();
    binding
}

#[test]
fn the_opencode2_pages_are_the_published_set() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&sums)), SUMS_SHA256);
    let sums = String::from_utf8(sums).unwrap();
    assert_eq!(sums.lines().count(), 10, "nine replies and the README");
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The recorded head passes the contract's head rule, and with the recorded
/// description it grants the app steering and idle starts.
#[test]
fn the_recorded_head_and_description_grant_the_app_sends() {
    let head = head_meta(&sample("03-session-head.json")).unwrap();
    let describe = check_describe(&sample("09-role-describe.json")).unwrap();
    assert!(describe.declares("read_display_facts"));
    match send_grant(&head, Some(&describe), false) {
        SendGrant::Granted(sender) => {
            assert_eq!(
                sender["modes"],
                json!(["steer", "idle_start"]),
                "{sender:?}"
            )
        }
        other => panic!("{other:?}"),
    }
}

/// Occipital's rows for the recorded read follow the runner's own display rows
/// for the same entries: the same messages, roles and text, and the `bash` call
/// is one app tool part keyed by the runner's part key, with its result folded
/// in and its times from the read's display facts.
#[test]
fn opencode2_rows_follow_the_runners_display_rows() {
    let source = BrocaSource::new(opencode2_binding(), FakeRoute::new());
    assert_eq!(source.harness(), "opencode2");
    let read = sample("02-session-read-display-facts.json");
    let rows = source
        .rows(read["messages"].as_array().unwrap(), false, &[])
        .unwrap();
    let display = sample("05-session-history-display.json");
    let expected = display["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows.len(), expected.len());
    for (row, want) in rows.iter().zip(expected) {
        assert_eq!(row.mid, want["mid"].as_str().unwrap());
        assert_eq!(row.ordinal, want["ordinal"].as_u64());
        let want = &want["row"];
        assert_eq!(row.display["role"], want["role"], "{}", row.mid);
        assert_eq!(row.display["text"], want["text"], "{}", row.mid);
        for call in want["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "tool_call")
        {
            let tools: Vec<&Value> = row.display["parts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|part| part["type"] == "tool")
                .collect();
            assert_eq!(tools.len(), 1, "{}", row.display);
            let tool = tools[0];
            assert_eq!(tool["callId"], call["tool_call_id"]);
            assert_eq!(tool["tool"], call["name"]);
            assert_eq!(tool["partKey"], call["part"]);
            assert!(tool["outputBytes"].is_u64(), "{tool}");
            assert_eq!(tool["startedAtMs"], 1_793_716_614_076_u64);
            assert_eq!(tool["finishedAtMs"], 1_793_716_614_082_u64);
        }
    }
}

/// A user entry a runner wrote for a send it delivered with a tool result is
/// left out of pages: the model read its text inside the tool's result, so the
/// apps must not draw it as the person's message. The other rows keep their
/// ordinals, and a user entry delivered any other way, here as a steer (the
/// person's message joining the running turn), stays.
#[tokio::test]
async fn a_send_delivered_with_a_tool_result_is_left_out_of_pages() {
    let read = sample("02-session-read-display-facts.json");
    let mut entries = read["messages"].as_array().unwrap().clone();
    let entry = |from: &Value, mid: &str, ordinal: u64, mode: &str| {
        let mut entry = from.clone();
        entry.as_object_mut().unwrap().remove("client_message_id");
        entry["mid"] = json!(mid);
        entry["ordinal"] = json!(ordinal);
        entry["delivered_as"] = json!({"mode": mode});
        entry
    };
    let mut ride = entry(&entries[0], "ride-status", 6, "tool_result");
    ride["delivered_as"]["call_key"] = json!("msg_1a1d773a300151XR52m1swzb79/call-1");
    let steer = entry(&entries[3], "steered", 7, "steer");
    entries.extend([ride, steer]);
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries;
    let source = BrocaSource::new(opencode2_binding(), route);
    let page = source.newest(20).await.unwrap();
    let kept: Vec<_> = page
        .rows
        .iter()
        .map(|row| (row.ordinal, row.mid.as_str()))
        .collect();
    assert_eq!(kept.len(), 6, "{kept:?}");
    assert_eq!(
        kept.iter().map(|(ordinal, _)| *ordinal).collect::<Vec<_>>(),
        [Some(1), Some(2), Some(3), Some(4), Some(5), Some(7)]
    );
    assert_eq!(kept[5].1, "steered");
}

/// Serves the recorded replies: the head, the read (with display facts when
/// asked for them), sliced to the requested ordinals, and the description.
struct Recorded;

struct Quiet;

#[async_trait]
impl EventStream for Quiet {
    async fn next(&mut self) -> Result<Option<Value>, ReadError> {
        std::future::pending().await
    }
}

#[async_trait]
impl BrocaRoute for Recorded {
    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        let request: Value = serde_json::from_slice(&body(op, params)).unwrap();
        let params = &request["params"];
        match request["method"].as_str().unwrap() {
            "session.head" => result(sample("03-session-head.json")),
            "role.describe" => result(sample("09-role-describe.json")),
            "session.read" => {
                let mut read = if params["include_display_facts"] == true {
                    sample("02-session-read-display-facts.json")
                } else {
                    sample("01-session-read.json")
                };
                let from = params["from_ordinal"].as_u64().unwrap_or(1);
                let limit = params["limit"].as_u64().unwrap_or(u64::MAX) as usize;
                let messages: Vec<Value> = read["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|entry| entry["ordinal"].as_u64().unwrap() >= from)
                    .take(limit)
                    .cloned()
                    .collect();
                read["messages"] = json!(messages);
                result(read)
            }
            method => panic!("the recorded runner was asked {method}"),
        }
    }

    async fn subscribe(&self, _live_ids: bool) -> Result<Box<dyn EventStream>, ReadError> {
        Ok(Box::new(Quiet))
    }
}

struct RecordedRoutes;

#[async_trait]
impl BrocaRoutes for RecordedRoutes {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "opencode2");
        Ok(Arc::new(Recorded))
    }
}

/// The app's newest page, exact read and details over the recorded runner: the
/// page binds the recorded history counter and log, the person's messages echo
/// the app's ids, and the call carries its recorded times.
#[tokio::test]
async fn an_app_reads_the_recorded_opencode2_session() {
    let app = super::super::super::AppReader::new(Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
            OPENCODE_RUNNER_MODULE_ID,
        ))))),
        Arc::new(RecordedRoutes),
    )));
    let session = json!({"session": "ses_head", "harness": "opencode2"});
    let mut request = session.clone();
    request["limit"] = json!(50);
    let page = app
        .read(super::super::super::TRANSCRIPT_PAGE, request)
        .await
        .unwrap();
    assert_eq!(
        page["version"],
        format!("history:0:incarnation:{INCARNATION}")
    );
    assert_eq!(page["reachedStart"], true);
    let messages = page["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 5, "{page}");
    assert_eq!(messages[0]["clientMessageId"], "app-message-1");
    assert_eq!(messages[3]["clientMessageId"], "app-message-2");
    let tool = |message: &Value| {
        message["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["type"] == "tool")
            .cloned()
            .unwrap()
    };
    for message in [&messages[1], &{
        let mut exact = session.clone();
        exact["messageOrdinal"] = json!(2);
        let exact = app
            .read(super::super::super::TRANSCRIPT_PAGE, exact)
            .await
            .unwrap();
        exact["messages"][0].clone()
    }] {
        let call = tool(message);
        assert_eq!(call["callId"], "call_probe", "{call}");
        assert_eq!(call["tool"], "bash");
        assert_eq!(call["startedAtMs"], 1_793_716_614_076_u64);
        assert_eq!(call["finishedAtMs"], 1_793_716_614_082_u64);
        // OpenCode 2 sessions don't declare call identities yet.
        assert!(call.get("callKey").is_none() && call.get("schemaPin").is_none());
    }
    let details = app
        .read(super::super::super::DETAILS, session)
        .await
        .unwrap();
    assert_eq!(details["incarnation"], INCARNATION);
    assert_eq!(details["lastRun"]["state"], "completed", "{details}");
}
