//! Agent Runner holding a queued send while the agent's turn runs, on all
//! three of its hosts (OpenCode 2, Pi and Claude Code), and "Send now" moving
//! it into that turn. Each host's replies were recorded on the real host and
//! copied unchanged into `tests/data/thalamus-send-promote-v2-1116/` (its
//! README is left out, since it names a local path; `SHA256SUMS` covers the
//! three replies): the send answers `pending` with a submission id and no run,
//! `session.send_status` follows it until it lands as a step of a finished run,
//! and `session.promote` answers `promoted`, or `delivered` once it already ran.

use super::*;

fn data() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-send-promote-v2-1116")
}

fn host(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(data().join(name)).unwrap()).unwrap()
}

/// On each host a queued message is listed `queued` while the runner holds it,
/// its "Send now" answers reach the app unchanged, and it leaves the list once
/// it lands in the turn.
#[tokio::test]
async fn a_held_queued_send_is_listed_promoted_and_landed_on_every_host() {
    let sums = std::fs::read_to_string(data().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 3);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(data().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
    // Each host's own description, recorded for the first "Send now" samples,
    // which declares `send_promote`.
    for (name, describe) in [
        ("opencode2.json", "10-role-describe-opencode2.json"),
        ("pi.json", "11-role-describe-pi.json"),
        ("claude-code.json", "12-role-describe-claude-code.json"),
    ] {
        let recorded = host(name);
        let f = granted_fixture(Kind::Head);
        let describe: Value = serde_json::from_slice(
            &std::fs::read(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/data/thalamus-send-promote-1116")
                    .join(describe),
            )
            .unwrap(),
        )
        .unwrap();
        f.runner
            .scripted
            .lock()
            .unwrap()
            .push_back(Ok(recorded["session.send"].clone()));
        f.runner.captured.lock().unwrap().insert(
            "session.send_status",
            json!({"result": recorded["session.send_status.pending"]}),
        );
        let receipt = f.sender.send(message("held", "words")).await.unwrap();
        assert_eq!(receipt["ok"], true, "{name}: {receipt}");
        // The recording's send used the legacy `queue` string, which the runner
        // now holds on the same ordered `idle_start` queue that this module's
        // queued sends name directly.
        assert_eq!(recorded["delivery"], "queue", "{name}");
        let sent = sends(&f.runner).pop().unwrap();
        assert_eq!(sent["delivery"], json!(["idle_start"]), "{name}: {sent}");
        let listed = || f.receipts.queue().list("s", START).unwrap();
        assert_eq!(listed().unwrap().0[0]["state"], "queued", "{name}");
        f.runner
            .captured
            .lock()
            .unwrap()
            .insert("role.describe", json!({"result": describe}));

        for (answer, request_id) in [
            ("session.promote", "now-1"),
            ("session.promote.delivered", "now-2"),
        ] {
            f.runner
                .captured
                .lock()
                .unwrap()
                .insert("session.promote", json!({"result": recorded[answer]}));
            let reply = f
                .sender
                .promote(json!({"session": "s", "clientMessageId": "held",
                    "clientRequestId": request_id}))
                .await
                .unwrap_or_else(|refusal| panic!("{name} {answer}: {refusal:?}"));
            assert_eq!(reply, recorded[answer], "{name} {answer}");
        }

        f.runner.captured.lock().unwrap().insert(
            "session.send_status",
            json!({"result": recorded["session.send_status.landed"]}),
        );
        assert!(f.sender.nudge(receipt["effectId"].as_str().unwrap()));
        eventually(|| listed().is_none()).await;
    }
}
