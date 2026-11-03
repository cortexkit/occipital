//! The attached-file samples the apps pin, `phone-reader-files-v1`. Its README
//! says what each file shows. The chat is a test OpenCode 2 session whose two
//! messages are the runner contract's own stored bodies, from its
//! `display-opencode-v2.json` (see `occipital-app-reader/tests/display_profiles.rs`):
//! a message recorded from OpenCode 2.2.3 with text and an image, and the
//! contract's message holding an image alone. `served_samples` says how the set
//! is checked and rewritten.

use super::served_samples::{decoded, Served};
use super::*;

/// Routes to the test session, an OpenCode 2 one.
struct OpenCode2(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for OpenCode2 {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "opencode2");
        Ok(self.0.clone())
    }
}

/// The contract's stored body for its case `name`.
fn body(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../occipital-app-reader/tests/fixtures/llm-runner-0.8.0/display-opencode-v2.json");
    let set: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    set["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap()["body"]
        .clone()
}

#[tokio::test]
async fn the_attached_file_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = vec![
        json!({"mid": "msg_text_and_file", "ordinal": 1, "written_at": 1_793_365_553_098_u64,
            "message": body("2.2.3-seq-54")}),
        json!({"mid": "msg_file_alone", "ordinal": 2, "written_at": 1_793_365_560_000_u64,
            "message": body("synthetic-existing-image-ref")}),
    ];
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                OPENCODE_RUNNER_MODULE_ID,
            ))))),
            Arc::new(OpenCode2(route)),
        ),
    )));
    let mut served = Served::new("phone-reader-files-v1", app);
    let session = json!({"session": "ses_head", "harness": "opencode2"});
    // A page names each file; an exact read, which is for a message's whole
    // tool output, leaves files out, so this set holds the page alone.
    let file = |message: &Value| -> Value {
        let files: Vec<&Value> = message["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "file")
            .collect();
        assert_eq!(files.len(), 1, "{message}");
        files[0].clone()
    };
    let expected = json!({"type": "file", "partKey": "1", "partIndex": 1,
        "mime": "image/png", "filename": "pixel.png", "byteCount": 69});

    let mut page_request =
        json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": session});
    page_request["params"]["limit"] = json!(50);
    let page = served.read(1, &page_request).await;
    let messages = decoded(&page)["result"]["messages"].clone();
    assert_eq!(messages[0]["text"], "S0_TAIL_KEEP");
    assert_eq!(file(&messages[0]), expected);
    let mut alone = expected.clone();
    alone["partKey"] = json!("0");
    alone["partIndex"] = json!(0);
    assert_eq!(messages[1]["text"], "");
    assert_eq!(file(&messages[1]), alone);
    served.keep("01-files-newest-page", &page_request, page);
    served.check();
}
