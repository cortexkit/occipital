//! Broca's replies when routing has no free capacity for an account, after it
//! pauses a running run instead of ending it
//! (`tests/data/broca-capacity-refusal-1125b`, copied unchanged). Three of
//! them reach this reader's send paths: a send refused at once, and the
//! `run.status` of a queued send's run that ended in an error, refused at
//! admission or after its pause. Each still reads as a wait with routing's
//! time; the members Broca added (`account_continue`, `routing_detail` on the
//! run's error) change nothing here. The paused run's own status is read as
//! the chat's last run, in `broca::tests::capacity_refusal_1125b`.

use super::*;

fn recorded(name: &str) -> Value {
    let data = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/broca-capacity-refusal-1125b");
    let sums = std::fs::read_to_string(data.join("SHA256SUMS")).unwrap();
    let (expected, _) = sums
        .lines()
        .filter_map(|line| line.split_once("  "))
        .find(|(_, file)| *file == name)
        .unwrap();
    let bytes = std::fs::read(data.join(name)).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), expected, "{name}");
    serde_json::from_slice(&bytes).unwrap()
}

/// Refused at once: the app gets `runner_refused`, transient, with Broca's
/// code and the wait it names.
#[test]
fn a_send_refused_at_once_is_a_wait() {
    let runner = recorded("admission-direct.session.send.json");
    let refusal = super::super::refused(
        runner["code"].as_str().unwrap(),
        runner["message"].as_str().unwrap(),
        runner.get("detail"),
    );
    assert_eq!(refusal.code, "runner_refused");
    assert_eq!(
        refusal.detail,
        Some(
            json!({"class": "transient", "runnerCode": "account_selection_refused",
            "retryAfterMs": 28_000})
        )
    );
}

/// A queued send whose run ended in an error, at admission or after its one
/// pause, is listed unsent with the run's code and the wait it names.
#[tokio::test(start_paused = true)]
async fn a_queued_send_whose_run_ended_without_capacity_is_listed_as_a_wait() {
    for (run, code, wait_ms) in [
        (
            "admission-ordered.run.status.json",
            "account_selection_refused",
            30_000,
        ),
        (
            "continuation.run.status.json",
            "account_limit_exhausted",
            604_800_000,
        ),
    ] {
        let f = fixture(Kind::Head);
        {
            let mut captured = f.runner.captured.lock().unwrap();
            captured.insert(
                "session.send",
                recorded("admission-ordered.session.send.json"),
            );
            captured.insert(
                "session.send_status",
                recorded("admission-ordered.session.send_status.json"),
            );
            captured.insert("run.status", recorded(run));
        }
        f.sender.send(message("q1", "queued words")).await.unwrap();
        let list = || f.receipts.queue().list("s", START).unwrap();
        eventually(|| list().is_some_and(|(entries, _)| entries[0]["state"] == "unsent")).await;
        let entry = list().unwrap().0[0].clone();
        assert_eq!(entry["reason"], "error", "{run}: {entry}");
        assert_eq!(entry["runnerCode"], code, "{run}: {entry}");
        assert_eq!(entry["retryAfterMs"], wait_ms, "{run}: {entry}");
    }
}
