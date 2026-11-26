//! AFT's own notice records and notice views, as AFT exported them from its
//! reviewed notice branch, copied byte for byte into
//! `tests/data/aft-notice-1120/` (its README says what each file is). Each
//! view reaches Occipital as AFT's views bucket delivers it, through the
//! bucket decoding and the fetcher's checks; each record as AFT announces it,
//! read back with `display.read` and stored. The app sample set
//! `phone-reader-notice-records-aft-v2` is what the apps then receive for a
//! chat holding those notices, so they draw AFT's real cards. `served_samples`
//! says how the set is checked and rewritten.
//!
//! AFT recorded its views against the tool catalog of the day. A catalog
//! change makes AFT publish the same views naming the new catalog, so each
//! view is read here with its catalog digest moved to the current one, the
//! only change AFT's republished view carries.

use super::served_samples::{decoded, Served};
use super::tool_display_samples::AnyHarness;
use super::*;
use crate::display::app::{ToolDisplay, TEMPLATE, VIEWS};
use crate::display::daemon::CallFailure;
use crate::display::store::{DisplayStore, ViewState};
use crate::display::views::{ModuleFacts, Providers as ViewProviders};
use cortexkit_role_tool_display::{NoticeBody, NoticeRecord};
use cortexkit_role_tool_provider::catalog::CatalogTool;
use cortexkit_role_tool_provider::views::{ViewsReply, ViewsRequest};
use cortexkit_test_support::ScratchDir;
use std::collections::HashMap;
use subc_protocol::Principal;

pub(super) const AFT: &str = "aft";
pub(super) const SESSION: &str = "ses_aft_notice_samples";
/// The notices in the order the chat holds them. `still-running` rides a tool
/// result; the others are their own messages.
const KINDS: [&str; 7] = [
    "still-running",
    "finished",
    "failed",
    "stopped",
    "timed-out",
    "watch-match",
    "watch-task-exit",
];

pub(super) fn data(path: &str) -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/aft-notice-1120");
    std::fs::read(root.join(path)).unwrap()
}

pub(super) fn json_file(path: &str) -> Value {
    serde_json::from_slice(&data(path)).unwrap()
}

/// The tool catalog AFT's recorded views name.
const RECORDED_CATALOG: &str = "6e3239281ade150b0935a2ca239ead0f7f922ab0fdeeffc470a19fc0fbf3ddb0";

/// A recorded view's bytes as AFT publishes them for the current catalog.
fn on_current_catalog(bytes: Vec<u8>) -> Vec<u8> {
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains(RECORDED_CATALOG));
    text.replace(
        RECORDED_CATALOG,
        occipital_lang::Catalog::tool_templates_digest(),
    )
    .into_bytes()
}

/// AFT's write for notice `kind`.
pub(super) fn write(kind: &str) -> Value {
    json_file(&format!("records/{kind}.write.json"))
}

/// AFT, as the views fetcher asks it: it serves views and has no tool the
/// notice keys name. Its views come from its bucket, so `tool.views` is never
/// asked.
struct AftViews;

#[async_trait]
impl ViewProviders for AftViews {
    async fn modules(&self) -> Result<HashMap<String, ModuleFacts>, String> {
        Ok(HashMap::from([(
            AFT.to_owned(),
            ModuleFacts {
                version: "0.64.0+01289469c709".into(),
                serves_views: true,
            },
        )]))
    }
    async fn catalog(&self, _: &str) -> Result<Vec<CatalogTool>, String> {
        Ok(Vec::new())
    }
    async fn views(&self, _: &str, _: &ViewsRequest) -> Result<ViewsReply, String> {
        Err("tool.views isn't asked for a view the bucket delivered".into())
    }
}

/// AFT, as the record pull reaches it: listed by the daemon as a module that
/// proved who it is (so its records are taken), answering each announced
/// record id with the write it stored.
struct AftRecords(HashMap<String, Value>);

#[async_trait]
impl crate::display::pull::Providers for AftRecords {
    async fn verified(&self, _: &str) -> Result<bool, String> {
        Ok(true)
    }
    async fn read(&self, _: &str, record_id: &str) -> Result<Value, CallFailure> {
        Ok(self.0[record_id].clone())
    }
}

/// The agent directory (Entorhinal), which tells the reader which runner hosts
/// a session. It knows only the session AFT's records were written to: an
/// OpenCode 2 chat on Thalamus's runner.
pub(super) struct ByHarnessForAft;

#[async_trait]
impl Entorhinal for ByHarnessForAft {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        match method {
            "agent.session_for_harness" => {
                assert_eq!(params, json!({"harness": "opencode2", "session": SESSION}))
            }
            "agent.session_for_runner" => assert_eq!(
                params,
                json!({"runner": THALAMUS_RUNNER_MODULE_ID, "session": SESSION})
            ),
            other => panic!("unexpected Entorhinal call {other}"),
        }
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref",
            "incarnation":"entorhinal-test", "ended_at_ms":null,
            "runner":THALAMUS_RUNNER_MODULE_ID, "session":SESSION, "harness":"opencode2",
            "project_root":"/workspace/head", "network":"unknown", "network_history":[]}}),
        )
    }
}

/// A user-role row AFT sent as its own notice.
fn notice_row(ordinal: u64, send_id: &str) -> Value {
    let text = format!("<system-reminder>\nAFT notice {ordinal}.\n</system-reminder>");
    json!({"mid": format!("msg_{ordinal}"), "ordinal": ordinal, "role": "user",
        "written_at": 1_793_800_000_000_u64 + ordinal * 1_000,
        "sender": AFT, "send_id": send_id, "delivered_as": {"mode": "idle_start"},
        "notice": true,
        "message": {"id": format!("msg_{ordinal}"), "type": "user", "text": text,
            "time": {"created": 1_793_800_000_000_u64 + ordinal * 1_000}}})
}

fn chat() -> Vec<Value> {
    let send = |kind: &str| write(kind)["send_id"].as_str().unwrap().to_owned();
    let mut rows = vec![json!({"mid": "msg_1", "ordinal": 1,
    "tool_times": [{"tool_call_id": "call_one", "started_at": 1_793_800_000_100_u64,
        "finished_at": 1_793_800_000_160_u64, "call_key": "ck_one"}],
    "notices": [{"part": 0, "call_key": "ck_one", "send_id": send("still-running"),
        "sender": AFT, "text": "<system-reminder>\nAFT notice 1.\n</system-reminder>",
        "delivered_as": "tool_result"}],
    "message": {"id": "msg_1", "type": "assistant",
        "time": {"created": 1_793_800_000_000_u64, "completed": 1_793_800_001_000_u64},
        "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
        "content": [
            {"type": "tool", "id": "call_one", "name": "read",
                "state": {"status": "completed", "input": {"path": "README.md"},
                    "content": [{"type": "text", "text": "# Example\n"}]}},
            {"type": "text", "text": "The upload is still going."}
        ]}})];
    for (n, kind) in KINDS.iter().enumerate().skip(1) {
        rows.push(notice_row(n as u64 + 1, &send(kind)));
    }
    rows
}

/// The record the apps are served for notice `kind`: AFT's body, its view
/// qualified by AFT.
pub(super) fn served(kind: &str) -> Value {
    let body: NoticeBody = serde_json::from_value(write(kind)["body"].clone()).unwrap();
    let writer = Principal::Reserved {
        module_id: AFT.into(),
    };
    serde_json::to_value(NoticeRecord::from_body(&writer, body).unwrap()).unwrap()
}

/// AFT's three notice views, each offered to `store` as AFT's bucket delivers
/// it and kept by the time this returns, with their keys. The views task runs
/// until the returned handle drops.
pub(super) async fn keep_aft_views(
    store: &Arc<DisplayStore>,
) -> (Vec<String>, tokio::task::JoinHandle<()>) {
    let (fetcher, fetching) = crate::display::views::start(store.clone(), Arc::new(AftViews), None);
    let mut keys = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/aft-notice-1120/views"),
    )
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .collect();
    entries.sort();
    for path in entries {
        let bucket_key = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let (key, reply) = crate::bus::served_view(
            &bucket_key,
            &on_current_catalog(std::fs::read(&path).unwrap()),
        )
        .unwrap();
        fetcher.offer(AFT, &key, reply).await;
        keys.push(key);
    }
    assert_eq!(
        keys,
        [
            "nv1:aft:bash_finished:1",
            "nv1:aft:bash_still_running:1",
            "nv1:aft:bash_watch:1"
        ]
    );
    for _ in 0..2_000 {
        if keys
            .iter()
            .all(|key| matches!(store.view(AFT, key), Ok(Some(ViewState::View { .. }))))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    for key in &keys {
        assert!(
            matches!(store.view(AFT, key), Ok(Some(ViewState::View { .. }))),
            "{key} wasn't kept"
        );
    }
    (keys, fetching)
}

#[tokio::test]
async fn aft_s_notices_reach_the_apps_with_aft_s_own_views() {
    let dir = ScratchDir::new("occipital-aft-notice-1120");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, _task) = crate::notices::Notices::start(file);
    let slot = crate::notices::NoticeSlot::default();
    let _ = slot.set(notices.clone());
    let store = Arc::new(DisplayStore::open(&descriptor, main.data_root()).unwrap());

    // AFT's views, as its bucket delivers each one.
    let (keys, _fetching) = keep_aft_views(&store).await;

    // AFT's records, each read back as its announcement names it.
    let announced: HashMap<String, Value> = KINDS
        .iter()
        .map(|kind| {
            let ready = json_file(&format!("records/{kind}.ready.json"));
            (ready["record_id"].as_str().unwrap().to_owned(), write(kind))
        })
        .collect();
    let sink = crate::display::pull::Sink {
        state: None,
        views: None,
        hints: None,
        notices: Some(notices.clone()),
    };
    let providers = AftRecords(announced.clone());
    for record_id in announced.keys() {
        assert_eq!(
            crate::display::pull::pull(&providers, &sink, AFT, record_id).await,
            crate::display::pull::Pulled::Stored,
            "{record_id}"
        );
    }

    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = chat();
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(ByHarnessForAft))),
            Arc::new(AnyHarness(route.clone())),
        )
        .with_notices(slot),
    )));
    app.set_notices(notices);
    let hints = tokio::sync::broadcast::channel(16).0;
    app.set_tool_display(Arc::new(ToolDisplay::new(store, None, hints)));
    let mut served_set = Served::new("phone-reader-notice-records-aft-v2", app);
    let request = |method: &str, members: Value| {
        let mut params = json!({"session": SESSION, "harness": "opencode2"});
        for (name, value) in members.as_object().unwrap() {
            params[name] = value.clone();
        }
        json!({"method": method, "params": params})
    };

    // 01: the page, which names each row's send.
    let page_request = request(super::super::super::TRANSCRIPT_PAGE, json!({"limit": 50}));
    let page = served_set.read(1, &page_request).await;
    assert_eq!(
        decoded(&page)["result"]["messages"]
            .as_array()
            .unwrap()
            .len(),
        KINDS.len()
    );
    served_set.keep("01-page", &page_request, page);

    // 02: every notice row's record, each joined on AFT's own view.
    let ids: Vec<String> = (2..=KINDS.len()).map(|n| format!("msg_{n}")).collect();
    let asked = request(
        crate::notices::wakes::READ,
        json!({"messageIds": ids, "acceptNoticeRecords": true}),
    );
    let records = served_set.read(2, &asked).await;
    let expected: serde_json::Map<String, Value> = KINDS
        .iter()
        .enumerate()
        .skip(1)
        .map(|(n, kind)| (format!("msg_{}", n + 1), served(kind)))
        .collect();
    assert_eq!(
        decoded(&records)["result"],
        json!({"messages": {}, "notices": expected})
    );
    served_set.keep("02-wake-records-with-notices", &asked, records);

    // 03: the reminder that rode the call's result.
    let riding = request(
        crate::notices::READ,
        json!({"callKeys": ["ck_one"], "acceptNoticeRecords": true}),
    );
    let mut read = Vec::new();
    for corr in 3..1_000 {
        read = served_set.read(corr, &riding).await;
        if !decoded(&read)["result"]["calls"]["ck_one"].is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        decoded(&read)["result"]["calls"]["ck_one"][0]["noticeRecord"],
        served("still-running")
    );
    served_set.keep("03-notices-riding", &riding, read);

    // 04: AFT's three notice views, each with a row and a page that passed
    // Occipital's checks.
    let views_request = json!({"method": VIEWS, "params": {"pins": keys}});
    let views = served_set.read(1_001, &views_request).await;
    let answers = decoded(&views)["result"]["views"].clone();
    served_set.keep("04-notice-views", &views_request, views);

    // 05 to 10: each template by digest.
    let mut n = 5;
    for key in &keys {
        for slot in ["row", "page"] {
            let digest = &answers[key][slot];
            assert!(digest.is_string(), "{key} {slot}: {}", answers[key]);
            let template_request = json!({"method": TEMPLATE, "params": {"digest": digest}});
            let document = served_set.read(1_000 + n, &template_request).await;
            assert!(
                decoded(&document)["result"]["template"]["source"].is_string(),
                "{key} {slot}"
            );
            let name = key.trim_start_matches("nv1:aft:").trim_end_matches(":1");
            served_set.keep(
                &format!("{n:02}-template-{name}-{slot}"),
                &template_request,
                document,
            );
            n += 1;
        }
    }

    served_set.check();
}
