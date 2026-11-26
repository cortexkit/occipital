//! Broca's `run.status` bodies with step liveness (runner contract 0.10.1
//! §6.7, `tests/data/broca-step-liveness-1126/`, copied unchanged from Broca's
//! real serve connection). An active run's `step` gains `phase`, deadlines and
//! the time of its last event, and `role.describe` gains `step_liveness`.
//! Occipital reads only a run's `state` (Stop, to tell an ended run from a
//! cancelled one) and its `error` (an unsent send's reason), so every body
//! reads as before, a phase it doesn't know included.

use super::*;
use std::path::Path;

/// One of Broca's recorded bodies, as its serve connection answered.
fn recorded(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/broca-step-liveness-1126")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[tokio::test]
async fn stop_reads_each_step_liveness_body_by_its_state() {
    let f = fixture(Kind::Head);
    f.runner.captured.lock().unwrap().insert(
        "role.describe",
        json!({"result": {"capabilities": ["run_cancel", "delivery_order", "run_cancel_by_interrupt",
            "run_status", "step_liveness"]}}),
    );
    f.runner
        .captured
        .lock()
        .unwrap()
        .insert("run.cancel", json!({"result": "not_active"}));
    // A phase a later contract adds reads like any other.
    let mut later = recorded("streaming.json");
    later["result"]["step"]["phase"] = json!("a_later_phase");
    let bodies = [
        "preparing.json",
        "awaiting_response.json",
        "streaming.json",
        "retrying.json",
        "running_tools.json",
        "completed.json",
    ]
    .map(|name| (name.to_owned(), recorded(name)))
    .into_iter()
    .chain([("a later phase".to_owned(), later)]);
    for (n, (name, body)) in bodies.enumerate() {
        f.runner.captured.lock().unwrap().insert("run.status", body);
        let stop = json!({"session": "s", "runId": "r-old",
            "clientRequestId": format!("stop-{n}")});
        assert_eq!(
            f.sender.stop(stop).await.unwrap(),
            json!({"state": "not_active", "runId": "r-old"}),
            "{name}"
        );
    }
}
