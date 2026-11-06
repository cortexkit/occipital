//! The sample set the apps pin for a chat whose history the runner is still
//! taking in, `phone-reader-history-intake-v1`; its README says what each file
//! shows.
//!
//! When a head moves onto Thalamus's OpenCode 2 runner (module
//! `thalamus-harness`), the runner takes in the history the host already held,
//! oldest first, and its head says `history_intake` until it's done. Meanwhile
//! the newest page holds the newest message taken in so far, an old one. The
//! details and the newest page say `historyIntake: "running"` then, and leave
//! it out once the intake ends. The text is invented. `served_samples` says how
//! the set is checked and rewritten.

use super::served_samples::{decoded, Served};
use super::*;

/// Routes to the test session whatever its harness: here, OpenCode 2.
struct AnyHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for AnyHarness {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// The session the test resolver knows (`ByHarness`).
const SESSION: &str = "ses_head";

/// A stored OpenCode 2 message, the person's on odd ordinals and the head's
/// replies on even ones.
fn message(ordinal: u64, text: &str) -> Value {
    let id = format!("msg_{ordinal:02}");
    let at = 1_790_000_000_000_u64 + ordinal * 60_000;
    let body = if ordinal % 2 == 1 {
        json!({"id": id, "type": "user", "time": {"created": at}, "text": text})
    } else {
        json!({"id": id, "type": "assistant",
            "time": {"created": at, "completed": at + 4_000},
            "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
            "content": [{"type": "text", "text": text}]})
    };
    json!({"mid": id, "ordinal": ordinal, "written_at": at + 5_000, "message": body})
}

/// The chat's whole history on the host, oldest first.
fn history() -> Vec<Value> {
    [
        "Which herbs grow well on a north-facing sill?",
        "Mint, parsley and chives cope with less light than most.",
        "How often should I water the mint?",
        "When the top centimetre of soil is dry, usually every two or three days.",
    ]
    .iter()
    .enumerate()
    .map(|(index, text)| message(index as u64 + 1, text))
    .collect()
}

fn intake(route: &FakeRoute, value: Option<&str>) {
    let mut head = route.head_extra.lock().unwrap();
    match value {
        Some(value) => head.insert("history_intake".into(), json!(value)),
        None => head.remove("history_intake"),
    };
}

#[tokio::test]
async fn the_history_intake_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    // Half the history is taken in so far.
    *route.entries.lock().unwrap() = history()[..2].to_vec();
    *route.run_state.lock().unwrap() = json!({"run_id": "r0", "state": "completed"});
    intake(&route, Some("running"));
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                OPENCODE_RUNNER_MODULE_ID,
            ))))),
            Arc::new(AnyHarness(route.clone())),
        ),
    )));
    let mut served = Served::new("phone-reader-history-intake-v1", app);
    let session = json!({"session": SESSION, "harness": "opencode2"});
    let details = json!({"method": super::super::super::DETAILS, "params": session});
    let mut page_params = session.clone();
    page_params["limit"] = json!(20);
    let page = json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": page_params});

    let loading = served.read(1, &details).await;
    assert_eq!(decoded(&loading)["result"]["historyIntake"], "running");
    served.keep("01-details-intake-running", &details, loading);

    let partial = served.read(2, &page).await;
    let partial_page = decoded(&partial)["result"].clone();
    assert_eq!(partial_page["historyIntake"], "running");
    assert_eq!(partial_page["messages"].as_array().unwrap().len(), 2);
    served.keep("02-newest-page-intake-running", &page, partial);

    // A value the reader doesn't know still means the intake is running.
    intake(&route, Some("verifying"));
    let unknown = served.read(3, &details).await;
    assert_eq!(decoded(&unknown)["result"]["historyIntake"], "running");

    // The intake ends: the rest of the history is stored after what was there,
    // under the same page version, and the head stops naming the intake.
    *route.entries.lock().unwrap() = history();
    intake(&route, None);
    let whole = served.read(4, &page).await;
    let whole_page = decoded(&whole)["result"].clone();
    assert!(whole_page.get("historyIntake").is_none(), "{whole_page}");
    assert_eq!(whole_page["messages"].as_array().unwrap().len(), 4);
    assert_eq!(whole_page["version"], partial_page["version"]);
    assert_ne!(whole_page["activityToken"], partial_page["activityToken"]);
    served.keep("03-newest-page-intake-done", &page, whole);
    served.check();
}
