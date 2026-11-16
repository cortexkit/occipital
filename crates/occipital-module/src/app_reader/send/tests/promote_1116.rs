//! Agent Runner's recorded `session.promote` answers ("Send now": the runner
//! moves a message still waiting for the running turn into that turn) for its
//! OpenCode 2, Pi and Claude Code hosts, from Thalamus's
//! `tests/data/thalamus-send-promote-1116/` (copied unchanged), served through
//! this module's `session.promote_send`. Each host's description (files 10-12)
//! is the one the runner is asked for first.

use super::*;

fn data() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-send-promote-1116")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(data().join(name)).unwrap()).unwrap()
}

/// What the app receives for each recorded answer: an outcome passes on
/// unchanged, and each refusal maps to the app's own code. The runner is asked
/// about the message's own send (the `send_id` it went out under), with the
/// app's `clientRequestId` as its `request_id`.
#[tokio::test]
async fn the_recorded_promote_answers_reach_the_app() {
    let sums = std::fs::read_to_string(data().join("SHA256SUMS")).unwrap();
    // Twelve bodies and the README.
    assert_eq!(sums.lines().count(), 13);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(data().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
    // (file, the app's reply: Ok(the runner's reply) or Err(the app's code))
    let cases: [(&str, Result<(), &str>); 9] = [
        ("01-promoted.json", Ok(())),
        ("02-repeated-request.json", Ok(())),
        ("03-delivered.json", Ok(())),
        ("04-not-queued.json", Ok(())),
        ("05-not-promoted-no-active-run.json", Ok(())),
        (
            "06-refused-request-id-reuse.json",
            Err("client_request_id_conflict"),
        ),
        ("07-refused-invalid-params.json", Err("runner_refused")),
        (
            "08-refused-scope-owner-mismatch.json",
            Err("enqueue_not_authorized"),
        ),
        ("09-refused-unsupported.json", Err("unsupported")),
    ];
    for describe in [
        "10-role-describe-opencode2.json",
        "11-role-describe-pi.json",
        "12-role-describe-claude-code.json",
    ] {
        let f = granted_fixture(Kind::Head);
        let queued = f.sender.send(message("msg-1", "words")).await.unwrap();
        assert_eq!(queued["ok"], true, "{queued}");
        let sent_as = f
            .runner
            .calls()
            .into_iter()
            .find(|(method, _)| method == "session.send")
            .map(|(_, body)| body["send_id"].clone())
            .unwrap();
        // The host's own description, which "Send now" reads before it asks.
        f.runner
            .captured
            .lock()
            .unwrap()
            .insert("role.describe", json!({"result": sample(describe)}));
        for (file, expected) in cases {
            let recorded = sample(file);
            let answer = match recorded.get("reply") {
                Some(reply) => json!({"result": reply}),
                None => recorded["refusal"].clone(),
            };
            f.runner
                .captured
                .lock()
                .unwrap()
                .insert("session.promote", answer);
            let request_id = recorded["request"]["request_id"].as_str().unwrap();
            let reply = f
                .sender
                .promote(json!({"session": "s", "clientMessageId": "msg-1",
                    "clientRequestId": request_id}))
                .await;
            match (expected, reply) {
                (Ok(()), Ok(reply)) => {
                    assert_eq!(reply, recorded["reply"], "{describe} {file}");
                }
                (Err(code), Err(refusal)) => {
                    assert_eq!(refusal.code, code, "{describe} {file}: {}", refusal.message);
                }
                (expected, reply) => panic!("{describe} {file}: {expected:?} but {reply:?}"),
            }
            let asked = f
                .runner
                .calls()
                .into_iter()
                .filter(|(method, _)| method == "session.promote")
                .map(|(_, params)| params)
                .next_back()
                .unwrap();
            assert_eq!(
                asked,
                json!({"send_id": sent_as, "request_id": request_id}),
                "{describe} {file}"
            );
        }
    }
}
