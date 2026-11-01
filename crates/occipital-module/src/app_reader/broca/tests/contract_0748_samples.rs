//! Broca's replies after it adopts runner contract 0.7.42 to 0.7.48, recorded
//! from its serve path, as the reader decodes them. Copied from Broca's shared
//! sample set `contract-0748-1101` (whose `ALL-SHA256SUMS` hashes to
//! `a03f2723d797e849…`), keeping only the replies this reader reads:
//! - `run.status`, which the app's Stop reads after a cancel finds the run not
//!   active; it gains `finish`, the run's last model step's finish reason.
//! - `session.read` with and without `include_display_facts`, which the reader
//!   now asks for on Broca too, since Broca declares `read_display_facts`.
//! - `session.history` display rows, read only from a runner without display
//!   facts; a user row's `sender` now also names steers and queued sends.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-contract-0.7.48")
}

/// A sample's reply body: the `result` of a recorded reply, or the whole file
/// where Broca saved the page alone (the sender samples).
fn result(name: &str) -> Value {
    let value: Value = serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap();
    value.get("result").cloned().unwrap_or(value)
}

#[test]
fn the_contract_0748_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 12);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Stop reads only `state` from `run.status`, as plain JSON, so `finish` and
/// any member a later runner adds are ignored, for every provider family.
#[test]
fn stop_reads_the_state_of_every_family_run_status() {
    let families = std::fs::read_dir(root().join("finish-sender-samples/finish")).unwrap();
    let mut seen = 0;
    for family in families {
        let family = family.unwrap().file_name().into_string().unwrap();
        let status = result(&format!(
            "finish-sender-samples/finish/{family}/run-status.json"
        ));
        assert!(status["state"].is_string(), "{family}");
        assert!(
            status["finish"].is_string(),
            "{family}: the sample carries the new member"
        );
        seen += 1;
    }
    assert_eq!(seen, 7);
}

/// With display facts, the read carries the person's client message id beside
/// their message and the calls' times beside the assistant's; the reader's rows
/// show both. Without the opt-in, the same read carries neither.
#[test]
fn broca_display_facts_reach_the_rows() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    let rows_of = |name: &str| {
        let page: RawPage = serde_json::from_value(result(name)).unwrap();
        source.rows(&page.messages, false, &[]).unwrap()
    };
    let with = rows_of("display-facts-samples/opt-in-read.json");
    assert_eq!(with[0].display["clientMessageId"], "phone_facts_1");
    let timed = with[1].display["parts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part.get("startedAtMs").is_some());
    assert!(timed, "{}", with[1].display);
    let without = rows_of("display-facts-samples/default-read.json");
    assert!(without[0].display.get("clientMessageId").is_none());
}

/// Display history rows decode as the reader reads them, with `sender` naming
/// owners and named senders for run starts, steers and queued sends alike.
#[test]
fn broca_history_rows_with_senders_decode() {
    for name in [
        "display-facts-samples/history.json",
        "finish-sender-samples/sender/matrix/history.json",
        "finish-sender-samples/sender/owner-steer/history.json",
    ] {
        let page: DisplayPage =
            serde_json::from_value(result(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(!page.messages.is_empty(), "{name}");
    }
}
