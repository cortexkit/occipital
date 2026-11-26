//! Runner contract 0.8.58's `not_delivered` reason `host_refused`: the host
//! refused the send for a reason resending the same words can't fix. The
//! send's run ends `error` with class `host_refused` and a named `cause`. Every
//! body here is the contract's own wire vector (`send.json`,
//! `send-status.json`, `run-result.json`), served through this module's real
//! send path.

use super::*;

/// The contract vector file `name`.
fn vector(name: &str) -> Value {
    let (_, body) = cortexkit_role_llm_runner::vectors::FILES
        .iter()
        .find(|(file, _)| *file == name)
        .unwrap();
    serde_json::from_str(body).unwrap()
}

/// The named case in a vector file's `list` whose body is under `member`.
fn case(name: &str, list: &str, member: &str, case: &str) -> Value {
    vector(name)[list]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"].as_str().unwrap().contains(case))
        .unwrap_or_else(|| panic!("{name}: no case {case}"))[member]
        .clone()
}

/// A send the host refused for good is listed unsent with reason
/// `host_refused`, the run's code, and its named cause, so the app can say
/// what to change; a repeat of its id is refused as final. Each of the four
/// named causes reaches the app; the policy refusal carries its own code.
#[tokio::test(start_paused = true)]
async fn a_host_refused_send_lists_its_cause_and_is_final() {
    let refused = case("send-status.json", "replies", "reply", "host refusal");
    assert_eq!(refused["reason"], "host_refused");
    for (answer, cause, code) in [
        (
            "unreadable attachment",
            "attachment_unreadable",
            "provider_error",
        ),
        (
            "invalid host configuration",
            "host_configuration_invalid",
            "provider_error",
        ),
        (
            "unsupported host version",
            "host_version_unsupported",
            "provider_error",
        ),
        (
            "managed host policy",
            "host_policy_refused",
            "host_policy_refused",
        ),
    ] {
        let status = case("run-result.json", "answers", "answer", answer);
        let f = fixture(Kind::Head);
        {
            let mut captured = f.runner.captured.lock().unwrap();
            captured.insert("session.send_status", json!({"result": refused}));
            captured.insert("run.status", json!({"result": status}));
        }
        f.sender.send(message("held", "words")).await.unwrap();
        f.now.store(START + 5_000, Ordering::SeqCst);
        let list = || f.receipts.queue().list("s", START).unwrap();
        eventually(|| list().is_some_and(|(entries, _)| entries[0]["state"] == "unsent")).await;
        let entry = list().unwrap().0[0].clone();
        assert_eq!(entry["reason"], "host_refused", "{cause}: {entry}");
        assert_eq!(entry["runnerCode"], code, "{cause}: {entry}");
        assert_eq!(entry["cause"], cause, "{cause}: {entry}");
        assert!(entry.get("retryAfterMs").is_none(), "{cause}: {entry}");
    }

    let f = fixture(Kind::Head);
    f.runner.scripted.lock().unwrap().push_back(Ok(case(
        "send.json",
        "replies",
        "reply",
        "host refused",
    )));
    let receipt = f.sender.send(message("again", "words")).await.unwrap();
    assert_eq!(receipt["code"], "not_delivered", "{receipt}");
    assert_eq!(receipt["reason"], "host_refused", "{receipt}");
    assert_eq!(receipt["terminal"], true, "{receipt}");
}

/// Thalamus's recorded bodies for each of the four causes, copied unchanged
/// (`tests/data/thalamus-host-refused-1117/`): the first answer is `pending`
/// naming its run, so the send is followed; `session.send_status` then says
/// `not_delivered`, reason `host_refused`, with no cause of its own; and the
/// run's status carries the cause. The send is listed unsent with the reason, the
/// run's code and the cause, and is no longer followed.
#[tokio::test(start_paused = true)]
async fn agent_runners_recorded_host_refusals_list_their_cause() {
    let data = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-host-refused-1117");
    let sums = std::fs::read_to_string(data.join("SHA256SUMS")).unwrap();
    // Sixteen bodies; the set's sums leave out its README.
    assert_eq!(sums.lines().count(), 16);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(data.join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
    let body = |name: String| -> Value {
        serde_json::from_slice(&std::fs::read(data.join(name)).unwrap()).unwrap()
    };
    for (cause, code) in [
        ("attachment_unreadable", "provider_error"),
        ("host_configuration_invalid", "provider_error"),
        ("host_version_unsupported", "provider_error"),
        ("host_policy_refused", "host_policy_refused"),
    ] {
        let first = body(format!("{cause}-send-reply.json"));
        assert_eq!(first["state"], "pending", "{cause}");
        let f = fixture(Kind::Head);
        f.runner.scripted.lock().unwrap().push_back(Ok(first));
        {
            let mut captured = f.runner.captured.lock().unwrap();
            captured.insert(
                "session.send_status",
                json!({"result": body(format!("{cause}-send-status.json"))}),
            );
            captured.insert(
                "run.status",
                json!({"result": body(format!("{cause}-run-status.json"))}),
            );
        }
        let receipt = f.sender.send(message("held", "words")).await.unwrap();
        assert_eq!(receipt["ok"], true, "{cause}: {receipt}");
        f.now.store(START + 5_000, Ordering::SeqCst);
        let list = || f.receipts.queue().list("s", START).unwrap();
        eventually(|| list().is_some_and(|(entries, _)| entries[0]["state"] == "unsent")).await;
        let entry = list().unwrap().0[0].clone();
        assert_eq!(entry["reason"], "host_refused", "{cause}: {entry}");
        assert_eq!(entry["runnerCode"], code, "{cause}: {entry}");
        assert_eq!(entry["cause"], cause, "{cause}: {entry}");
        assert!(entry.get("retryAfterMs").is_none(), "{cause}: {entry}");
        assert!(f.receipts.followed(10).unwrap().is_empty(), "{cause}");
        // The run's final answer decodes too, with the same cause.
        let result = body(format!("{cause}-run-result.json"));
        assert_eq!(result["error"]["class"], "host_refused", "{cause}");
        assert_eq!(result["error"]["cause"], cause, "{cause}");
    }
}

/// A cause the contract doesn't name, or a cause on another class of error,
/// isn't passed on: the app gets the reason and code alone.
#[tokio::test(start_paused = true)]
async fn an_unnamed_cause_is_left_out() {
    for error in [
        json!({"class": "host_refused", "provider_code": "provider_error",
            "cause": "host_on_fire"}),
        json!({"class": "host_unavailable", "provider_code": "provider_error",
            "cause": "host_unreachable"}),
    ] {
        let f = fixture(Kind::Head);
        {
            let mut captured = f.runner.captured.lock().unwrap();
            captured.insert(
                "session.send_status",
                json!({"result": {"state": "not_delivered", "reason": "host_refused",
                    "run_id": "run-x"}}),
            );
            captured.insert(
                "run.status",
                json!({"result": {"run_id": "run-x", "state": "error", "error": error}}),
            );
        }
        f.sender.send(message("held", "words")).await.unwrap();
        f.now.store(START + 5_000, Ordering::SeqCst);
        let list = || f.receipts.queue().list("s", START).unwrap();
        eventually(|| list().is_some_and(|(entries, _)| entries[0]["state"] == "unsent")).await;
        let entry = list().unwrap().0[0].clone();
        assert_eq!(entry["runnerCode"], "provider_error", "{entry}");
        assert!(entry.get("cause").is_none(), "{entry}");
    }
}
