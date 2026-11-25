//! Broca's paused run when routing has no free capacity for its account
//! (`tests/data/broca-capacity-refusal-1125b/continuation-paused.run.status.json`,
//! copied unchanged): `paused` with reason `account_limit`, waiting on the
//! runner until routing's reset, with the provider's own message. The chat's
//! last run carries the same state and `awaits` in `session.head`, and reaches
//! the apps as a wait. The new `account_continue` member on the run's error
//! changes nothing here. Its sends are in `send::tests::capacity_refusal_1125b`.

use super::session_facts::{assert_last_run_phone_bytes, expected_last_run};
use super::*;

#[tokio::test]
async fn a_run_paused_for_its_account_reaches_the_apps_as_a_wait() {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/broca-capacity-refusal-1125b/continuation-paused.run.status.json"),
    )
    .unwrap();
    let status: Value = serde_json::from_slice::<Value>(&bytes).unwrap()["result"].clone();
    let state = json!({"run_id": "r1", "state": status["state"], "reason": status["reason"],
        "resume_not_before_ms": status["resume_not_before_ms"], "awaits": status["awaits"]});
    let expected = expected_last_run(&state);
    assert_eq!(
        (&expected["state"], &expected["reason"]),
        (&json!("paused"), &json!("account_limit"))
    );
    assert_eq!(expected["awaits"]["party"], "runner", "{expected}");
    assert_eq!(
        expected["awaits"]["nextMs"], status["resume_not_before_ms"],
        "{expected}"
    );
    assert_eq!(
        expected["awaits"]["message"]["from"], "provider",
        "{expected}"
    );
    assert_last_run_phone_bytes(state, None, &expected.to_string()).await;
}
