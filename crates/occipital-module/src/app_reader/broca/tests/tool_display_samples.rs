//! The tool display samples the apps pin, `phone-reader-tool-display-v4`. Its
//! README says what each file shows and what changed from v3. A test OpenCode 2 session on Thalamus's
//! runner serves four calls with their keys and pins; the display store holds
//! the views and writes a provider would have sent. `served_samples` says how
//! the set is checked and rewritten. v2 was recorded the same way on the
//! runner's OpenCode 1 sessions, which no longer exist, and v1 differs from v2
//! only in the tool catalog digest its templates name.

use super::served_samples::{decoded, Served};
use super::*;
use crate::display::app::{ToolDisplay, DETAIL, RECORDS, RECORDS_WATCH, TEMPLATE, VIEWS};
use crate::display::store::{DisplayStore, ViewState};
use cortexkit_role_tool_display::{DisplayKind, DisplayWrite};
use cortexkit_role_tool_provider::views::{RecordSchema, ViewManifest};
use cortexkit_test_support::ScratchDir;
use subc_protocol::{FrameType, Principal};

/// Routes to the test session whatever its harness: here, OpenCode 2.
struct AnyHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for AnyHarness {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// A pin whose schema digest is `digit` 64 times, a lowercase hex digit.
fn pin(tool: &str, digit: char) -> String {
    format!("tp1:{tool}:{}:1", digit.to_string().repeat(64))
}

fn reserved(module: &str) -> Principal {
    Principal::Reserved {
        module_id: module.into(),
    }
}

/// The tool catalog digest the sample templates name: the one the tool catalog
/// had before the narrow Slot width, now an alias of the current digest. The
/// set was recorded under it, and a provider's template naming it must keep
/// passing, so the samples stay as recorded.
const SAMPLE_TOOL_CATALOG: &str =
    "sha256:9a23139f4f1f48d6ed366a4a48b9221b1e5ac0ccd87473c0b241b8a37a876fbf";

fn template(profile: &str, source: &str) -> Value {
    json!({
        "schema": "occipital.template/1",
        "profile": profile,
        "executionDialect": "1.0",
        "catalog": {"version": "1.1", "digest": SAMPLE_TOOL_CATALOG},
        "reads": [],
        "source": source,
    })
}

/// A provider's view: its record schema and its row and page templates.
fn view(name: &str, fields: Value, row: &str, page: &str, revision: u64) -> Vec<u8> {
    let schema: RecordSchema =
        serde_json::from_value(json!({"name": name, "revision": 1, "fields": fields})).unwrap();
    let manifest = ViewManifest::new(
        schema,
        vec![],
        json!({"row": template("tool_row", row), "page": template("tool_page", page)}),
    )
    .with_template_revision(revision);
    serde_jcs::to_vec(&manifest).unwrap()
}

fn input_schema(required: &str, properties: Value) -> Vec<u8> {
    serde_jcs::to_vec(&json!({"type": "object", "properties": properties, "required": [required]}))
        .unwrap()
}

const BASH_ROW: &str = "<Card appearance=\"clear\" direction=\"row\" gap=\"s\">\n<InlineCode code={input.command} />\n<Slot width=\"short\" needs={view.completion ?? view.record}>\n<Badge text={(view.completion ?? view.record).exitCode == 0 ? \"Done\" : \"Failed\"} />\n</Slot>\n</Card>\n";
const BASH_PAGE: &str = "<TextContent text={view.output ?? \"\"} />\n";
const EDIT_ROW: &str = "<Card appearance=\"clear\" direction=\"row\" gap=\"s\">\n<InlineCode code={input.path} />\n<Slot width=\"short\" needs={view.record}>\n<TextContent text={\"+\" + Text(view.record.added) + \" \u{2212}\" + Text(view.record.removed)} />\n</Slot>\n</Card>\n";
const EDIT_PAGE: &str = "<TextContent text={view.detail.diff ?? \"\"} />\n";

/// An OpenCode 2 answer with four calls, each served with its key and pin: a
/// command, an edit, a read whose provider has no view, and a call two
/// modules serve views for.
fn answer() -> Vec<Value> {
    let call = |id: &str, tool: &str, input: Value| {
        json!({"type": "tool", "id": id, "name": tool,
            "state": {"status": "completed", "input": input,
                "content": [{"type": "text", "text": "ok\n"}]}})
    };
    let times = |id: &str, key: &str, pin: String, at: u64| {
        json!({"tool_call_id": id, "started_at": at, "finished_at": at + 40,
            "call_key": key, "schema_pin": pin})
    };
    vec![json!({"mid": "msg_a", "ordinal": 1,
    "tool_times": [
        times("call_ls", "ck_ls", pin("bash", 'b'), 1_793_500_001_000),
        times("call_edit", "ck_edit", pin("edit", 'e'), 1_793_500_002_000),
        times("call_read", "ck_read", pin("read", 'a'), 1_793_500_003_000),
        times("call_two", "ck_two", pin("twice", 'd'), 1_793_500_004_000)
    ],
    "message": {"id": "msg_a", "type": "assistant",
        "time": {"created": 1_793_500_000_400_u64, "completed": 1_793_500_009_000_u64},
        "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
        "content": [
            call("call_ls", "bash", json!({"command": format!("ls {}", "-l ".repeat(700))})),
            call("call_edit", "edit", json!({"path": "src/main.rs", "old": "a", "new": "b"})),
            call("call_read", "read", json!({"path": "README.md"})),
            call("call_two", "twice", json!({"x": 1})),
            {"type": "text", "text": "Done."}
        ]}})]
}

/// The views and writes a provider would have sent for those calls.
fn seed(store: &DisplayStore) {
    let bash_input = input_schema("command", json!({"command": {"type": "string"}}));
    let edit_input = input_schema(
        "path",
        json!({"path": {"type": "string"}, "old": {"type": "string"}, "new": {"type": "string"}}),
    );
    let kept = |writer: &str, pin: &str, state: ViewState| {
        store.keep_view(writer, pin, &state, "1.0", 1).unwrap();
    };
    let bash = ViewState::View {
        manifest: view(
            "aft.bash",
            json!({"exitCode": "integer"}),
            BASH_ROW,
            BASH_PAGE,
            1,
        ),
        input_schema: bash_input.clone(),
    };
    kept("aft", &pin("bash", 'b'), bash.clone());
    kept(
        "aft",
        &pin("edit", 'e'),
        ViewState::View {
            manifest: view(
                "aft.edit",
                json!({"added": "integer", "removed": "integer"}),
                EDIT_ROW,
                EDIT_PAGE,
                0,
            ),
            input_schema: edit_input,
        },
    );
    kept("aft", &pin("read", 'a'), ViewState::Unknown);
    kept("aft", &pin("twice", 'd'), bash.clone());
    kept("core", &pin("twice", 'd'), bash);

    let runner = reserved(OPENCODE_RUNNER_MODULE_ID);
    let write = |writer: &str, call: &str, kind: DisplayKind, pin: String, body: Value| {
        let write = DisplayWrite::new(
            runner.clone(),
            call,
            kind,
            pin,
            body.as_object().unwrap().clone(),
        );
        store.write(&reserved(writer), &write, 1).unwrap();
    };
    let done = json!({"durationMs": 40, "outcome": "success", "exitCode": 0});
    write(
        "aft",
        "ck_ls",
        DisplayKind::Record,
        pin("bash", 'b'),
        json!({"durationMs": 40, "outcome": "held"}),
    );
    write(
        "aft",
        "ck_ls",
        DisplayKind::Completion,
        pin("bash", 'b'),
        done.clone(),
    );
    // Another module's write for the same call: it serves no view of the
    // call's pin, so it never joins.
    write(
        "intruder",
        "ck_ls",
        DisplayKind::Record,
        pin("bash", 'b'),
        json!({"durationMs": 1, "outcome": "error", "reason": "forged"}),
    );
    write(
        "aft",
        "ck_edit",
        DisplayKind::Record,
        pin("edit", 'e'),
        json!({"durationMs": 12, "outcome": "success", "added": 1, "removed": 1}),
    );
    write(
        "aft",
        "ck_edit",
        DisplayKind::Detail,
        pin("edit", 'e'),
        json!({"diff": "@@ -1 +1 @@\n-a\n+b\n"}),
    );
    // Two modules serve a view of this call's pin and both wrote: neither joins.
    write(
        "aft",
        "ck_two",
        DisplayKind::Record,
        pin("twice", 'd'),
        done.clone(),
    );
    write(
        "core",
        "ck_two",
        DisplayKind::Record,
        pin("twice", 'd'),
        done,
    );
}

#[tokio::test]
async fn the_tool_display_samples_are_what_the_reader_serves() {
    let dir = ScratchDir::new("occipital-tool-display-samples");
    let descriptor = crate::store::tests::descriptor(&dir);
    let store = Arc::new(DisplayStore::open(&descriptor, dir.path()).unwrap());
    seed(&store);
    let slot = crate::display::call_pins::CallPinSlot::default();
    let (call_pins, _task) = crate::display::call_pins::start(store.clone(), None);
    let _ = slot.set(call_pins);
    let hints = tokio::sync::broadcast::channel(crate::display::app::HINTS).0;
    let tool = Arc::new(ToolDisplay::new(store.clone(), None, hints.clone()));

    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = answer();
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                OPENCODE_RUNNER_MODULE_ID,
            ))))),
            Arc::new(AnyHarness(route)),
        )
        .with_call_pins(slot),
    )));
    app.set_tool_display(tool);
    let mut served = Served::new("phone-reader-tool-display-v4", app);
    let session = json!({"session": "ses_head", "harness": "opencode2"});
    let request = |method: &str, extra: Value| {
        let mut params = session.clone();
        for (name, value) in extra.as_object().unwrap() {
            params[name] = value.clone();
        }
        json!({"method": method, "params": params})
    };

    // 01: the page names each call and its tool version; the command's row
    // input is shortened, the others are whole.
    let page_request = request(super::super::super::TRANSCRIPT_PAGE, json!({"limit": 50}));
    let page = served.read(1, &page_request).await;
    let parts = &decoded(&page)["result"]["messages"][0]["parts"];
    assert_eq!(parts[0]["callKey"], "ck_ls", "{parts}");
    assert_eq!(parts[0]["inputTruncatedFields"], json!(["/command"]));
    assert_eq!(
        parts[1]["input"],
        json!({"path": "src/main.rs", "old": "a", "new": "b"})
    );
    served.keep("01-page-with-call-keys", &page_request, page);
    for _ in 0..1_000 {
        if store
            .call_pin(OPENCODE_RUNNER_MODULE_ID, "ck_two")
            .unwrap()
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    // 02: a view with both templates' digests, a view of revision 0, a pin
    // whose provider has no view, one not fetched yet, and one two modules
    // serve.
    let all = [
        pin("bash", 'b'),
        pin("edit", 'e'),
        pin("read", 'a'),
        pin("new", 'c'),
        pin("twice", 'd'),
    ];
    let views_request = json!({"method": VIEWS, "params": {"pins": all}});
    let views = served.read(2, &views_request).await;
    let answers = &decoded(&views)["result"]["views"];
    let bash = &answers[pin("bash", 'b')];
    assert_eq!(bash["provider"], "aft", "{answers}");
    assert_eq!(bash["templateRevision"], 1);
    let row_digest = bash["row"].as_str().unwrap().to_owned();
    assert!(bash["page"].is_string());
    assert_eq!(answers[pin("read", 'a')], "unknown");
    assert_eq!(answers[pin("new", 'c')], "unavailable");
    assert_eq!(answers[pin("twice", 'd')], "unknown");
    served.keep("02-views", &views_request, views);

    // 03: a pin held at its kept revision is left out.
    let held_request = json!({"method": VIEWS, "params": {
        "pins": [pin("bash", 'b'), pin("edit", 'e')],
        "held": {pin("bash", 'b'): 1}}});
    let held = served.read(3, &held_request).await;
    let answers = decoded(&held)["result"]["views"].clone();
    assert!(answers.get(pin("bash", 'b')).is_none(), "{answers}");
    assert!(answers.get(pin("edit", 'e')).is_some());
    served.keep("03-views-held", &held_request, held);

    // 04: a template by its digest, in its canonical form.
    let template_request = json!({"method": TEMPLATE, "params": {"digest": row_digest}});
    let template = served.read(4, &template_request).await;
    let document = &decoded(&template)["result"]["template"];
    assert_eq!(
        occipital_registry::sha256_digest(&serde_jcs::to_vec(document).unwrap()),
        row_digest
    );
    served.keep("04-template", &template_request, template);

    // 05: records by call. The command has its record and completion, the
    // edit its record and a detail; the read, the twice-served call and an
    // unknown call are left out.
    let records_request = request(
        RECORDS,
        json!({"callKeys": ["ck_ls", "ck_edit", "ck_read", "ck_two", "ck_missing"]}),
    );
    let records = served.read(5, &records_request).await;
    let result = &decoded(&records)["result"];
    let calls = result["calls"].as_object().unwrap();
    assert_eq!(
        calls.keys().collect::<Vec<_>>(),
        ["ck_edit", "ck_ls"],
        "{result}"
    );
    assert_eq!(calls["ck_ls"]["record"]["outcome"], "held");
    assert_eq!(calls["ck_ls"]["completion"]["exitCode"], 0);
    assert_eq!(calls["ck_ls"]["detail"], false);
    assert_eq!(calls["ck_edit"]["detail"], true);
    assert_eq!(result["loadingWindowMs"], 5_000);
    served.keep("05-records", &records_request, records);

    // 06: a call's detail, read on tap.
    let detail_request = request(DETAIL, json!({"callKey": "ck_edit"}));
    let detail = served.read(6, &detail_request).await;
    assert_eq!(
        decoded(&detail)["result"]["detail"]["diff"],
        "@@ -1 +1 @@\n-a\n+b\n"
    );
    served.keep("06-detail", &detail_request, detail);

    // 07: the records watch says it is listening, then names a call of the
    // session that got a new write.
    let watch_request = request(RECORDS_WATCH, json!({}));
    served.send(7, &watch_request).await;
    let (ty, open) = served.next(7, Duration::from_secs(5)).await.unwrap();
    assert_eq!(ty, FrameType::StreamData);
    assert_eq!(decoded(&open), json!({"callKeys": []}));
    let late = |runner: &str, call_key: &str| {
        json!({
            "runner": reserved(runner),
            "call_key": call_key,
            "kind": "record",
            "schema_pin": pin("read", 'a'),
            "body": {"durationMs": 3, "outcome": "success"},
        })
    };
    // A call this session's pages never carried, and the same key under
    // another runner, aren't this session's: the notice names neither.
    for (runner, call_key) in [
        (OPENCODE_RUNNER_MODULE_ID, "ck_elsewhere"),
        ("broca", "ck_read"),
        (OPENCODE_RUNNER_MODULE_ID, "ck_read"),
    ] {
        crate::display::write(
            Some(&crate::display::State::Open(store.clone())),
            None,
            Some(&hints),
            Some(reserved("aft")),
            &late(runner, call_key),
        )
        .await;
    }
    let (_, hint) = served.next(7, Duration::from_secs(5)).await.unwrap();
    assert_eq!(decoded(&hint), json!({"callKeys": ["ck_read"]}));
    // A newer template revision kept for a pin.
    hints
        .send(crate::display::app::Hint::Pin(pin("bash", 'b')))
        .unwrap();
    let (_, revised) = served.next(7, Duration::from_secs(5)).await.unwrap();
    assert_eq!(decoded(&revised), json!({"pins": [pin("bash", 'b')]}));
    served.cancel(7);
    served.keep("07-records-watch-open", &watch_request, open);
    served.keep("08-records-watch-hint", &watch_request, hint);
    served.keep("09-records-watch-pins", &watch_request, revised);

    // 10 and 11: refusals.
    let many: Vec<String> = (0..65)
        .map(|n| format!("tp1:t{n}:{}:1", "0".repeat(64)))
        .collect();
    let too_many = json!({"method": VIEWS, "params": {"pins": many}});
    served.send(9, &too_many).await;
    let (ty, refused) = served.next(9, Duration::from_secs(5)).await.unwrap();
    assert_eq!(
        ty,
        FrameType::Error,
        "{}",
        String::from_utf8_lossy(&refused)
    );
    served.keep("10-views-too-many-pins", &too_many, refused);
    let unknown_template =
        json!({"method": TEMPLATE, "params": {"digest": format!("sha256:{}", "0".repeat(64))}});
    served.send(10, &unknown_template).await;
    let (ty, refused) = served.next(10, Duration::from_secs(5)).await.unwrap();
    assert_eq!(
        ty,
        FrameType::Error,
        "{}",
        String::from_utf8_lossy(&refused)
    );
    served.keep("11-template-not-found", &unknown_template, refused);

    served.check();
}
