//! Broca naming why Anthropic refused a request whose signed thinking was
//! changed: the run's error gains `cause: "signed_reasoning_rejected"` beside
//! `provider_code: "request_invalid"` (runner contract 0.8.43, `RunError.cause`).
//! Recorded from its real serializers on a synthetic session, copied unchanged
//! into `tests/data/broca-signed-reasoning-cause-1112/`. Each `before` body is
//! its `after` body with the member cleared.
//!
//! This reader never reads the member. Over `run.status` the queued-send
//! follower lists the run's code (tested beside it in `send/tests.rs`), and
//! `run.result` it never calls. Over `session.subscribe` an error unit maps to a
//! display input. This checks the stream side: with the member, the unit is
//! the same live event and maps to the same input.

use super::*;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/broca-signed-reasoning-cause-1112")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

#[test]
fn the_signed_reasoning_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    // Nine bodies and the README.
    assert_eq!(sums.lines().count(), 10);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The two stream bodies come from separate synthetic sessions, so their
/// cursors differ; with the `after` cursor set to the `before` one and the
/// member removed, the bodies are equal. Both are a live event, and both map
/// to the same display input.
#[test]
fn a_signed_reasoning_cause_changes_nothing_the_stream_reader_reads() {
    let before = sample("before.subscribe-error.json");
    let mut after = sample("after.subscribe-error.json");
    assert_eq!(
        after["unit"]["error"]["cause"], "signed_reasoning_rejected",
        "{after}"
    );
    after["cursor"] = before["cursor"].clone();
    let mut stripped = after.clone();
    stripped["unit"]["error"]
        .as_object_mut()
        .unwrap()
        .remove("cause");
    assert_eq!(stripped, before);
    assert_eq!(carries_live_event(&after), carries_live_event(&before));
    let mapped = |body: &Value| {
        normalize(body.clone()).map(|event| (display_events::map(&event), event["event"].clone()))
    };
    assert_eq!(mapped(&after), mapped(&before));
    assert!(mapped(&after).is_some(), "the unit is read as an event");
}
