//! The sends of `phone-reader-capacity-wait-v1`: routing has no free capacity
//! for an agent's model (`concurrency_full`, a wait under llm-runner 0.9.0),
//! so the send waits rather than failing. Its README says what each file
//! shows. With `OCCIPITAL_WRITE_SAMPLES` set to a folder, the files are
//! written there instead of compared.

use super::*;

const SET: &str = "phone-reader-capacity-wait-v1";

/// Write `value` to the set as `name`, or compare it with the published file.
fn keep(name: &str, value: &Value) {
    if let Some(folder) = std::env::var_os("OCCIPITAL_WRITE_SAMPLES") {
        let folder = std::path::PathBuf::from(folder).join(SET);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(name), serde_json::to_vec(value).unwrap()).unwrap();
        return;
    }
    let published: Value = serde_json::from_slice(
        &std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../occipital-app-reader/test-vectors")
                .join(SET)
                .join(name),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(value, &published, "{name}");
}

/// A send queued behind a running turn whose own admission found no capacity
/// when its turn came: the send reads `finished` with reason `error`, and the
/// run's error names `concurrency_full` with routing's reset. The entry is
/// listed unsent with that code and the time left until the reset from when
/// the turn ended, so the app sends the words again then, as a new message.
#[tokio::test(start_paused = true)]
async fn a_queued_send_without_capacity_is_listed_as_a_wait() {
    let f = fixture(Kind::Head);
    {
        let mut captured = f.runner.captured.lock().unwrap();
        captured.insert(
            "session.send_status",
            json!({"result": {"state": "finished", "reason": "error",
                "run_id": "run-cap", "submission_id": "sub-cap"}}),
        );
        captured.insert(
            "run.status",
            json!({"result": {"run_id": "run-cap", "state": "error", "error": {
                "class": "transient", "provider_code": "concurrency_full",
                "message": "routing has no free capacity for this model",
                "resets_at_ms": START + 65_000}}}),
        );
    }
    f.sender.send(message("q1", "queued words")).await.unwrap();
    // Its turn comes five seconds after it was queued.
    f.now.store(START + 5_000, Ordering::SeqCst);
    let list = || f.receipts.queue().list("s", START).unwrap();
    eventually(|| list().is_some_and(|(entries, _)| entries[0]["state"] == "unsent")).await;
    let entry = list().unwrap().0[0].clone();
    assert_eq!(entry["runnerCode"], "concurrency_full", "{entry}");
    assert_eq!(entry["retryAfterMs"], 60_000, "{entry}");
    let mut reply = json!({});
    f.sender.attach_queued(&mut reply, "s").await;
    keep(
        "04-queued-unsent.json",
        &json!({"queuedSends": reply["queuedSends"]}),
    );
}

/// A send refused at once because routing has no free capacity: the outer
/// code is `concurrency_full`, the contract's mapping of routing's `no_route`
/// with that reason, carrying routing's reset (`resets_at_ms`) and the wait
/// it suggests (`retry_after_secs`), which takes precedence. The app
/// gets `runner_refused`, transient, with the wait before sending again; the
/// runner stored nothing, so that send is a new one.
#[test]
fn a_send_refused_for_capacity_is_a_wait() {
    let runner = json!({"code": "concurrency_full",
        "message": "routing has no free capacity for this model; retry after 30 s",
        "detail": {"routing_code": "no_route", "class": "transient", "retry_after_secs": 30.0,
            "resets_at_ms": START + 30_000,
            "routing_detail": {"reason": "concurrency_full", "reset_at_ms": START + 30_000}}});
    let refusal = super::super::refused(
        runner["code"].as_str().unwrap(),
        runner["message"].as_str().unwrap(),
        runner.get("detail"),
    );
    let wire = json!({"kind": "moduleError", "code": refusal.code,
        "message": refusal.message, "detail": refusal.detail});
    assert_eq!(
        wire["detail"],
        json!({"class": "transient", "runnerCode": "concurrency_full", "retryAfterMs": 30_000})
    );
    keep("05-send-refused.runner.json", &runner);
    keep("05-send-refused.json", &wire);
}
