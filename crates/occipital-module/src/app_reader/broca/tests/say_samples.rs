//! The sample set `phone-reader-say-v3`: an agent's `say` calls as the apps
//! get them. On a chat whose plan gives the agent prefrontal's `say`, an
//! answered call is the agent's message, a `message` part in the call's place;
//! a call that failed stays a tool card; one still running is left out. On any
//! other chat the same calls are tool cards. `session.details` names the
//! chat's message tool to an app that asks. Its README says what each file shows.

use super::served_samples::{decoded, Served};
use super::tool_display_samples::{pin, reserved};
use super::*;
use crate::display::store::DisplayStore;
use cortexkit_role_tool_display::{DisplayKind, DisplayWrite};
use cortexkit_test_support::ScratchDir;

pub(super) const SESSION: &str = "alfonso:head";
const TEXT: &str = "Build passed. <Badge tone=\"success\">1,651 tests</Badge>";

/// The reply catalog the runner stamped the agent's turns with, so the apps
/// draw a message's tags as they draw a reply's.
pub(super) const CATALOG: &str = "4e256f56bf6a6f69d85f0bd9a101fd53637784fd76f7e8c53447069fd1dd77f5";

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
pub(super) fn chat(with_say: bool) -> (Arc<super::super::super::AppReader>, Arc<FakeRoute>) {
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
    // The frozen v1 and v2 tokens describe only the head; v3 covers both sets of message rows.
    let mut served = Served::new("phone-reader-say-v3", app);
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

    check_say_record_samples(served).await;
}

/// The same chat with each call's key and pin, as a runner serving call
/// identity stores them, and a second `say` that answers without a quote.
fn entries_with_keys() -> Vec<Value> {
    let mut entries = entries();
    let times = |id: &str, key: &str, tool: &str| {
        json!({"tool_call_id": id, "started_at": 1_500, "finished_at": 1_600,
            "call_key": key, "schema_pin": pin(tool, 's')})
    };
    entries[1]["tool_times"] = json!([
        times("call_read", "ck_read", "read"),
        times("call_say", "ck_say", "say")
    ]);
    entries[4]["tool_times"] = json!([times("call_failed", "ck_failed", "say")]);
    entries[6]["tool_times"] = json!([times("call_running", "ck_running", "say")]);
    entries.push(json!({"mid": "m8", "ordinal": 8, "written_at": 8_000,
        "rich_ui": {"dialect": "1.0", "catalog_digest": CATALOG},
        "tool_times": [times("call_later", "ck_later", "say")],
        "message": {"role": "assistant", "content": [{"type": "tool_call",
            "tool_call_id": "call_later", "tool_name": "say",
            "input": {"reply_to": "1", "quote": "Did it pass?", "text": "Also: the docs build passed."}}]}}));
    entries.push(json!({"mid": "m9", "ordinal": 9, "written_at": 9_000,
        "message": {"role": "tool", "content": [{"type": "tool_result",
            "tool_call_id": "call_later", "output": {"kind": "text", "text": "sent"},
            "is_error": false}]}}));
    entries
}

/// Prefrontal's message record for `call_key`, as it writes it before
/// answering the call.
fn message(call_key: &str, body: Value) -> DisplayWrite {
    DisplayWrite::new(
        reserved("broca"),
        call_key,
        DisplayKind::Message,
        pin("say", 's'),
        body.as_object().unwrap().clone(),
    )
}

/// The chat of `entries_with_keys` read by an app, with `records` stored as
/// prefrontal writes them. The display store lives in the returned folder,
/// and the caller keeps it and the call-pin task until it's done reading.
fn chat_with_records(
    records: &[&DisplayWrite],
) -> (
    Arc<super::super::super::AppReader>,
    Arc<DisplayStore>,
    ScratchDir,
    tokio::task::JoinHandle<()>,
) {
    let dir = ScratchDir::new("occipital-say-records");
    let descriptor = crate::store::tests::descriptor(&dir);
    let store = Arc::new(DisplayStore::open(&descriptor, dir.path()).unwrap());
    for write in records {
        let stored = store.write(&reserved("prefrontal-core"), write, 1).unwrap();
        assert_eq!(stored, cortexkit_role_tool_display::WriteReply::Stored);
    }
    let (call_pins, task) = crate::display::call_pins::start(store.clone(), None);
    let slot = crate::display::call_pins::CallPinSlot::default();
    let _ = slot.set(call_pins);
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries_with_keys();
    *route.run_state.lock().unwrap() = Value::Null;
    *route.describe.lock().unwrap() = Some(describe_with(&[
        "attachments",
        "transcript_reads",
        "plan_view",
    ]));
    *route.plan_view.lock().unwrap() = Some(Ok(plan(true)));
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(OneChat))),
            Arc::new(OneRoute(route)),
        )
        .with_call_pins(slot),
    )));
    (app, store, dir, task)
}

/// The keyed half of `phone-reader-say-v3`: the messages with prefrontal's records joined. The
/// first answers the person's message with a verified quote, carries a picture
/// and a link, and alerts; the second answers without a quote, since its quote
/// didn't match the person's words, so prefrontal left it out.
async fn check_say_record_samples(mut served: Served) {
    let first = message(
        "ck_say",
        json!({"v": 1, "reply_to": "m1", "quote": "Did the build pass?",
            "media": [
                {"kind": "image", "artifact": "art_1234abcd", "mime": "image/png",
                    "byteCount": 48_211, "width": 1280, "height": 720},
                {"kind": "link", "url": "https://github.com/cortexkit/occipital/actions",
                    "title": "Gate run"}
            ],
            "level": "attention", "key": "build", "deliver": "phone"}),
    );
    let later = message(
        "ck_later",
        json!({"v": 1, "reply_to": "m1", "media": [], "level": "chat"}),
    );
    let (app, store, _dir, _task) = chat_with_records(&[&first, &later]);
    // Only prefrontal writes a message record, under either of its names.
    let refused = store.write(&reserved("aft"), &first, 1).unwrap_err();
    assert_eq!(refused.code, "display_writer_unverified");
    assert_eq!(
        store.write(&reserved("prefrontal"), &first, 2).unwrap(),
        cortexkit_role_tool_display::WriteReply::AlreadyStored
    );
    served.serve_with(app);
    let newest = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": SESSION}});
    let reply = served.read(6, &newest).await;
    let page = decoded(&reply);
    assert_eq!(
        parts(&page, "m2")[2],
        json!({"type": "message", "partIndex": 2, "partKey": "2", "callId": "call_say",
            "callKey": "ck_say", "text": TEXT,
            "replyTo": {"messageId": "m1", "quote": "Did the build pass?", "verified": true},
            "media": first.body["media"],
            "level": "attention", "key": "build", "deliver": "phone"}),
        "{page}"
    );
    assert_eq!(
        parts(&page, "m8")[0],
        json!({"type": "message", "partIndex": 0, "partKey": "0", "callId": "call_later",
            "callKey": "ck_later", "text": "Also: the docs build passed.",
            "replyTo": {"messageId": "m1"}, "level": "chat"}),
        "{page}"
    );
    assert_eq!(parts(&page, "m5")[0]["type"], "tool", "{page}");
    for marker in [
        occipital_app_reader::say::SAY_TEXT,
        occipital_app_reader::say::SAY_RECORD,
    ] {
        assert!(
            !String::from_utf8_lossy(&reply).contains(marker),
            "{marker}"
        );
    }
    served.keep("06-newest-page-with-records", &newest, reply);

    let exact = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": SESSION, "messageOrdinal": 2}});
    let reply = served.read(7, &exact).await;
    let message = decoded(&reply)["result"]["messages"][0]["parts"][2].clone();
    assert_eq!(message["replyTo"]["verified"], true, "{message}");
    served.keep("07-exact-read-with-records", &exact, reply);

    served.check();
}

/// A full Git object id for these samples, invented.
fn sha(digit: char) -> String {
    digit.to_string().repeat(40)
}

/// `phone-reader-say-diff-v2`: messages whose records carry `diff` media, as
/// prefrontal writes them (tool-display contract §9). The first carries a
/// pull request's changes and a commit's, with a renamed and a binary file in
/// each list; the second a range in a local checkout, which names no host or
/// owner, so the apps draw no repository link for it.
#[tokio::test]
async fn the_say_diff_samples_are_what_the_reader_serves() {
    let repo = json!({"host": "github.com", "owner": "cortexkit", "name": "occipital"});
    let first = message(
        "ck_say",
        json!({"v": 1, "reply_to": "m1", "quote": "Did the build pass?",
            "media": [
                {"kind": "diff", "artifact": "art_5e1f0a2b", "repo": repo,
                    "pr": {"number": 512, "head": sha('a')},
                    "files": [
                        {"path": "crates/occipital-lang/src/geometry/layout.rs",
                            "added": 42, "removed": 7},
                        {"path": "crates/occipital-registry/TEMPLATES.md",
                            "from": "crates/occipital-registry/ROWS.md",
                            "added": 3, "removed": 1},
                        {"path": "ports/swift/OccipitalCore.xcframework/ios-arm64/liboccipital_ffi.a",
                            "binary": true}
                    ],
                    "added": 45, "removed": 8},
                {"kind": "diff", "artifact": "art_9c3d7e41", "repo": repo,
                    "commit": sha('b'),
                    "files": [
                        {"path": "docs/release.md", "added": 12, "removed": 0},
                        {"path": "assets/icon.png", "from": "assets/logo.png", "binary": true}
                    ],
                    "added": 12, "removed": 0}
            ],
            "level": "chat"}),
    );
    let later = message(
        "ck_later",
        json!({"v": 1, "reply_to": "m1",
            "media": [
                {"kind": "diff", "artifact": "art_77aa0b19", "repo": {"name": "notes"},
                    "range": {"base": sha('c'), "head": sha('d')},
                    "files": [{"path": "todo.md", "added": 2, "removed": 1}],
                    "added": 2, "removed": 1}
            ],
            "level": "chat"}),
    );
    let (app, _store, _dir, _task) = chat_with_records(&[&first, &later]);
    // The diff pages from v1 are frozen; v2 checks their record-sensitive tokens.
    let mut served = Served::new("phone-reader-say-diff-v2", app);

    let newest = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": SESSION}});
    let reply = served.read(1, &newest).await;
    let page = decoded(&reply);
    // The media reach the message part exactly as prefrontal wrote them.
    assert_eq!(
        parts(&page, "m2")[2]["media"],
        first.body["media"],
        "{page}"
    );
    assert_eq!(
        parts(&page, "m8")[0]["media"],
        later.body["media"],
        "{page}"
    );
    served.keep("01-newest-page", &newest, reply);

    let exact = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": SESSION, "messageOrdinal": 2}});
    let reply = served.read(2, &exact).await;
    let message = decoded(&reply)["result"]["messages"][0]["parts"][2].clone();
    assert_eq!(message["media"], first.body["media"], "{message}");
    served.keep("02-exact-read", &exact, reply);

    served.check();
}

/// The kinds of the newest page's row `mid`'s parts, read straight from a
/// session source.
fn kinds(page: &Value, mid: &str) -> Vec<String> {
    page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["messageId"] == mid)
        .unwrap_or_else(|| panic!("{page}"))["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| part["type"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// While a chat's plan can't be read and no earlier answer is kept, a page
/// holding a `say` call is refused for now, never served with its calls
/// guessed: an app keeps a page until its version moves, and settling the
/// calls later doesn't move it. Once the plan reads, the same page draws them
/// as messages, and a later read that fails keeps that answer.
#[tokio::test]
async fn say_calls_wait_for_a_readable_plan() {
    check_plan_outage("provider_unavailable").await;
}

#[tokio::test]
async fn an_oversized_plan_is_not_evidence_about_say() {
    check_plan_outage("too_large").await;
}

async fn check_plan_outage(code: &'static str) {
    let (_, route) = chat(true);
    let unreadable = || Some(Err(ReadError::new(code, "the plan cannot be read")));
    *route.plan_view.lock().unwrap() = unreadable();
    let sources = BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(OneChat))),
        Arc::new(OneRoute(route.clone())),
    );
    let reference = SessionRef {
        runner: "broca".into(),
        session: SESSION.into(),
    };
    let source = sources.source(&reference).await.unwrap();
    let reader = TranscriptReader::default();
    let params = json!({"session": SESSION});

    let refused = reader
        .read(Some(source.as_ref()), params.clone())
        .await
        .unwrap_err();
    assert_eq!(refused.code, "provider_unavailable", "{}", refused.message);
    assert_eq!(source.message_tool().await, None);

    *route.plan_view.lock().unwrap() = Some(Ok(plan(true)));
    let page = reader
        .read(Some(source.as_ref()), params.clone())
        .await
        .unwrap();
    assert_eq!(kinds(&page, "m2"), ["text", "tool", "message"], "{page}");

    *route.plan_view.lock().unwrap() = unreadable();
    source.expire_message_tool();
    let page = reader.read(Some(source.as_ref()), params).await.unwrap();
    assert_eq!(kinds(&page, "m2"), ["text", "tool", "message"], "{page}");
    assert_eq!(source.message_tool().await, Some("say"));
}

/// A runner whose description can't be read now is refused its plan view for
/// now, not as one that serves none, which is final; and a page whose plan the
/// runner holds none of draws its `say` calls as tool cards.
#[tokio::test]
async fn an_unread_description_isnt_taken_for_a_runner_without_plans() {
    let (_, route) = chat(true);
    let sources = BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(OneChat))),
        Arc::new(OneRoute(route.clone())),
    );
    let reference = SessionRef {
        runner: "broca".into(),
        session: SESSION.into(),
    };
    let source = sources.source(&reference).await.unwrap();
    let described = route.describe.lock().unwrap().take();
    assert_eq!(
        source.media_read("sha", 0, 1).await.unwrap_err().code,
        "provider_unavailable"
    );
    let refused = source.plan_view().await.unwrap_err();
    assert_eq!(refused.code, "provider_unavailable", "{}", refused.message);

    *route.describe.lock().unwrap() = described;
    *route.plan_view.lock().unwrap() = None;
    let page = TranscriptReader::default()
        .read(Some(source.as_ref()), json!({"session": SESSION}))
        .await
        .unwrap();
    assert_eq!(kinds(&page, "m2"), ["text", "tool", "tool"], "{page}");
}
