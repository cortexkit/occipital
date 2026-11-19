//! The sample set `phone-reader-simple-v1`: the Simple view of a busy chat as
//! the apps read it with `view: "simple"` (`occipital_app_reader::simple`).
//! Its README says what each file shows. The chat is a Broca head whose plan
//! gives the agent prefrontal's `say` and `ask`: the person's messages and
//! the agent's messages and asks sit among 1,300 messages of the agent's own
//! prose, and a wake holds the person's answer to an ask.

use super::served_samples::{decoded, Served};
use super::tool_display_samples::{pin, reserved};
use super::*;
use crate::display::store::DisplayStore;
use cortexkit_role_tool_display::{DisplayKind, DisplayWrite};
use cortexkit_test_support::ScratchDir;
use subc_client_rs::HandlerOutcome;
use subc_protocol::Principal;

const SET: &str = "phone-reader-simple-v1";
const SESSION: &str = "alfonso:head";
/// The agent's own prose between the messages the view shows.
const FILLER: u64 = 1_300;

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

/// The plan in force: prefrontal's `say` and `ask` beside AFT's `bash`, or
/// `bash` alone.
fn plan(message_tool: bool) -> Value {
    let tool = |name: &str, provider: &str| {
        json!({"name": name, "description": name, "input_schema": {"type": "object"},
            "provider": provider})
    };
    let mut tools = vec![tool("bash", "aft")];
    if message_tool {
        tools.push(tool("say", "prefrontal"));
        tools.push(tool("ask", "prefrontal"));
    }
    json!({"generation": 3, "applied_at_ms": 1_794_400_000_000_u64,
        "system": {"text": "Be brief."}, "tools": tools, "as_planned": true})
}

fn entry(ordinal: u64, message: Value) -> Value {
    json!({"mid": format!("m{ordinal}"), "ordinal": ordinal, "written_at": 1_000 * ordinal,
        "message": message})
}

fn person(ordinal: u64, text: &str) -> Value {
    entry(
        ordinal,
        json!({"role": "user", "content": [{"type": "text", "text": text}]}),
    )
}

fn prose(ordinal: u64) -> Value {
    entry(
        ordinal,
        json!({"role": "assistant", "content": [{"type": "text", "text": "Working on it."}]}),
    )
}

fn call(id: &str, tool: &str, input: Value) -> Value {
    json!({"type": "tool_call", "tool_call_id": id, "tool_name": tool, "input": input})
}

fn result(ordinal: u64, id: &str, text: &str) -> Value {
    entry(
        ordinal,
        json!({"role": "tool", "content": [{"type": "tool_result", "tool_call_id": id,
            "output": {"kind": "text", "text": text}, "is_error": false}]}),
    )
}

/// A wake prefrontal sent, with its send id.
fn wake(ordinal: u64, send: &str) -> Value {
    let mut wake = entry(
        ordinal,
        json!({"role": "user", "content": [{"type": "text",
            "text": "<system-reminder>\nInbox: 1 notice.\n</system-reminder>"}]}),
    );
    wake["sender"] = json!("prefrontal");
    wake["send_id"] = json!(send);
    wake["delivered_as"] = json!({"mode": "idle_start"});
    wake["notice"] = json!(true);
    wake
}

/// The chat: the person's question, the agent's prose, then each kind the
/// view shows among kinds it leaves out.
fn entries() -> Vec<Value> {
    let mut entries = vec![person(1, "Is the release ready?")];
    entries.extend((2..2 + FILLER).map(prose));
    let at = 2 + FILLER;
    let mut host_only = person(at + 3, "[Request interrupted by user]");
    host_only["host_parts"] = json!([{"part": "0", "host_code": "interrupted"}]);
    let mut photo = entry(
        at + 4,
        json!({"role": "user", "content": [{"type": "media", "kind": "image",
            "mime": "image/png", "sha256": "ab".repeat(32), "byte_size": 3_592,
            "width": 640, "height": 160}]}),
    );
    photo["notice"] = Value::Null;
    let mut asked = entry(
        at + 5,
        json!({"role": "assistant", "content": [
            {"type": "text", "text": "Checking the release notes."},
            call("call_say", "say", json!({"text": "The release notes are ready.", "level": "chat"})),
            call("call_ask_open", "ask", json!({"question": "Publish the release notes?",
                "options": ["Publish", "Hold"]})),
            call("call_dismiss", "ask", json!({"action": "dismiss", "id": "ask_0123456789abcdef"})),
        ]}),
    );
    asked["tool_times"] = json!([
        {"tool_call_id": "call_say", "started_at": 1_000 * (at + 5), "finished_at": 1_000 * (at + 6),
            "call_key": "ck_say", "schema_pin": pin("say", 's')},
        {"tool_call_id": "call_ask_open", "started_at": 1_000 * (at + 5),
            "finished_at": 1_000 * (at + 6), "call_key": "ck_ask_open", "schema_pin": pin("ask", 'a')},
        {"tool_call_id": "call_dismiss", "started_at": 1_000 * (at + 5),
            "finished_at": 1_000 * (at + 6), "call_key": "ck_dismiss", "schema_pin": pin("ask", 'a')},
    ]);
    let mut asked_again = entry(
        at + 7,
        json!({"role": "assistant", "content": [call("call_ask_done", "ask",
            json!({"question": "Tag the release now?", "options": ["Tag it", "Wait"]}))]}),
    );
    asked_again["tool_times"] = json!([{"tool_call_id": "call_ask_done",
        "started_at": 1_000 * (at + 7), "finished_at": 1_000 * (at + 8),
        "call_key": "ck_ask_done", "schema_pin": pin("ask", 'a')}]);
    entries.extend([
        wake(at, "inbox-send_0000000000000001"),
        person(
            at + 1,
            "<system-reminder>\nInbox: 2 notices.\n</system-reminder>",
        ),
        person(
            at + 2,
            "<system-reminder>\nInbox: 1 notice.\n</system-reminder>\nAnd the changelog?",
        ),
        host_only,
        photo,
        asked,
        entry(
            at + 6,
            json!({"role": "tool", "content": [
                {"type": "tool_result", "tool_call_id": "call_say",
                    "output": {"kind": "text", "text": "sent"}, "is_error": false},
                {"type": "tool_result", "tool_call_id": "call_ask_open",
                    "output": {"kind": "text", "text": "ask_1111111111111111 · open"}, "is_error": false},
                {"type": "tool_result", "tool_call_id": "call_dismiss",
                    "output": {"kind": "text", "text": "dismissed"}, "is_error": false},
            ]}),
        ),
        asked_again,
        result(at + 8, "call_ask_done", "ask_2222222222222222 · open"),
        wake(at + 9, "inbox-send_0000000000000002"),
        prose(at + 10),
    ]);
    entries
}

/// The person's answer to the second ask, as prefrontal writes the wake's
/// record before it sends the wake: the first `ask_answer` item in the
/// tool-display contract's `wakes.json` vector.
fn answer() -> Value {
    let (_, text) = cortexkit_role_tool_display::vectors::FILES
        .iter()
        .find(|(name, _)| *name == "wakes.json")
        .unwrap();
    let vectors: Value = serde_json::from_str(text).unwrap();
    let item = vectors["writes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|case| case["write"]["body"]["items"].as_array().unwrap().clone())
        .find(|item| item["kind"] == "ask_answer")
        .unwrap();
    json!({ "items": [item] })
}

/// A reader of the chat on `route`, with its notices and display store.
struct Chat {
    app: Arc<super::super::super::AppReader>,
    route: Arc<FakeRoute>,
    store: Arc<DisplayStore>,
    hints: tokio::sync::broadcast::Sender<crate::display::app::Hint>,
    _dir: ScratchDir,
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

async fn chat(message_tool: bool, entries: Vec<Value>) -> Chat {
    let dir = ScratchDir::new("occipital-simple-samples");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, notices_task) = crate::notices::Notices::start(file);
    let notice_slot = crate::notices::NoticeSlot::default();
    let _ = notice_slot.set(notices.clone());
    let write = json!({"kind": "wake", "runner": {"kind": "reserved", "module_id": "broca"},
        "session": SESSION, "send_id": "inbox-send_0000000000000002", "body": answer()});
    let reply = crate::notices::wakes::write(
        Some(&notices),
        Some(Principal::Reserved {
            module_id: "prefrontal".into(),
        }),
        &write,
    )
    .await;
    assert!(matches!(reply, HandlerOutcome::Response(_)), "{reply:?}");

    let store = Arc::new(DisplayStore::open(&descriptor, dir.path()).unwrap());
    let (call_pins, pins_task) = crate::display::call_pins::start(store.clone(), None);
    let pins = crate::display::call_pins::CallPinSlot::default();
    let _ = pins.set(call_pins);
    let hints = tokio::sync::broadcast::channel(crate::display::app::HINTS).0;

    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries;
    *route.run_state.lock().unwrap() = Value::Null;
    *route.describe.lock().unwrap() = Some(describe_with(&[
        "attachments",
        "transcript_reads",
        "plan_view",
    ]));
    *route.plan_view.lock().unwrap() = Some(Ok(plan(message_tool)));
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(OneChat))),
            Arc::new(OneRoute(route.clone())),
        )
        .with_call_pins(pins)
        .with_notices(notice_slot),
    )));
    app.set_notices(notices);
    app.set_tool_display(Arc::new(crate::display::app::ToolDisplay::new(
        store.clone(),
        None,
        hints.clone(),
    )));
    Chat {
        app,
        route,
        store,
        hints,
        _dir: dir,
        _tasks: vec![notices_task, pins_task],
    }
}

fn page(extra: Value) -> Value {
    let mut params = json!({"session": SESSION, "harness": "broca", "view": "simple"});
    for (name, value) in extra.as_object().unwrap() {
        params[name] = value.clone();
    }
    json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": params})
}

fn ids(reply: &Value) -> Vec<String> {
    reply["result"]["messages"]
        .as_array()
        .or_else(|| reply["result"]["newTurns"].as_array())
        .or_else(|| reply["result"]["resync"]["messages"].as_array())
        .unwrap_or_else(|| panic!("{reply}"))
        .iter()
        .map(|row| row["messageId"].as_str().unwrap().to_owned())
        .collect()
}

fn claim(version: &Value, since: &str, floor: &str) -> Value {
    json!({"heldVersion": version, "sinceMessageId": since, "heldFloorMessageId": floor})
}

/// The set being served, numbering its requests.
struct Sample {
    served: Served,
    corr: u64,
}

impl Sample {
    /// Read `request`, keep the reply as file `name`, and return it decoded.
    async fn read(&mut self, name: &str, request: Value) -> Value {
        self.corr += 1;
        let reply = self.served.read(self.corr, &request).await;
        let decoded = decoded(&reply);
        self.served.keep(name, &request, reply);
        decoded
    }

    /// Read `request` without keeping it, as a read between two samples.
    async fn quiet(&mut self, request: Value) -> Value {
        self.corr += 1;
        decoded(&self.served.read(self.corr, &request).await)
    }

    /// Send `request`, which is refused, and keep the refusal as file `name`.
    async fn refused(&mut self, name: &str, request: Value) -> String {
        self.corr += 1;
        self.served.send(self.corr, &request).await;
        let (_, reply) = self
            .served
            .next(self.corr, Duration::from_secs(5))
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&reply).into_owned();
        self.served.keep(name, &request, reply);
        text
    }
}

#[tokio::test]
async fn the_simple_view_samples_are_what_the_reader_serves() {
    let at = 2 + FILLER;
    let c = chat(true, entries()).await;
    let mut served = Sample {
        served: Served::new(SET, c.app.clone()),
        corr: 0,
    };

    // 01: the newest five rows the view shows, each trimmed to what it draws.
    let newest = served
        .read("01-newest-page", page(json!({"limit": 5})))
        .await;
    let m = |ordinal: u64| format!("m{ordinal}");
    assert_eq!(
        ids(&newest),
        [m(at + 2), m(at + 4), m(at + 5), m(at + 7), m(at + 9)]
    );
    let first = &newest["result"];
    assert_eq!(first["view"], "simple");
    assert!(
        first["version"].as_str().unwrap().ends_with(":simple:1"),
        "{first}"
    );
    let rows = first["messages"].as_array().unwrap();
    let kinds = |row: &Value| -> Vec<String> {
        row["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|part| part["type"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(kinds(&rows[2]), ["message", "tool"], "{}", rows[2]);
    assert_eq!(rows[2]["parts"][0]["callKey"], "ck_say");
    assert_eq!(rows[2]["parts"][1]["tool"], "ask");
    assert_eq!(rows[2]["text"], "", "the agent's prose isn't drawn");
    assert_eq!(
        kinds(&rows[4]),
        Vec::<String>::new(),
        "the wake draws from its record"
    );
    let next = first["nextBeforeOrdinal"].as_u64().unwrap();

    // 02: older, across more of the agent's prose than one read goes through.
    let older = served
        .read(
            "02-older-past-the-bound",
            page(json!({"limit": 5, "beforeOrdinal": next})),
        )
        .await;
    assert!(ids(&older).is_empty(), "{older}");
    let next = older["result"]["nextBeforeOrdinal"].as_u64().unwrap();
    assert!(older["result"].get("reachedStart").is_none());

    // 03: older again, to the start.
    let start = served
        .read(
            "03-older-to-the-start",
            page(json!({"limit": 5, "beforeOrdinal": next})),
        )
        .await;
    assert_eq!(ids(&start), [m(1)]);
    assert_eq!(start["result"]["reachedStart"], true);

    // 04: a claim on the newest page after the agent's prose and a new
    // message from the person: only the message is new.
    let version = first["version"].clone();
    {
        let mut entries = c.route.entries.lock().unwrap();
        entries.push(prose(at + 11));
        entries.push(person(at + 12, "Ship it."));
    }
    let matched = served
        .read(
            "04-claim-matched",
            page(json!({"limit": 5, "claim": claim(&version, &m(at + 9), &m(at + 2))})),
        )
        .await;
    assert_eq!(matched["result"]["claim"], "matched", "{matched}");
    assert_eq!(ids(&matched), [m(at + 12)]);

    // 05: a `say` call that was still unanswered when the claimant last read
    // the page is answered after the person's next message: its row now joins
    // the view inside the held range, so the claim answers `diverged` from it.
    {
        let mut entries = c.route.entries.lock().unwrap();
        entries.push(entry(
            at + 13,
            json!({"role": "assistant", "content": [call("call_late", "say",
                json!({"text": "Tagging now.", "level": "chat"}))]}),
        ));
        entries.push(person(at + 14, "Also check the docs."));
    }
    let baseline = served
        .quiet(page(
            json!({"limit": 5, "claim": claim(&version, &m(at + 12), &m(at + 4))}),
        ))
        .await;
    assert_eq!(ids(&baseline), [m(at + 14)]);
    c.route
        .entries
        .lock()
        .unwrap()
        .push(result(at + 15, "call_late", "sent"));
    let diverged = served
        .read(
            "05-claim-diverged-say-answered",
            page(json!({"limit": 5, "claim": claim(&version, &m(at + 14), &m(at + 5))})),
        )
        .await;
    assert_eq!(diverged["result"]["claim"], "diverged", "{diverged}");
    assert!(ids(&diverged).contains(&m(at + 13)), "{diverged}");

    // 06: prefrontal's record for a held `say` arrives late: the row comes
    // back whole, with the record's reply.
    let record = DisplayWrite::new(
        reserved("broca"),
        "ck_say",
        DisplayKind::Message,
        pin("say", 's'),
        json!({"v": 1, "reply_to": m(at + 2), "media": [], "level": "chat"})
            .as_object()
            .unwrap()
            .clone(),
    );
    c.store.write(&reserved("prefrontal"), &record, 1).unwrap();
    let _ = c.hints.send(crate::display::app::Hint::Write {
        runner: "broca".into(),
        call_key: "ck_say".into(),
    });
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    let late = served
        .read(
            "06-claim-diverged-late-record",
            // A window wide enough to hold the changed row again.
            page(json!({"limit": 10, "claim": claim(&version, &m(at + 14), &m(at + 5))})),
        )
        .await;
    assert_eq!(late["result"]["claim"], "diverged", "{late}");
    let held = late["result"]["resync"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["messageId"] == m(at + 5))
        .cloned()
        .unwrap_or_else(|| panic!("{late}"));
    assert_eq!(
        held["parts"][0]["replyTo"]["messageId"],
        m(at + 2),
        "{held}"
    );

    // 07: a claimant that last read the oldest message, far behind the
    // window this read goes through, accepts outside_window.
    let mut outside = page(json!({"limit": 5, "claim": claim(&version, &m(1), &m(1))}));
    outside["params"]["acceptClaims"] = json!(["outside_window"]);
    let outside = served.read("07-claim-outside-window", outside).await;
    assert_eq!(outside["result"]["claim"], "outside_window", "{outside}");

    // 08: a full claim sent with the view names the full version: expired.
    let full = version.as_str().unwrap().trim_end_matches(":simple:1");
    let expired = served
        .read(
            "08-full-claim-on-the-view",
            page(json!({"limit": 5, "claim": claim(&json!(full), &m(at + 14), &m(at + 5))})),
        )
        .await;
    assert_eq!(expired["result"]["claim"], "cursor_expired", "{expired}");

    // 09: the history is rewritten: the claim expires with the new page.
    c.route.history.fetch_add(1, Ordering::SeqCst);
    let rewritten = served
        .read(
            "09-claim-after-rewrite",
            page(json!({"limit": 5, "claim": claim(&version, &m(at + 14), &m(at + 5))})),
        )
        .await;
    assert_eq!(
        rewritten["result"]["claim"], "cursor_expired",
        "{rewritten}"
    );
    assert_ne!(rewritten["result"]["resync"]["version"], version);

    // 10: one message by position reads the full stream: the view is refused.
    let refused = served
        .refused("10-exact-read-refused", page(json!({"messageOrdinal": 1})))
        .await;
    assert!(refused.contains("invalid_request"), "{refused}");

    // 11: a chat with messages, none of which the view shows.
    let quiet = chat(true, (1..=30).map(prose).collect()).await;
    served.served.serve_with(quiet.app.clone());
    let nothing = served
        .read("11-nothing-visible", page(json!({"limit": 5})))
        .await;
    assert!(ids(&nothing).is_empty());
    assert_eq!(nothing["result"]["emptyReason"], "nothing_visible");

    // 12: a chat whose agent has no message tool: its `say` calls are tool
    // calls, so only the person's rows, the asks and the answer show.
    let plain = chat(false, entries()).await;
    served.served.serve_with(plain.app.clone());
    let without = served
        .read("12-no-message-tool", page(json!({"limit": 5})))
        .await;
    let rows = without["result"]["messages"].as_array().unwrap();
    assert!(
        rows.iter().all(|row| !row["parts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| part["type"] == "message")),
        "{without}"
    );

    // 13: the asks' ids, from prefrontal's records of the two calls that
    // filed them, read beside the page as any tool record is.
    served.served.serve_with(c.app.clone());
    let ask_view = crate::display::store::ViewState::View {
        manifest: super::tool_display_samples::view(
            "prefrontal.ask",
            json!({"action": "code", "askId": "id"}),
            super::tool_display_samples::BASH_ROW,
            super::tool_display_samples::BASH_PAGE,
            0,
        ),
        input_schema: super::tool_display_samples::input_schema(
            "question",
            json!({"question": {"type": "string"}}),
        ),
    };
    c.store
        .keep_view("prefrontal", &pin("ask", 'a'), &ask_view, "1.0", 1)
        .unwrap();
    for (call_key, ask) in [
        ("ck_ask_open", "ask_1111111111111111"),
        ("ck_ask_done", "ask_2222222222222222"),
    ] {
        let record = DisplayWrite::new(
            reserved("broca"),
            call_key,
            DisplayKind::Record,
            pin("ask", 'a'),
            json!({"action": "create", "askId": ask})
                .as_object()
                .unwrap()
                .clone(),
        );
        c.store.write(&reserved("prefrontal"), &record, 1).unwrap();
    }
    let records = served
        .read(
            "13-ask-records",
            json!({"method": crate::display::app::RECORDS, "params": {"session": SESSION,
                "harness": "broca", "callKeys": ["ck_ask_open", "ck_ask_done"]}}),
        )
        .await;
    let calls = &records["result"]["calls"];
    assert_eq!(
        calls["ck_ask_open"]["record"]["askId"], "ask_1111111111111111",
        "{records}"
    );
    assert_eq!(
        calls["ck_ask_done"]["record"]["askId"],
        "ask_2222222222222222"
    );

    served.served.check();
}

/// A busy chat read in both views keeps the Simple copies its claimants were
/// served apart from the full ones: a Simple claim from before 300 full reads
/// still finds its copy, so a `say` record that arrived since answers
/// `diverged` from the row it changed.
#[tokio::test]
async fn a_simple_copy_outlasts_full_reads() {
    late_record_after_full_reads(false).await;
}

/// A record written for a `say` long after its row was kept, and past the
/// newest messages a read checks again, drops the kept rows, so the row is
/// read again with the record.
#[tokio::test]
async fn a_late_record_reaches_a_row_kept_long_ago() {
    late_record_after_full_reads(true).await;
}

/// A Simple claimant reads, 300 full reads follow with a message each, and
/// then prefrontal's record for the first `say` arrives. With `refreshed`,
/// another Simple reader reads the newest rows before the record does, so
/// only dropping the kept rows brings the row back with its record.
async fn late_record_after_full_reads(refreshed: bool) {
    let at = 2 + FILLER;
    let c = chat(true, entries()).await;
    let mut served = Sample {
        served: Served::new(SET, c.app.clone()),
        corr: 0,
    };
    let simple = served.quiet(page(json!({"limit": 10}))).await;
    let version = simple["result"]["version"].clone();
    let rows = simple["result"]["messages"].as_array().unwrap();
    let newest = rows.last().unwrap()["messageId"]
        .as_str()
        .unwrap()
        .to_owned();
    let floor = rows[0]["messageId"].as_str().unwrap().to_owned();

    let full = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": SESSION, "harness": "broca", "limit": 5}});
    for step in 0..300 {
        c.route.entries.lock().unwrap().push(prose(at + 11 + step));
        served.quiet(full.clone()).await;
    }
    if refreshed {
        served.quiet(page(json!({"limit": 10}))).await;
    }

    let record = DisplayWrite::new(
        reserved("broca"),
        "ck_say",
        DisplayKind::Message,
        pin("say", 's'),
        json!({"v": 1, "reply_to": "m1304", "media": [], "level": "chat"})
            .as_object()
            .unwrap()
            .clone(),
    );
    c.store.write(&reserved("prefrontal"), &record, 1).unwrap();
    let _ = c.hints.send(crate::display::app::Hint::Write {
        runner: "broca".into(),
        call_key: "ck_say".into(),
    });
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    let claimed = served
        .quiet(page(
            json!({"limit": 10, "claim": claim(&version, &newest, &floor)}),
        ))
        .await;
    assert_eq!(claimed["result"]["claim"], "diverged", "{claimed}");
    assert_eq!(
        claimed["result"]["validThroughMessageId"], "m1306",
        "{claimed}"
    );
}
