//! A page built from a runner that declares `call_identity` names each call
//! and its tool version, and building it keeps the pin the runner stored, so
//! a later records read can join the call without trusting the app.

use super::*;
use cortexkit_test_support::ScratchDir;

/// Routes to the test session whatever its harness: here, OpenCode 2.
struct AnyHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for AnyHarness {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

const PIN: &str = "tp1:bash:839469d5e28de1286cbd75382e5329eb7002d401575fc6c02c4aa17694939b3f:1";

/// An OpenCode 2 answer whose one call the runner served with its key and pin,
/// and whose command is too long for a row.
fn answered_call(command: &str) -> Vec<Value> {
    vec![json!({"mid": "msg_a", "ordinal": 1,
        "tool_times": [{"tool_call_id": "call_ls", "started_at": 10, "finished_at": 20,
            "call_key": "ck_ls", "schema_pin": PIN}],
        "message": {"id": "msg_a", "type": "assistant", "time": {"created": 1_u64, "completed": 30_u64},
            "content": [{"type": "tool", "id": "call_ls", "name": "bash",
                "state": {"status": "completed", "input": {"command": command},
                    "content": [{"type": "text", "text": "x\n"}]}}]}})]
}

#[tokio::test]
async fn a_page_names_its_calls_and_keeps_their_pins() {
    let dir = ScratchDir::new("occipital-reader-call-pins");
    let descriptor = crate::store::tests::descriptor(&dir);
    let display =
        Arc::new(crate::display::store::DisplayStore::open(&descriptor, dir.path()).unwrap());
    let slot = crate::display::call_pins::CallPinSlot::default();
    let (call_pins, _task) = crate::display::call_pins::start(display.clone(), None);
    let _ = slot.set(call_pins);

    let route = FakeRoute::new();
    let command = "echo ".to_owned() + &"a".repeat(3_000);
    *route.entries.lock().unwrap() = answered_call(&command);
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
    let page = app
        .read(
            super::super::super::TRANSCRIPT_PAGE,
            json!({"session": "ses_head", "harness": "opencode2", "limit": 50}),
        )
        .await
        .unwrap();
    let part = &page["messages"][0]["parts"][0];
    assert_eq!(part["callKey"], "ck_ls", "{part}");
    assert_eq!(part["schemaPin"], PIN);
    assert_eq!(part["startedAtMs"], 10);
    // The row's input is the arguments object with the command shortened.
    assert_eq!(part["inputTruncatedFields"], json!(["/command"]));
    let input = part["input"]["command"].as_str().unwrap();
    assert!(
        input.starts_with("echo aaa") && input.ends_with('…'),
        "{input}"
    );
    assert!(part["input"].to_string().len() <= occipital_app_reader::tool_input::INPUT_BYTES);

    for _ in 0..1_000 {
        if display
            .call_pin(OPENCODE_RUNNER_MODULE_ID, "ck_ls")
            .unwrap()
            .is_some()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    assert_eq!(
        display
            .call_pin(OPENCODE_RUNNER_MODULE_ID, "ck_ls")
            .unwrap(),
        Some(("ses_head".into(), PIN.into()))
    );
}
