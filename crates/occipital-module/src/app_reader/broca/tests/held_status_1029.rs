//! Broca's `session.subscribe` frames before and after it reported held tool calls as a
//! durable status, from the sample set named below.
//!
//! The samples under `occipital-app-reader/tests/fixtures/broca/held-calls-status-1029/`
//! are Broca's real serve-path frames, live and on attach replay, one frame per line.
//! After the change a run waiting on a held call sends a `control` frame whose unit is
//! `{type: status}`, with `held_calls`, and no `event` member; settlement sends another
//! status without `held_calls`. Such a frame carries no live event, so it must keep the
//! feed and have no display effect: a held call is neither a turn end nor a reset.

use super::*;
use occipital_app_reader::display_events;
use sha2::{Digest, Sha256};

const FIXTURES: &str = "../occipital-app-reader/tests/fixtures/broca/held-calls-status-1029";
const CASES: [&str; 10] = [
    "status-expired-listed-live",
    "status-expired-listed-replay",
    "status-expired-settled-live",
    "status-expired-settled-replay",
    "status-late-listed-live",
    "status-late-listed-replay",
    "status-late-settled-live",
    "status-late-settled-replay",
    "status-ordinary-settled-live",
    "status-ordinary-settled-replay",
];

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURES)
            .join(name),
    )
    .unwrap()
}

fn frames(side: &str, case: &str) -> Vec<Vec<u8>> {
    bytes(&format!("{side}/{case}.jsonl"))
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

#[test]
fn the_held_status_samples_are_complete_and_their_checksums_verify() {
    let sums = String::from_utf8(bytes("SHA256SUMS")).unwrap();
    let listed: Vec<&str> = sums
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes(name))),
                digest,
                "{name}"
            );
            name
        })
        .collect();
    assert_eq!(listed.len(), 20);
}

/// Every frame, before and after, decodes and keeps the feed. A held-call status frame
/// has no display effect, and status frames appear only in the after-change held cases.
#[test]
fn a_held_call_status_keeps_the_feed_and_never_touches_the_display() {
    for side in ["before", "after"] {
        for case in CASES {
            let mut statuses = 0;
            let source = BrocaSource::new(resolved(), FakeRoute::new());
            for line in frames(side, case) {
                let frame = decode_subscription_frame(&line)
                    .unwrap_or_else(|e| panic!("{side}/{case}: {e:?}"));
                if frame["unit"]["type"] == "status" {
                    statuses += 1;
                    assert!(frame.get("event").is_none(), "{side}/{case}: {frame}");
                    let effect = normalize(frame.clone()).and_then(|n| display_events::map(&n));
                    assert!(
                        effect.is_none(),
                        "{side}/{case}: a held-call status changed the display: {effect:?}"
                    );
                }
                assert!(
                    source.observe(frame.clone()),
                    "{side}/{case}: feed ended at {frame}"
                );
            }
            let held = case.contains("-listed-")
                || case.contains("-settled-") && !case.contains("ordinary");
            match (side, held) {
                ("after", true) => assert!(statuses > 0, "{side}/{case}: no status frame"),
                _ => assert_eq!(statuses, 0, "{side}/{case}: unexpected status frames"),
            }
        }
    }
}
