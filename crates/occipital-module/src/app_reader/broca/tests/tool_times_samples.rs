//! The tool-times samples the apps pin, `phone-reader-tool-times-v3`. Its README
//! says what each file shows and what changed from v2. The replies come from a test session on
//! Thalamus's OpenCode 2 runner, which serves the call times beside each read
//! entry, so the times are fixed and the set is reproducible; `served_samples`
//! says how the set is checked and rewritten. v1 was recorded the same way on
//! the runner's OpenCode 1 sessions, which no longer exist.

use super::served_samples::{decoded, Served};
use super::*;

/// Routes to the test session whatever its harness: here, OpenCode 2.
struct AnyHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for AnyHarness {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "opencode2");
        Ok(self.0.clone())
    }
}

/// An OpenCode 2 turn asking for two commands, then the answer that ran them,
/// with the times the runner serves beside the answer.
fn opencode_turn() -> Vec<Value> {
    let call = |id: &str, tool: &str, state: Value| json!({"type": "tool", "id": id, "name": tool, "state": state});
    let output = |text: &str| json!([{"type": "text", "text": text}]);
    vec![
        json!({"mid": "msg_u", "ordinal": 1, "written_at": 1_793_500_000_000_u64, "message": {
            "id": "msg_u", "type": "user", "time": {"created": 1_793_500_000_000_u64},
            "text": "List the folder, then read the missing file."}}),
        json!({"mid": "msg_a", "ordinal": 2, "written_at": 1_793_500_000_400_u64,
        "completed_at": 1_793_500_009_000_u64,
        "tool_times": [
            // Ran and answered: both times.
            {"tool_call_id": "call_ls", "started_at": 1_793_500_001_000_u64, "finished_at": 1_793_500_001_250_u64},
            // Failed: both times, with the error.
            {"tool_call_id": "call_read", "started_at": 1_793_500_002_000_u64, "finished_at": 1_793_500_002_040_u64}
            // Served without times: neither member appears.
        ],
        "message": {"id": "msg_a", "type": "assistant",
            "time": {"created": 1_793_500_000_400_u64, "completed": 1_793_500_009_000_u64},
            "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
            "content": [
                call("call_ls", "bash", json!({"status": "completed", "input": {"command": "ls"},
                    "content": output("README.md\n")})),
                call("call_read", "read", json!({"status": "error", "input": {"path": "missing.txt"},
                    "error": {"type": "unknown", "message": "file not found: missing.txt"}})),
                call("call_date", "bash", json!({"status": "completed", "input": {"command": "date"},
                    "content": output("Sat\n")})),
                {"type": "text", "text": "Done."}
            ]}}),
    ]
}

#[tokio::test]
async fn the_tool_times_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = opencode_turn();
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                THALAMUS_RUNNER_MODULE_ID,
            ))))),
            Arc::new(AnyHarness(route)),
        ),
    )));
    let mut served = Served::new("phone-reader-tool-times-v3", app);
    let session = json!({"session": "ses_head", "harness": "opencode2"});

    let mut page_request =
        json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": session});
    page_request["params"]["limit"] = json!(50);
    let page = served.read(1, &page_request).await;
    let parts = &decoded(&page)["result"]["messages"][1]["parts"];
    let times = |parts: &Value| -> Vec<(Value, Value)> {
        parts
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "tool")
            .map(|part| (part["startedAtMs"].clone(), part["finishedAtMs"].clone()))
            .collect()
    };
    let expected = vec![
        (json!(1_793_500_001_000_u64), json!(1_793_500_001_250_u64)),
        (json!(1_793_500_002_000_u64), json!(1_793_500_002_040_u64)),
        (Value::Null, Value::Null),
    ];
    assert_eq!(times(parts), expected, "{parts}");
    served.keep("01-opencode2-newest-page", &page_request, page);

    // The exact read of the answer carries the same times beside the whole output.
    let mut exact_request =
        json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": session});
    exact_request["params"]["messageOrdinal"] = json!(2);
    let exact = served.read(2, &exact_request).await;
    let parts = &decoded(&exact)["result"]["messages"][0]["parts"];
    assert_eq!(times(parts), expected, "{parts}");
    served.keep("02-opencode2-exact-read", &exact_request, exact);

    served.check();
}
