//! Broca's fixed notice writer, recorded from its real serializers, through
//! every reader path that decodes it. Copied unchanged into
//! `tests/data/broca-notice-writer-1107/` from the set Broca published for its
//! readers (its `SHA256SUMS` hashes to `08ec7c162e515608…`); the README
//! there is Occipital's own. Before the fix, the tool-result message that carried three notices
//! was marked `delivered_as: {mode: "tool_result"}`, as if the ride had made a
//! transcript entry; after it, the carrier is unmarked. Either way the carrier
//! is the tool's own result: it stays on pages, its result reaches its call,
//! and its notices come only from the `notices` fact and the live
//! `notice_delivered` event.

use super::*;
use cortexkit_test_support::ScratchDir;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-notice-writer-1107")
}

fn body(side: &str, name: &str) -> Value {
    let bytes = std::fs::read(root().join(side).join(format!("0811.{name}.json"))).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// A recorded reply's `result`.
fn result(side: &str, name: &str) -> Value {
    runner_result(body(side, name)).unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"))
}

const SIDES: [&str; 2] = ["before", "after"];
const READS: [&str; 3] = [
    "session.read.default",
    "session.read.facts",
    "session.history.raw",
];

#[test]
fn the_notice_writer_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 14);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Only the carrier's entry-level mark differs between the sides.
#[test]
fn the_fix_unmarks_only_the_carrier() {
    for side in SIDES {
        for name in READS {
            let page: RawPage = serde_json::from_value(result(side, name)).unwrap();
            let marked: Vec<_> = page
                .messages
                .iter()
                .filter(|entry| entry.get("delivered_as").is_some())
                .map(|entry| entry["mid"].as_str().unwrap())
                .collect();
            let expected: &[&str] = if side == "before" { &["m2"] } else { &[] };
            assert_eq!(marked, expected, "{side}/{name}");
        }
    }
}

/// Every read builds a row for each entry; on the newest page the carrier
/// stays, and its result reaches the call it answers.
#[tokio::test]
async fn the_carrier_stays_on_pages_and_answers_its_call() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    for side in SIDES {
        for name in READS {
            let page: RawPage = serde_json::from_value(result(side, name)).unwrap();
            let rows = source
                .rows(&page.messages, false, &page.pruned_reasoning)
                .unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"));
            assert_eq!(rows.len(), page.messages.len(), "{side}/{name}");
        }
        let page: RawPage = serde_json::from_value(result(side, "session.read.facts")).unwrap();
        let route = FakeRoute::new();
        *route.entries.lock().unwrap() = page.messages;
        let newest = BrocaSource::new(resolved(), route)
            .newest(20)
            .await
            .unwrap();
        let mids: Vec<_> = newest.rows.iter().map(|row| row.mid.as_str()).collect();
        assert_eq!(mids, ["m0", "m1", "m2", "m3"], "{side}");
        let call = &newest.rows[1].display["parts"][0];
        assert_eq!(call["type"], "tool", "{side}: {call}");
        assert!(
            call["outputBytes"].as_u64().is_some_and(|bytes| bytes > 0),
            "{side}: {call}"
        );
        let display: DisplayPage =
            serde_json::from_value(result(side, "session.history.display")).unwrap();
        assert_eq!(display.messages.len(), 4, "{side}");
    }
}

/// With display facts, the three notices that rode the call are found on the
/// carrier under the call's key, in delivery order, the same on both sides.
/// Without display facts there are none.
#[test]
fn the_notices_come_from_the_fact_alone() {
    for side in SIDES {
        let page: RawPage = serde_json::from_value(result(side, "session.read.facts")).unwrap();
        let call_key = page.messages[2]["notices"][0]["call_key"]
            .as_str()
            .unwrap()
            .to_owned();
        let notices = crate::notices::from_entries(&page.messages, "broca", "alfonso:s1");
        let found: Vec<_> = notices
            .iter()
            .map(|notice| {
                (
                    notice.call_key.as_str(),
                    notice.text.as_str(),
                    notice.position,
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (call_key.as_str(), "  synthetic notice\n☃\n", Some(0)),
                (call_key.as_str(), "second notice", Some(1)),
                (call_key.as_str(), "unkeyed notice", Some(2)),
            ],
            "{side}"
        );
        let plain: RawPage = serde_json::from_value(result(side, "session.read.default")).unwrap();
        assert!(crate::notices::from_entries(&plain.messages, "broca", "alfonso:s1").is_empty());
    }
}

/// Every live frame decodes, and none ends the source's live subscription;
/// `notice_delivered` reaches the notices kept for the apps under the call it
/// rode.
#[tokio::test]
async fn the_live_frames_keep_the_feed_and_deliver_the_notice() {
    for side in SIDES {
        let frames = body(side, "subscribe.envelope");
        let source = BrocaSource::new(resolved(), FakeRoute::new());
        for (index, frame) in frames.as_array().unwrap().iter().enumerate() {
            let bytes = serde_json::to_vec(frame).unwrap();
            let frame = decode_subscription_frame(&bytes)
                .unwrap_or_else(|error| panic!("{side}:{index}: {error:?}"));
            if let Some(normalized) = normalize(frame.clone()) {
                let _ = display_events::map(&normalized);
            }
            assert!(
                source.observe(frame),
                "{side}:{index} ended the subscription"
            );
        }
    }

    let dir = ScratchDir::new("occipital-broca-notice-writer");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, _task) = crate::notices::Notices::start(file.clone());
    let slot = crate::notices::NoticeSlot::default();
    let _ = slot.set(notices);
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    *source.notices.guard() = slot;
    let delivered = body("after", "notice_delivered.frame");
    let call_key = delivered["event"]["call_key"].as_str().unwrap().to_owned();
    let frame = decode_subscription_frame(&serde_json::to_vec(&delivered).unwrap()).unwrap();
    assert!(source.observe(frame));
    let mut kept = Vec::new();
    for _ in 0..1_000 {
        kept = file.for_call("broca", "alfonso:s1", &call_key).unwrap();
        if !kept.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let texts: Vec<_> = kept.iter().map(|kept| kept.text.as_str()).collect();
    assert_eq!(texts, [delivered["event"]["text"].as_str().unwrap()]);
}
