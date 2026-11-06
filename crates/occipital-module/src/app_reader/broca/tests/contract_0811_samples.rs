//! Broca's replies after it adopts runner contract 0.8.11, recorded from its
//! serve path, through every reader path that decodes them. Copied unchanged
//! into `tests/data/broca-contract-0.8.11/` from the set Broca published for
//! its readers (its `SHA256SUMS` hashes to `61993a1122bb5179…`), with Broca's
//! README, which says how each pair was captured. Before and after the change:
//! - `role.describe` declares `notices`.
//! - Cursors on reads, heads and the live stream carry the log's incarnation
//!   and a frame digest.
//! - With display facts, a module's message names its sender, the tool-result
//!   message lists the notice that rode the call, and a denied call carries its
//!   call key.
//! - The live stream sends `notice_delivered` with the notice's text.

use super::*;
use cortexkit_test_support::ScratchDir;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-contract-0.8.11")
}

fn bytes(side: &str, name: &str) -> Vec<u8> {
    std::fs::read(root().join(side).join(name)).unwrap()
}

/// A recorded reply's `result`.
fn result(side: &str, name: &str) -> Value {
    let body = serde_json::from_slice(&bytes(side, name)).unwrap();
    runner_result(body).unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"))
}

/// The call key Broca minted for the call a notice rode, in the main pair.
const CARRYING_CALL: &str = "3032d37b-1fc9-44c8-a32b-882150f0b9f8:5";

#[test]
fn the_contract_0811_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 57);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Both describes pass the contract's check the reader requires and the plain
/// decode the send path uses. Only the later one declares notices.
#[test]
fn broca_0811_describes_decode_on_both_paths() {
    for (side, notices) in [("before", false), ("after", true)] {
        let value = result(side, "role.describe.json");
        let checked = check_describe(&value).unwrap_or_else(|error| panic!("{side}: {error:?}"));
        let plain: RoleDescribe = serde_json::from_value(value).unwrap();
        for describe in [&checked, &plain] {
            for group in ["read_display_facts", "message_senders", "head_sender"] {
                assert!(describe.declares(group), "{side}: {group}");
            }
            assert_eq!(describe.declares("notices"), notices, "{side}");
        }
    }
}

/// Each head, its cursor now naming the log's incarnation and a frame digest,
/// passes the contract's rule.
#[test]
fn broca_0811_heads_pass_the_contract_rule() {
    for side in ["before", "after"] {
        head_meta(&result(side, "session.head.json"))
            .unwrap_or_else(|error| panic!("{side}: {error:?}"));
    }
}

/// Every read and raw history page decodes and builds rows, with and without
/// display facts, and the display history decodes too: for a runner without
/// display facts, that's where the reader finds which app message each
/// person's row came from.
#[test]
fn broca_0811_pages_build_rows() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    for side in ["before", "after", "before-denied", "after-denied"] {
        let names: &[&str] = if side.ends_with("denied") {
            &["session.read.default.json", "session.read.facts.json"]
        } else {
            &[
                "session.read.default.json",
                "session.read.facts.json",
                "session.history.raw.json",
            ]
        };
        for name in names {
            let page: RawPage = serde_json::from_value(result(side, name))
                .unwrap_or_else(|error| panic!("{side}/{name}: {error}"));
            let rows = source
                .rows(&page.messages, false, &page.pruned_reasoning)
                .unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"));
            assert_eq!(rows.len(), page.messages.len(), "{side}/{name}");
        }
    }
    for side in ["before", "after"] {
        let page: DisplayPage =
            serde_json::from_value(result(side, "session.history.display.json"))
                .unwrap_or_else(|error| panic!("{side}: {error}"));
        assert_eq!(page.messages.len(), 4, "{side}");
    }
}

/// With display facts, the module's message reads labelled with its module,
/// the call carries its key, and the notice that rode it is found on the
/// tool-result message under that key. Without them, or before the change,
/// none of that is there.
#[test]
fn broca_0811_display_facts_reach_the_rows_and_notices() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    let read = |side: &str, name: &str| {
        let page: RawPage = serde_json::from_value(result(side, name)).unwrap();
        let rows = source.rows(&page.messages, false, &[]).unwrap();
        (page.messages, rows)
    };
    let (entries, rows) = read("after", "session.read.facts.json");
    assert_eq!(
        rows[0].display["sentBy"],
        json!({"module": "sample-sender"})
    );
    let call = &rows[1].display["parts"][0];
    assert_eq!(call["type"], "tool", "{call}");
    assert_eq!(call["callKey"], CARRYING_CALL, "{call}");
    let notices = crate::notices::from_entries(&entries, "broca", "alfonso:s1");
    let placed: Vec<_> = notices
        .iter()
        .map(|notice| {
            (
                notice.call_key.as_str(),
                notice.sender.as_str(),
                notice.text.as_str(),
                notice.delivered_as.as_str(),
                notice.position,
            )
        })
        .collect();
    assert_eq!(
        placed,
        [(
            CARRYING_CALL,
            "sample-sender",
            "synthetic notice ☃",
            "tool_result",
            Some(0)
        )]
    );
    for (side, name) in [
        ("after", "session.read.default.json"),
        ("before", "session.read.facts.json"),
        ("before", "session.read.default.json"),
    ] {
        let (entries, rows) = read(side, name);
        assert!(rows[0].display.get("sentBy").is_none(), "{side}/{name}");
        assert!(
            crate::notices::from_entries(&entries, "broca", "alfonso:s1").is_empty(),
            "{side}/{name}"
        );
    }
}

/// A denied call carries its key with display facts, beside its `denied`
/// answer, so a notice or a record can find it.
#[test]
fn broca_0811_a_denied_call_carries_its_key() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    let calls = |side: &str| {
        let page: RawPage =
            serde_json::from_value(result(side, "session.read.facts.json")).unwrap();
        let rows = source.rows(&page.messages, false, &[]).unwrap();
        rows[1].display["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|part| (part["callKey"].clone(), part["unanswered"].clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        calls("after-denied"),
        [
            (
                json!("582515cc-1489-4df6-8eee-b4dd0a716d11:5"),
                json!("denied")
            ),
            (
                json!("582515cc-1489-4df6-8eee-b4dd0a716d11:6"),
                json!("denied")
            ),
        ]
    );
    assert!(calls("before-denied").iter().all(|(key, _)| key.is_null()));
}

fn frames(side: &str) -> Vec<Value> {
    let prefix = if side.ends_with("denied") {
        "live"
    } else {
        "subscribe"
    };
    let mut index = 0;
    let mut frames = Vec::new();
    while root()
        .join(side)
        .join(format!("{prefix}.{index}.json"))
        .exists()
    {
        let name = format!("{prefix}.{index}.json");
        frames.push(
            decode_subscription_frame(&bytes(side, &name))
                .unwrap_or_else(|error| panic!("{side}/{name}: {error:?}")),
        );
        index += 1;
    }
    frames
}

/// Every live frame, cursors with incarnation and digest included, decodes,
/// maps and keeps the feed. The later stream's `notice_delivered` reaches the
/// notices kept for the apps, with the call it rode and its exact text, and
/// changes no message.
#[tokio::test]
async fn broca_0811_live_frames_keep_the_feed_and_deliver_the_notice() {
    for side in ["before", "after", "before-denied", "after-denied"] {
        let source = BrocaSource::new(resolved(), FakeRoute::new());
        for (index, frame) in frames(side).into_iter().enumerate() {
            if let Some(normalized) = normalize(frame.clone()) {
                let _ = display_events::map(&normalized);
            }
            assert!(source.observe(frame), "{side}:{index} ended the feed");
        }
    }

    let dir = ScratchDir::new("occipital-broca-0811-notice");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, _task) = crate::notices::Notices::start(file.clone());
    let slot = crate::notices::NoticeSlot::default();
    let _ = slot.set(notices);
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    *source.notices.guard() = slot;
    for frame in frames("after") {
        assert!(source.observe(frame));
    }
    let mut kept = Vec::new();
    for _ in 0..1_000 {
        kept = file.for_call("broca", "alfonso:s1", CARRYING_CALL).unwrap();
        if !kept.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let kept: Vec<_> = kept
        .iter()
        .map(|kept| (kept.sender.as_str(), kept.text.as_str()))
        .collect();
    assert_eq!(kept, [("sample-sender", "synthetic notice ☃")]);
}
