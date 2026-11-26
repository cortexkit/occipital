//! Thalamus's Pi runs that return a structured result (`structured_output`,
//! llm-runner 0.8.66), recorded by Thalamus from a send-started Pi 1.6.0
//! against a stand-in model and copied unchanged into
//! `tests/data/thalamus-structured-output-pi-1120/`. Each file holds a run's
//! `run.status` and `run.result` bodies. Occipital reads `run.status` in two
//! places only, and `run.result` never: a Stop on a run that's no longer
//! active reads its `state`, and a queued send whose run ended in an error
//! reads that run's `error`. The structured result, `finish` and the raw
//! `final_step_finish_reason` beside them must change neither.

use super::*;

fn data() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-structured-output-pi-1120")
}

const CASES: [&str; 6] = [
    "submit-passed",
    "submit-failed",
    "submit-absent",
    "submit-skipped",
    "final-text-passed",
    "final-text-failed",
];

fn run_status(case: &str) -> Value {
    let file: Value =
        serde_json::from_slice(&std::fs::read(data().join(format!("{case}.json"))).unwrap())
            .unwrap();
    file["run_status"].clone()
}

#[test]
fn the_structured_output_samples_keep_their_checksums() {
    let sums = std::fs::read(data().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("8f355ec5"));
    let sums = String::from_utf8(sums).unwrap();
    // Six bodies and the README.
    assert_eq!(sums.lines().count(), 7);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(data().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// A Stop that finds the run already over answers `not_active` for every
/// structured outcome, as for any run that completed on its own.
#[tokio::test]
async fn a_stop_after_a_structured_run_reads_it_as_ended() {
    for case in CASES {
        let f = fixture(Kind::Head);
        {
            let mut captured = f.runner.captured.lock().unwrap();
            captured.insert(
                "role.describe",
                json!({"result": {"capabilities": ["run_cancel", "delivery_order", "run_cancel_by_interrupt"]}}),
            );
            captured.insert("run.cancel", json!({"result": "not_active"}));
            captured.insert("run.status", json!({"result": run_status(case)}));
        }
        let stop = json!({"session": "s", "runId": "run_pi", "clientRequestId": "stop-1"});
        assert_eq!(
            f.sender.stop(stop).await.unwrap(),
            json!({"state": "not_active", "runId": "run_pi"}),
            "{case}"
        );
    }
}

/// A send the runner didn't deliver reads its run's error, and a structured
/// run has none: it names no code, so the send is listed unsent with its
/// reason alone, never with a code drawn from the structured result.
#[tokio::test]
async fn a_structured_run_names_no_error_for_a_send() {
    for case in CASES {
        let f = fixture(Kind::Head);
        f.runner
            .captured
            .lock()
            .unwrap()
            .insert("run.status", json!({"result": run_status(case)}));
        let target = Target(
            f.runner.clone(),
            SessionRef {
                runner: "thalamus-harness".into(),
                session: "s".into(),
            },
        );
        let answer =
            json!({"state": "not_delivered", "run_id": "run_pi", "reason": "handover_failed"});
        assert_eq!(
            super::super::ended_run(&target, Some(&answer), START).await,
            None,
            "{case}"
        );
        // The same body with an error does name its code, so the `None` above
        // is the structured run's, not a reply this test failed to read.
        let mut errored = run_status(case);
        errored["error"] = json!({"provider_code": "provider_error"});
        f.runner
            .captured
            .lock()
            .unwrap()
            .insert("run.status", json!({"result": errored}));
        let ended = super::super::ended_run(&target, Some(&answer), START)
            .await
            .unwrap_or_else(|| panic!("{case}: the error went unread"));
        assert_eq!(ended.runner_code, "provider_error", "{case}");
    }
}
