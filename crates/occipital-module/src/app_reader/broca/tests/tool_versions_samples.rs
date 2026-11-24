//! The sample set `phone-reader-tool-versions-v2`: a call whose tool carries a
//! version (tool-provider 0.15.17), as the apps read it. Its README says what
//! each file shows. The runner pins the call with a `tp2` pin, which names the
//! tool's major version and never its minor or patch, so a compatible update
//! keeps the pin and its view grows under it (tool-display 0.1.14).

use super::served_samples::{decoded, Served};
use super::tool_display_samples::{reserved, template, AnyHarness, BASH_PAGE, BASH_ROW};
use super::*;
use crate::display::app::{ToolDisplay, RECORDS, VIEWS};
use crate::display::store::{DisplayStore, ViewState};
use cortexkit_role_tool_display::{DisplayKind, DisplayWrite};
use cortexkit_role_tool_provider::catalog::ToolVersion;
use cortexkit_role_tool_provider::views::{RecordSchema, ViewManifest, ViewTool};
use cortexkit_test_support::ScratchDir;

const SET: &str = "phone-reader-tool-versions-v2";

/// The command tool at `version`. A minor or patch update changes neither
/// its arguments' structure nor its pin.
fn bash(version: ToolVersion) -> ViewTool {
    ViewTool::new_versioned(
        "bash",
        json!({"type": "object", "properties": {"command": {"type": "string"}},
            "required": ["command"]}),
        version,
    )
}

/// The command's view at `revision`, with the record schema it names.
fn view(revision: u64, schema: Value, tool: ViewTool) -> ViewState {
    let schema: RecordSchema = serde_json::from_value(schema).unwrap();
    let manifest = ViewManifest::new(
        schema,
        vec![],
        json!({"row": template("tool_row", BASH_ROW), "page": template("tool_page", BASH_PAGE)}),
    )
    .with_template_revision(revision)
    .with_tool(tool.clone());
    ViewState::View {
        manifest: serde_jcs::to_vec(&manifest).unwrap(),
        input_schema: serde_jcs::to_vec(&tool.input_schema).unwrap(),
    }
}

fn write(pin: &str, kind: DisplayKind, body: Value) -> DisplayWrite {
    DisplayWrite::new(
        reserved(THALAMUS_RUNNER_MODULE_ID),
        "ck_test",
        kind,
        pin,
        body.as_object().unwrap().clone(),
    )
}

#[tokio::test]
async fn the_tool_versions_samples_are_what_the_reader_serves() {
    let dir = ScratchDir::new("occipital-tool-versions-samples");
    let descriptor = crate::store::tests::descriptor(&dir);
    let store = Arc::new(DisplayStore::open(&descriptor, dir.path()).unwrap());
    let first = bash(ToolVersion::new(2, 0, 0));
    let tp2 = first.schema_pin().unwrap();
    assert!(tp2.starts_with("tp2:bash:") && tp2.ends_with(":2"), "{tp2}");
    assert_eq!(bash(ToolVersion::new(2, 1, 3)).schema_pin().unwrap(), tp2);
    store
        .keep_view(
            "aft",
            &tp2,
            &view(
                1,
                json!({"name": "aft.bash", "revision": 1, "fields": {"exitCode": "integer"}}),
                first,
            ),
            "1.0",
            1,
        )
        .unwrap();
    // The provider writes the last output lines before the view that declares
    // them is kept: Occipital drops the field until then.
    let tail = json!([{"text": "test result: ok. 12 passed; 0 failed", "tone": "muted"}]);
    let completion = write(
        &tp2,
        DisplayKind::Completion,
        json!({"durationMs": 61_200, "outcome": "success", "exitCode": 0, "tail": tail}),
    );
    store.write(&reserved("aft"), &completion, 1).unwrap();
    let slot = crate::display::call_pins::CallPinSlot::default();
    let (call_pins, _task) = crate::display::call_pins::start(store.clone(), None);
    let _ = slot.set(call_pins);
    let hints = tokio::sync::broadcast::channel(crate::display::app::HINTS).0;
    let tool = Arc::new(ToolDisplay::new(store.clone(), None, hints));

    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = vec![json!({"mid": "msg_a", "ordinal": 1,
        "tool_times": [{"tool_call_id": "call_test", "started_at": 1_793_500_001_000_u64,
            "finished_at": 1_793_500_062_200_u64, "call_key": "ck_test", "schema_pin": tp2}],
        "message": {"id": "msg_a", "type": "assistant",
            "time": {"created": 1_793_500_000_400_u64, "completed": 1_793_500_063_000_u64},
            "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
            "content": [{"type": "tool", "id": "call_test", "name": "bash",
                "state": {"status": "completed", "input": {"command": "cargo test"},
                    "content": [{"type": "text", "text": "test result: ok\n"}]}}]}})];
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                THALAMUS_RUNNER_MODULE_ID,
            ))))),
            Arc::new(AnyHarness(route)),
        )
        .with_call_pins(slot),
    )));
    app.set_tool_display(tool);
    let mut served = Served::new(SET, app);
    let session = |method: &str, extra: Value| {
        let mut params = json!({"session": "ses_head", "harness": "opencode2"});
        for (name, value) in extra.as_object().unwrap() {
            params[name] = value.clone();
        }
        json!({"method": method, "params": params})
    };

    // 01: the page names the call's `tp2` pin as its `schemaPin`.
    let page_request = session(super::super::super::TRANSCRIPT_PAGE, json!({"limit": 50}));
    let page = served.read(1, &page_request).await;
    assert_eq!(
        decoded(&page)["result"]["messages"][0]["parts"][0]["schemaPin"],
        tp2.as_str()
    );
    served.keep("01-page", &page_request, page);
    for _ in 0..1_000 {
        if store
            .call_pin(THALAMUS_RUNNER_MODULE_ID, "ck_test")
            .unwrap()
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    // 02: the view for that pin, at revision 1.
    let views_request = json!({"method": VIEWS, "params": {"pins": [tp2]}});
    let views = served.read(2, &views_request).await;
    assert_eq!(
        decoded(&views)["result"]["views"][&tp2]["templateRevision"],
        1
    );
    served.keep("02-views", &views_request, views);

    // 03: the completion without the field its view doesn't declare yet.
    let records_request = session(RECORDS, json!({"callKeys": ["ck_test"]}));
    let records = served.read(3, &records_request).await;
    assert_eq!(
        decoded(&records)["result"]["calls"]["ck_test"]["completion"],
        json!({"durationMs": 61_200, "outcome": "success", "exitCode": 0})
    );
    served.keep("03-records", &records_request, records);

    // The provider's next minor version declares the lines. Its view grows
    // under the same pin: a higher template and record revision, every field
    // it had, and the new one. The fetcher replaces the kept view with it
    // (`display::views`); this stands in for that fetch.
    store
        .replace_view(
            "aft",
            &tp2,
            &view(
                2,
                json!({"name": "aft.bash", "revision": 2, "fields": {"exitCode": "integer"},
                    "completion_fields": {"tail": "text_lines"}}),
                bash(ToolVersion::new(2, 1, 0)),
            ),
            "1.1",
            2,
        )
        .unwrap();

    // 04: an app holding revision 1 gets the grown view.
    let grown_request = json!({"method": VIEWS, "params": {"pins": [tp2], "held": {&tp2: 1}}});
    let grown = served.read(4, &grown_request).await;
    assert_eq!(
        decoded(&grown)["result"]["views"][&tp2]["recordSchema"]["completion_fields"],
        json!({"tail": "text_lines"})
    );
    served.keep("04-views-grown", &grown_request, grown);

    // 05: the same stored completion, now with its lines.
    let records = served.read(5, &records_request).await;
    assert_eq!(
        decoded(&records)["result"]["calls"]["ck_test"]["completion"]["tail"],
        tail
    );
    served.keep("05-records-grown", &records_request, records);
    served.check();
}
