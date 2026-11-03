//! Broca's `provider_fallback` part read through every path that decodes a Broca
//! session (`provider-fallback-1028`).
//!
//! When Anthropic hands a reply from one model to another, Broca stores a
//! `provider_fallback` part between the reply's other parts: `from_model`,
//! `to_model` and the provider's own block as `raw`, which Broca echoes back to the
//! provider. It has no text for the model or the person and no runner tag. The
//! samples under `occipital-app-reader/tests/fixtures/broca/provider-fallback-1028/`
//! are Broca's real serve-path replies on a synthetic session; the decoding checks
//! below go through the production decode functions.
//!
//! Covered paths: raw `session.read` entry decoding in both media modes
//! (`broca_entries::rows`), and the served newest page, claim and exact read
//! (`AppReader` over a Broca source). The raw `session.history` form has no consumer
//! here: the `clientMessageId` echo reads display form only.

use super::*;
use crate::app_reader::tests::serve;
use crate::app_reader::TRANSCRIPT_PAGE;
use occipital_app_reader::broca_entries;
use sha2::{Digest, Sha256};

const FIXTURES: &str = "../occipital-app-reader/tests/fixtures/broca/provider-fallback-1028";
const FILES: [&str; 3] = [
    "session.read.default.json",
    "session.read.descriptors.json",
    "session.history.default.json",
];

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES)
}

/// A sample's entries, recorded under runner contract 0.7 and read as a runner on
/// the current contract serves them (`current_contract`).
fn entries(name: &str) -> Vec<Value> {
    let bytes = std::fs::read(fixture_dir().join(name)).unwrap();
    current_contract(serde_json::from_slice::<Value>(&bytes).unwrap())["result"]["messages"]
        .as_array()
        .unwrap()
        .clone()
}

/// No part shown to the phone carries the fallback part or its provider block.
fn assert_no_fallback(display: &Value) {
    let text = display.to_string();
    assert!(!text.contains("provider_fallback"), "{text}");
    assert!(!text.contains("claude-opus-4-8"), "{text}");
}

#[test]
fn the_fallback_samples_are_complete_and_their_checksums_verify() {
    let sums = std::fs::read_to_string(fixture_dir().join("SHA256SUMS")).unwrap();
    let mut listed: Vec<String> = sums
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(fixture_dir().join(name)).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest, "{name}");
            name.to_owned()
        })
        .collect();
    listed.sort();
    let mut expected = FILES.map(str::to_owned).to_vec();
    expected.sort();
    assert_eq!(listed, expected);
}

/// The fallback part is left off the phone's row, and every other part keeps its
/// stored position: the reasoning after it is part 2, the call part 3.
#[test]
fn the_fallback_part_is_left_off_the_row_in_both_media_modes() {
    for name in ["session.read.default.json", "session.read.descriptors.json"] {
        let entries = entries(name);
        assert_eq!(
            entries[1]["message"]["content"][1]["type"], "provider_fallback",
            "{name}: the sample carries the part"
        );
        for full in [false, true] {
            let rows = broca_entries::rows(&entries, full).unwrap();
            assert_eq!(
                rows.iter().map(|row| row.mid.as_str()).collect::<Vec<_>>(),
                ["m0", "m1", "m2", "m3"],
                "{name} full={full}"
            );
            let parts = rows[1].display["parts"].as_array().unwrap();
            assert_eq!(
                parts
                    .iter()
                    .map(|part| part["partIndex"].as_u64().unwrap())
                    .collect::<Vec<_>>(),
                [0, 2, 3],
                "{name} full={full}: {parts:?}"
            );
            assert_eq!(parts[2]["type"], "tool", "{name} full={full}");
            for row in &rows {
                assert_no_fallback(&row.display);
            }
        }
    }
}

/// The served newest page, a claim and an exact read of the reply all show it
/// without the fallback part.
#[tokio::test]
async fn the_served_page_claim_and_exact_read_leave_the_fallback_part_off() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = entries("session.read.default.json");
    let (_, app) = serving(route);
    let params = json!({"session": "alfonso:s1", "harness": "broca", "limit": 4});
    let page: Value = serde_json::from_slice::<Value>(&serve(&app, TRANSCRIPT_PAGE, &params).await)
        .unwrap()["result"]
        .clone();
    assert_eq!(page["messages"].as_array().unwrap().len(), 4, "{page}");
    assert_no_fallback(&page);
    let mut claimed = params.clone();
    claimed["claim"] = json!({
        "heldFloorMessageId": "m0",
        "sinceMessageId": "m0",
        "heldVersion": page["version"],
    });
    let reply: Value =
        serde_json::from_slice::<Value>(&serve(&app, TRANSCRIPT_PAGE, &claimed).await).unwrap()
            ["result"]
            .clone();
    assert_eq!(reply["claim"], "matched", "{reply}");
    assert_no_fallback(&reply);
    let mut exact = params;
    exact["messageOrdinal"] = json!(1);
    let read: Value = serde_json::from_slice::<Value>(&serve(&app, TRANSCRIPT_PAGE, &exact).await)
        .unwrap()["result"]
        .clone();
    assert_eq!(read["fullContent"], true, "{read}");
    assert_no_fallback(&read);
}
