//! Thalamus's `not_delivered` reason `host_unavailable` (runner contract
//! 0.8.57): a send accepted while its host couldn't be started or reached, and
//! never handed to it, ends so five minutes later. Recorded from Agent
//! Runner's committed tree and copied unchanged into
//! `tests/data/thalamus-host-unavailable-1116/`.

use super::*;

fn data() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-host-unavailable-1116")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(data().join(name)).unwrap()).unwrap()
}

#[test]
fn the_host_unavailable_samples_keep_their_checksums() {
    let sums = std::fs::read(data().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("de059bd8"));
    let sums = String::from_utf8(sums).unwrap();
    // Five bodies and the README.
    assert_eq!(sums.lines().count(), 6);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(data().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// A held send that ends `host_unavailable` is listed unsent with that reason
/// and its run's `provider_error`, and a repeat of its id is refused as final,
/// so the app sends the words again as a new message.
#[tokio::test(start_paused = true)]
async fn a_send_whose_host_stayed_unavailable_is_unsent_and_final() {
    let f = fixture(Kind::Head);
    {
        let mut captured = f.runner.captured.lock().unwrap();
        captured.insert(
            "session.send_status",
            json!({"result": sample("01-session-send-status.json")}),
        );
        captured.insert(
            "run.status",
            json!({"result": sample("02-run-status.json")}),
        );
    }
    f.sender.send(message("held", "words")).await.unwrap();
    f.now.store(START + 5_000, Ordering::SeqCst);
    let list = || f.receipts.queue().list("s", START).unwrap();
    eventually(|| list().is_some_and(|(entries, _)| entries[0]["state"] == "unsent")).await;
    let entry = list().unwrap().0[0].clone();
    assert_eq!(entry["reason"], "host_unavailable", "{entry}");
    assert_eq!(entry["runnerCode"], "provider_error", "{entry}");
    assert!(entry.get("retryAfterMs").is_none(), "{entry}");

    let f = fixture(Kind::Head);
    f.runner
        .scripted
        .lock()
        .unwrap()
        .push_back(Ok(sample("05-same-send-id-replay.json")));
    let receipt = f.sender.send(message("again", "words")).await.unwrap();
    assert_eq!(receipt["code"], "not_delivered", "{receipt}");
    assert_eq!(receipt["reason"], "host_unavailable", "{receipt}");
    assert_eq!(receipt["terminal"], true, "{receipt}");
}

/// A send that never started a turn of its own has no run the runner can be
/// asked about, so its `not_delivered` reply leaves `run_id` out. It's listed
/// unsent with its reason alone, and `run.status` is never asked.
#[tokio::test(start_paused = true)]
async fn a_not_delivered_reply_without_a_run_lists_the_reason_alone() {
    for reason in ["host_unavailable", "handover_failed"] {
        let f = fixture(Kind::Head);
        f.runner.captured.lock().unwrap().insert(
            "session.send_status",
            json!({"result": {"state": "not_delivered", "reason": reason}}),
        );
        f.sender.send(message("held", "words")).await.unwrap();
        f.now.store(START + 5_000, Ordering::SeqCst);
        let list = || f.receipts.queue().list("s", START).unwrap();
        eventually(|| list().is_some_and(|(entries, _)| entries[0]["state"] == "unsent")).await;
        let entry = list().unwrap().0[0].clone();
        assert_eq!(entry["reason"], reason, "{entry}");
        assert!(entry.get("runnerCode").is_none(), "{entry}");
        assert!(
            !f.runner
                .calls()
                .iter()
                .any(|(method, _)| method == "run.status"),
            "{reason}"
        );
    }
}
