//! The sample set the apps pin for a chat whose host program the runner has lost
//! touch with, `phone-reader-host-unknown-v1`; its README says what each file shows.
//!
//! A runner that keeps runs alive across its own restarts (runner contract
//! `restart_survival`) says `host: "unknown"` on its head while it can't tell
//! whether the session's host program is still running. The run stays active, but
//! nothing more arrives until the host is found, and Stop ends the run at once.
//! The details and the newest page pass it on as `host`, and leave it out once the
//! host is found. The text is invented. `served_samples` says how the set is
//! checked and rewritten.

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

/// A stored OpenCode 2 message: the person's question, then the head's reply in
/// progress when the runner restarted.
fn history() -> Vec<Value> {
    let at = 1_790_000_000_000_u64;
    let question = json!({"id": "msg_01", "type": "user", "time": {"created": at},
        "text": "Sort the photos by the day they were taken"});
    let reply = json!({"id": "msg_02", "type": "assistant", "time": {"created": at + 2_000},
        "model": {"id": "claude-opus-5-5", "providerID": "anthropic"},
        "content": [{"type": "text", "text": "Reading the dates from each file."}]});
    vec![
        json!({"mid": "msg_01", "ordinal": 1, "written_at": at + 1_000, "message": question}),
        json!({"mid": "msg_02", "ordinal": 2, "written_at": at + 3_000, "message": reply}),
    ]
}

fn host(route: &FakeRoute, value: Option<&str>) {
    let mut head = route.head_extra.lock().unwrap();
    match value {
        Some(value) => head.insert("host".into(), json!(value)),
        None => head.remove("host"),
    };
}

#[tokio::test]
async fn the_host_unknown_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = history();
    *route.run_state.lock().unwrap() = json!({"run_id": "r1", "state": "active"});
    host(&route, Some("unknown"));
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                THALAMUS_RUNNER_MODULE_ID,
            ))))),
            Arc::new(AnyHarness(route.clone())),
        ),
    )));
    let mut served = Served::new("phone-reader-host-unknown-v1", app);
    let session = json!({"session": SESSION, "harness": "opencode2"});
    let details = json!({"method": super::super::super::DETAILS, "params": session});
    let mut page_params = session.clone();
    page_params["limit"] = json!(20);
    let page = json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": page_params});

    let lost = served.read(1, &details).await;
    let lost_details = decoded(&lost)["result"].clone();
    assert_eq!(lost_details["host"], "unknown", "{lost_details}");
    assert_eq!(lost_details["lastRun"]["state"], "active", "{lost_details}");
    served.keep("01-details-host-unknown", &details, lost);

    let lost_page = served.read(2, &page).await;
    let lost_page_body = decoded(&lost_page)["result"].clone();
    assert_eq!(lost_page_body["host"], "unknown");
    served.keep("02-newest-page-host-unknown", &page, lost_page);

    // A spelling the reader doesn't know is passed on as the runner gives it.
    host(&route, Some("reattaching"));
    let other = served.read(3, &details).await;
    assert_eq!(decoded(&other)["result"]["host"], "reattaching");

    // The host is found and the run goes on: the head stops naming it, which
    // changes the facts and so `activityToken`, under the same page version.
    host(&route, None);
    let found = served.read(4, &details).await;
    let found_details = decoded(&found)["result"].clone();
    assert!(found_details.get("host").is_none(), "{found_details}");
    assert_eq!(found_details["lastRun"]["state"], "active");
    served.keep("03-details-host-found", &details, found);

    let found_page = served.read(5, &page).await;
    let found_page_body = decoded(&found_page)["result"].clone();
    assert!(found_page_body.get("host").is_none(), "{found_page_body}");
    assert_eq!(found_page_body["version"], lost_page_body["version"]);
    assert_ne!(
        found_page_body["activityToken"],
        lost_page_body["activityToken"]
    );
    served.keep("04-newest-page-host-found", &page, found_page);
    served.check();
}
