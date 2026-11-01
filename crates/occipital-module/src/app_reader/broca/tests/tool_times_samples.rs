//! The tool-times samples the apps pin, `phone-reader-tool-times-v1`. Its README
//! says what each file shows. The replies come from a test session on
//! Thalamus's OpenCode runner, whose stored tool parts carry the call times, so
//! the times are fixed and the set is reproducible; `served_samples` says how the
//! set is checked and rewritten.

use super::served_samples::{decoded, Served};
use super::*;

/// Routes to the test session whatever its harness: here, OpenCode.
struct AnyHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for AnyHarness {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "opencode");
        Ok(self.0.clone())
    }
}

/// An OpenCode turn asking for two commands, then the answer that ran them.
fn opencode_turn() -> Vec<Value> {
    vec![
        json!({"mid": "msg_u", "ordinal": 0, "message": {
            "info": {"id": "msg_u", "role": "user", "time": {"created": 1_793_500_000_000_u64}},
            "parts": [{"id": "prt_u0", "type": "text", "text": "List the folder, then read the missing file."}]}}),
        json!({"mid": "msg_a", "ordinal": 1, "message": {
        "info": {"id": "msg_a", "role": "assistant",
            "time": {"created": 1_793_500_000_400_u64, "completed": 1_793_500_009_000_u64},
            "providerID": "anthropic", "modelID": "claude-opus-5-5"},
        "parts": [
            {"id": "prt_a0", "type": "step-start"},
            // Ran and answered: both times.
            {"id": "prt_a1", "type": "tool", "tool": "bash", "callID": "call_ls",
                "state": {"status": "completed", "input": {"command": "ls"}, "output": "README.md\n",
                    "time": {"start": 1_793_500_001_000_u64, "end": 1_793_500_001_250_u64}}},
            // Failed: both times, with the error.
            {"id": "prt_a2", "type": "tool", "tool": "read", "callID": "call_read",
                "state": {"status": "error", "input": {"path": "missing.txt"},
                    "error": "file not found: missing.txt",
                    "time": {"start": 1_793_500_002_000_u64, "end": 1_793_500_002_040_u64}}},
            // Stored without times: neither member appears.
            {"id": "prt_a3", "type": "tool", "tool": "bash", "callID": "call_date",
                "state": {"status": "completed", "input": {"command": "date"}, "output": "Sat\n"}},
            {"id": "prt_a4", "type": "text", "text": "Done."},
            {"id": "prt_a5", "type": "step-finish"}
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
                OPENCODE_RUNNER_MODULE_ID,
            ))))),
            Arc::new(AnyHarness(route)),
        ),
    )));
    let mut served = Served::new("phone-reader-tool-times-v1", app);
    let session = json!({"session": "ses_head", "harness": "opencode"});

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
    served.keep("01-opencode-newest-page", &page_request, page);

    // The exact read of the answer carries the same times beside the whole output.
    let mut exact_request =
        json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": session});
    exact_request["params"]["messageOrdinal"] = json!(1);
    let exact = served.read(2, &exact_request).await;
    let parts = &decoded(&exact)["result"]["messages"][0]["parts"];
    assert_eq!(times(parts), expected, "{parts}");
    served.keep("02-opencode-exact-read", &exact_request, exact);

    served.check();
}
