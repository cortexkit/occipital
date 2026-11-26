//! A message queued behind a run the runner stopped (runner contract 0.10.1):
//! three steps in a row cut at the output limit (`output_limit`), or the same
//! tool call about to run a twentieth time (`tool_loop`). The message is
//! listed unsent, naming the run's code as `runnerCode`. The run's error is
//! the one Thalamus recorded for such a run
//! (`tests/data/thalamus-output-limit-v1/`), served as that run's
//! `run.status`; the send's own state is the runner contract's for a send
//! whose turn ended in an error. Each listing is a file of
//! `phone-reader-run-stopped-v1`, whose `lastRun` files the broca tests'
//! `output_limit_1127` writes.

use super::*;
use std::path::{Path, PathBuf};

const SET: &str = "phone-reader-run-stopped-v1";

/// `value` as file `name` of [`SET`]: written there when
/// `OCCIPITAL_WRITE_SAMPLES` names a folder, else compared with the published
/// file.
fn sample(name: &str, value: &Value) {
    if let Some(folder) = std::env::var_os("OCCIPITAL_WRITE_SAMPLES") {
        let folder = PathBuf::from(folder).join(SET);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(name), serde_json::to_vec(value).unwrap()).unwrap();
        return;
    }
    let published = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../occipital-app-reader/test-vectors")
        .join(SET)
        .join(name);
    let published: Value = serde_json::from_slice(&std::fs::read(published).unwrap()).unwrap();
    assert_eq!(value, &published, "{name}");
}

#[tokio::test(start_paused = true)]
async fn a_message_queued_behind_a_stopped_run_names_the_runs_code() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/thalamus-output-limit-v1");
    for (code, file, name) in [
        (
            "output_limit",
            "run-result-output-limit.json",
            "03-unsent-output-limit.json",
        ),
        (
            "tool_loop",
            "run-result-tool-loop.json",
            "04-unsent-tool-loop.json",
        ),
    ] {
        let f = fixture(Kind::Head);
        let run: Value = serde_json::from_slice(&std::fs::read(data.join(file)).unwrap()).unwrap();
        let run_id = run["result"]["run_id"].clone();
        {
            let mut captured = f.runner.captured.lock().unwrap();
            captured.insert(
                "session.send_status",
                json!({"result": {"state": "finished", "run_id": run_id, "reason": "error"}}),
            );
            captured.insert("run.status", run);
        }
        f.sender.send(message("q1", "queued words")).await.unwrap();
        f.now.store(START + 5_000, Ordering::SeqCst);
        let list = || f.receipts.queue().list("s", START).unwrap();
        eventually(|| list().is_some_and(|(entries, _)| entries[0]["state"] == "unsent")).await;
        let (entries, _) = list().unwrap();
        assert_eq!(entries[0]["reason"], "error", "{code}");
        assert_eq!(entries[0]["runnerCode"], code, "{code}");
        // A stopped run is final, so it names no wait before sending again.
        assert!(entries[0].get("retryAfterMs").is_none(), "{code}");
        let mut reply = json!({});
        f.sender.attach_queued(&mut reply, "s").await;
        sample(name, &json!({"queuedSends": reply["queuedSends"]}));
    }
}
