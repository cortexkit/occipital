//! Agent Runner's endings for a prompt it could never hand to its host, step 2:
//! a prompt proven never forwarded is retried once and then ends
//! `not_delivered`, and a run whose hand-over failed for good now ends in an
//! error (class `handover_failed`, `host_relaunch_failed`, `runner` or
//! `provider`) instead of an interrupted turn. Ten endings, recorded from its
//! real code paths and copied unchanged into
//! `tests/data/thalamus-not-delivered-step2-1116/`; its README says which file
//! is which case and surface.
//!
//! This reader takes three of those surfaces itself, and these tests run every
//! ending's body through them:
//! - `session.head`: the chat's `lastRun`, whose `providerCode` passes only for
//!   a code the contract names;
//! - `session.read`: an empty page, since a prompt never forwarded writes no
//!   entry;
//! - subscribe `run_finished` frames: whether they are live events, and what
//!   they do to the live view.
//!
//! The send replies and `run.status` go through the sender, in its own tests.

use super::run_awaits_1112::expected_error;
use super::session_facts::{assert_last_run_phone_bytes, expected_last_run};
use super::*;

pub(crate) fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-not-delivered-step2-1116")
}

pub(crate) fn body(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap())
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Each ending's file prefix, with the class its run error carries.
pub(crate) const ENDINGS: [(&str, &str); 10] = [
    ("host_unreachable", "handover_failed"),
    ("not_forwarded", "handover_failed"),
    ("nothing_written", "handover_failed"),
    ("no_stdin", "handover_failed"),
    ("blocked", "handover_failed"),
    ("attachment_unreadable", "handover_failed"),
    ("host_configuration_invalid", "host_relaunch_failed"),
    ("host_version_unsupported", "host_relaunch_failed"),
    ("host_policy_refused", "runner"),
    ("future-permanent", "provider"),
];

#[test]
fn the_step2_samples_keep_their_checksums() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("6887525d16e359ba"));
    let sums = String::from_utf8(sums).unwrap();
    let mut listed = 0;
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
        listed += 1;
    }
    // 121 bodies and envelopes, and the README.
    assert_eq!(listed, 122);
}

/// Every ending's head reaches the apps as `lastRun` byte for byte on details,
/// the newest page, a claim and a watch: state `error` with the run's class.
/// `providerCode` reads `provider_error` for the eight endings that carry it,
/// and is left out for `host_policy_refused` and `future-permanent`, which the
/// contract doesn't name.
#[tokio::test]
async fn every_ending_reaches_the_apps_as_its_last_run() {
    let mut coded = 0;
    for (ending, class) in ENDINGS {
        let head = body(&format!("{ending}--session.head.json"));
        let state = head["last_run_state"].clone();
        assert_eq!(state["state"], "error", "{ending}");
        let error = head["last_run_error"].clone();
        assert_eq!(error["class"], class, "{ending}");
        let mut expected = expected_last_run(&state);
        expected["error"] = expected_error(&error);
        match expected["error"].get("providerCode") {
            Some(code) => {
                assert_eq!(code, "provider_error", "{ending}");
                coded += 1;
            }
            None => assert!(
                matches!(ending, "host_policy_refused" | "future-permanent"),
                "{ending}"
            ),
        }
        let expected = serde_json::to_string(&expected).unwrap();
        assert_last_run_phone_bytes(state, Some(error), &expected).await;
    }
    assert_eq!(coded, 8);
}

/// A prompt never forwarded writes no entry, so each ending's read is an empty
/// page that decodes as the contract's read reply.
#[test]
fn every_ending_reads_as_an_empty_page() {
    for (ending, _) in ENDINGS {
        let read = body(&format!("{ending}--session.read.json"));
        let decoded: cortexkit_role_llm_runner::read::ReadPage =
            serde_json::from_value(read).unwrap_or_else(|e| panic!("{ending}: {e}"));
        assert!(decoded.messages.is_empty(), "{ending}");
    }
}

/// Every ending's `run_finished` frame is a live control event that ends the
/// live view of its own run, as a plain run ending does: its outer state
/// `error` and the stored runner event name change nothing.
#[test]
fn every_run_finished_frame_ends_its_run() {
    let plain = display_events::map(&json!({"kind": "control", "event": {"type": "run_finished",
        "run_id": "run_685832dab5bea7a31bc54fd1", "reason": "error"}}));
    assert!(plain.is_some());
    for (ending, _) in ENDINGS {
        let frame = body(&format!("{ending}--run-finished.json"));
        assert!(carries_live_event(&frame), "{ending}");
        let event = normalize(frame).unwrap_or_else(|| panic!("{ending}: not a runner event"));
        assert_eq!(display_events::map(&event), plain, "{ending}");
    }
}

/// Agent Runner's `host_unavailable` ending (runner contract 0.8.57,
/// `tests/data/thalamus-host-unavailable-1116/`) ends its run's live view the
/// same way.
#[test]
fn the_host_unavailable_run_finished_frame_ends_its_run() {
    let frame: Value =
        serde_json::from_slice(
            &std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "tests/data/thalamus-host-unavailable-1116/04-session-run-finished-event.json",
            ))
            .unwrap(),
        )
        .unwrap();
    let run = frame["run_id"].clone();
    assert!(carries_live_event(&frame));
    let event = normalize(frame).unwrap();
    assert_eq!(
        display_events::map(&event),
        display_events::map(&json!({"kind": "control", "event": {"type": "run_finished",
            "run_id": run, "reason": "error"}}))
    );
}
