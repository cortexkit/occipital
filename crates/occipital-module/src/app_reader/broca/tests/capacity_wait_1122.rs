//! Agent Runner's capacity wait (`tests/data/runner-capacity-wait-1122`, recorded
//! on OpenCode 2): routing refused a run's next model request with
//! `concurrency_full`, and the run waits for routing's reset instead of failing.
//! Only `run.status.paused.json` reaches a path of this reader; the routing and
//! auth-plugin bodies beside it are between the runner and routing.
//!
//! The run's wait reaches the apps as `lastRun` on every chat read, from the
//! last-run state the runner's `session.head` reports, which carries the same
//! `awaits` object as the run's status. A run that ends in an error while it waits names the wait as its
//! provider code, which the apps show as a wait, never as a failure.

use super::run_awaits_1112::expected_error;
use super::session_facts::{assert_last_run_phone_bytes, expected_last_run};
use super::*;

fn paused_status() -> Value {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/runner-capacity-wait-1122/run.status.paused.json"),
    )
    .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The paused run, before and after its reset passes (when `next_ms` is
/// dropped), reaches the apps with its reason and its wait.
#[tokio::test]
async fn a_capacity_wait_reaches_the_apps_as_the_runs_wait() {
    let status = paused_status();
    assert_eq!(
        (status["state"].as_str(), status["reason"].as_str()),
        (Some("paused"), Some("concurrency_full"))
    );
    let mut state = json!({"run_id": "r1", "state": status["state"],
        "reason": status["reason"], "awaits": status["awaits"]});
    let expected = expected_last_run(&state);
    assert_eq!(
        expected["awaits"],
        json!({"party": "runner", "reason": "concurrency_full",
            "sinceMs": 1_795_342_184_183_u64, "nextMs": 1_795_342_190_183_u64})
    );
    assert_last_run_phone_bytes(state.clone(), None, &expected.to_string()).await;

    state["awaits"].as_object_mut().unwrap().remove("next_ms");
    let expected = expected_last_run(&state);
    assert!(expected["awaits"].get("nextMs").is_none(), "{expected}");
    assert_last_run_phone_bytes(state, None, &expected.to_string()).await;
}

/// A run that ends in an error while it waits keeps the wait's code and its
/// reset, so the apps can say when it moves.
#[tokio::test]
async fn a_run_that_ends_while_it_waits_names_the_wait() {
    let state = json!({"run_id": "r1", "state": "error", "reason": "error"});
    let error = json!({"class": "transient", "provider_code": "concurrency_full",
        "resets_at_ms": 1_795_342_190_183_u64});
    let mut expected = expected_last_run(&state);
    expected["error"] = expected_error(&error);
    assert_eq!(
        expected["error"],
        json!({"class": "transient", "providerCode": "concurrency_full",
            "resetsAtMs": 1_795_342_190_183_u64})
    );
    assert_last_run_phone_bytes(state, Some(error), &expected.to_string()).await;
}
