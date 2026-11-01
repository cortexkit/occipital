//! Broca's `session.subscribe` control frames before and after it flattened
//! `text_with_state` to `text` in `assistant_message` units (`subscribe-flatten-1028`).
//!
//! The samples under `occipital-app-reader/tests/fixtures/broca/subscribe-flatten-1028/`
//! are Broca's real serve-path frames on synthetic data, for OpenAI Responses and Gemini
//! answers, live and on attach replay. The subscribe pump reads only a control frame's
//! typed `event`, never the unit's content parts, so both shapes must reach it alike.

use super::*;
use occipital_app_reader::display_events;
use sha2::{Digest, Sha256};

const FIXTURES: &str = "../occipital-app-reader/tests/fixtures/broca/subscribe-flatten-1028";
const CASES: [&str; 4] = [
    "gemini.live",
    "gemini.replay",
    "responses.live",
    "responses.replay",
];

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURES)
            .join(name),
    )
    .unwrap()
}

#[test]
fn the_flatten_samples_are_complete_and_their_checksums_verify() {
    let sums = String::from_utf8(bytes("SHA256SUMS")).unwrap();
    let mut listed: Vec<String> = sums
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes(name))),
                digest,
                "{name}"
            );
            name.to_owned()
        })
        .collect();
    listed.sort();
    assert_eq!(listed.len(), 8);
}

/// Before and after the change, each frame keeps the feed, at most ends its message's
/// live words, and normalizes to the same typed event.
#[test]
fn both_shapes_reach_the_pump_alike() {
    for case in CASES {
        let mut events = Vec::new();
        for side in ["before", "after"] {
            let frame = decode_subscription_frame(&bytes(&format!("{side}.{case}.json"))).unwrap();
            let normalized = normalize(frame.clone()).unwrap();
            assert_eq!(
                normalized["event"]["type"], "message_finished",
                "{side}.{case}"
            );
            assert!(
                matches!(
                    display_events::map(&normalized),
                    None | Some(display_events::DisplayInput::MessageEnd { .. })
                ),
                "{side}.{case}"
            );
            let source = BrocaSource::new(resolved(), FakeRoute::new());
            assert!(source.observe(frame), "{side}.{case}");
            events.push(normalized["event"].clone());
        }
        assert_eq!(events[0], events[1], "{case}");
    }
}
