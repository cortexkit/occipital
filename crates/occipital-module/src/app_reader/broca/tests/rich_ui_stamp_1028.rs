//! Broca's rich UI stamp (llm-runner `rich_ui_stamp`) read through every reader path
//! that decodes the surfaces it changes (`rich-ui-stamp-1028`).
//!
//! The samples under `occipital-app-reader/tests/fixtures/broca/rich-ui-stamp-1028/` are
//! Broca's real serve-path replies for a synthetic stamped run: `session.read` with and
//! without `include_rich_ui`, display-form `session.history`, `session.head`, and the
//! `run_started` subscribe frame. The decoding checks below go through the production
//! decode functions.

use super::*;
use crate::app_reader::tests::serve;
use crate::app_reader::TRANSCRIPT_PAGE;
use occipital_app_reader::{broca_entries, display_events};
use sha2::{Digest, Sha256};

const FIXTURES: &str = "../occipital-app-reader/tests/fixtures/broca/rich-ui-stamp-1028";
const FILES: [&str; 5] = [
    "run_started.json",
    "session.head.json",
    "session.history.json",
    "session.read.default.json",
    "session.read.opt-in.json",
];

/// A sample recorded under runner contract 0.7, read as a runner on the current
/// contract serves it (`current_contract`).
fn sample(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURES)
        .join(name);
    current_contract(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
}

fn entries(name: &str) -> Vec<Value> {
    sample(name)["result"]["messages"]
        .as_array()
        .unwrap()
        .clone()
}

fn stamp() -> Value {
    json!({"dialect": "1.0", "catalogDigest": "a".repeat(64)})
}

#[test]
fn the_stamp_samples_are_complete_and_their_checksums_verify() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES);
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
    let mut listed: Vec<String> = sums
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(dir.join(name)).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest, "{name}");
            name.to_owned()
        })
        .collect();
    listed.sort();
    assert_eq!(listed, FILES);
}

/// The opted-in read stamps only the assistant row; the default read's rows are the
/// same rows without the stamp.
#[test]
fn the_opted_in_read_stamps_the_assistant_row_and_the_default_read_matches_without_it() {
    for full in [false, true] {
        let stamped = broca_entries::rows(&entries("session.read.opt-in.json"), full).unwrap();
        assert_eq!(
            stamped[0].display.get("richUi"),
            None,
            "user row, full={full}"
        );
        assert_eq!(stamped[1].display["richUi"], stamp(), "full={full}");
        let plain = broca_entries::rows(&entries("session.read.default.json"), full).unwrap();
        assert_eq!(stamped.len(), plain.len());
        for (stamped, plain) in stamped.iter().zip(&plain) {
            let mut without = stamped.display.clone();
            without.as_object_mut().unwrap().remove("richUi");
            assert_eq!(without, plain.display, "full={full}");
        }
    }
}

/// A served newest page from a runner declaring the group carries the stamp on the
/// assistant row only.
#[tokio::test]
async fn the_served_page_carries_the_stamp_on_the_assistant_row() {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["rich_ui_stamp"]));
    *route.entries.lock().unwrap() = entries("session.read.opt-in.json");
    let (_, app) = serving(route);
    let params = json!({"session": "alfonso:s1", "harness": "broca", "limit": 4});
    let page: Value = serde_json::from_slice::<Value>(&serve(&app, TRANSCRIPT_PAGE, &params).await)
        .unwrap()["result"]
        .clone();
    let rows = page["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{page}");
    assert_eq!(rows[0].get("richUi"), None, "{page}");
    assert_eq!(rows[1]["richUi"], stamp(), "{page}");
}

/// The display-form history the `clientMessageId` echo reads, and the head the page
/// facts read, both decode with the stamp present.
#[test]
fn the_history_and_head_decode_with_the_stamp() {
    let history: DisplayPage =
        serde_json::from_value(sample("session.history.json")["result"].clone()).unwrap();
    assert_eq!(history.messages.len(), 2);
    let head: cortexkit_role_llm_runner::read::HeadMeta =
        serde_json::from_value(sample("session.head.json")["result"].clone()).unwrap();
    assert!(head.last_run_rich_ui.is_some());
}

/// The `run_started` frame starts the live turn with the run's stamp, in the phone's
/// shape, and keeps the feed.
#[test]
fn the_run_started_frame_starts_the_live_turn() {
    let bytes = serde_json::to_vec(&sample("run_started.json")).unwrap();
    let frame = decode_subscription_frame(&bytes).unwrap();
    let normalized = normalize(frame.clone()).unwrap();
    assert_eq!(
        display_events::map(&normalized),
        Some(display_events::DisplayInput::RunStart {
            rich_ui: Some(stamp())
        })
    );
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    assert!(source.observe(frame));
}
