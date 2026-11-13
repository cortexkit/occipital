//! Broca serving runner contract 0.8.46's `model_fallback` on a reply its
//! refusal fallback answered, recorded from its real serializers on a
//! synthetic session, before and after. Copied unchanged into
//! `tests/data/broca-model-fallback-1113/` from the set Broca published for its
//! readers; its README says what each file is. The page holds two replies:
//! one the requested model wrote, and one written by the fallback model after
//! the requested one refused.
//!
//! Broca takes the fact from its own records of the run, never from a marker
//! block a host writes into the reply (a `host_parts` fact), so its replies
//! carry no such blocks. A read asking for display facts names the fallback;
//! the apps' row for that reply carries it as `modelFallback`, and every other
//! row and every default read carries nothing.

use super::served_samples::{decoded, Served};
use super::tool_display_samples::AnyHarness;
use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-model-fallback-1113")
}

fn page(side: &str, name: &str) -> RawPage {
    let bytes = std::fs::read(root().join(side).join(format!("{name}.json"))).unwrap();
    let value = runner_result(serde_json::from_slice(&bytes).unwrap())
        .unwrap_or_else(|e| panic!("{side}/{name}: {e:?}"));
    serde_json::from_value(value).unwrap_or_else(|e| panic!("{side}/{name}: {e}"))
}

#[test]
fn the_model_fallback_samples_keep_their_checksums() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("515d10bef2bb4686"));
    let sums = String::from_utf8(sums).unwrap();
    assert_eq!(sums.lines().count(), 6);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The apps' `modelFallback` for each row of one read, by ordinal.
fn fallbacks(side: &str, name: &str) -> Vec<(u64, Value)> {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    let page = page(side, name);
    let rows = source
        .rows(&page.messages, false, &page.pruned_reasoning)
        .unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"));
    assert_eq!(rows.len(), 4, "{side}/{name}");
    page.messages
        .iter()
        .zip(&rows)
        .map(|(entry, row)| {
            (
                entry["ordinal"].as_u64().unwrap(),
                row.display["modelFallback"].clone(),
            )
        })
        .collect()
}

/// Only the after read with display facts names a fallback, on the fourth
/// row alone, and with it the rows carry nothing they didn't before.
#[test]
fn only_the_fallback_reply_carries_model_fallback() {
    let none = |ordinal| (ordinal, Value::Null);
    for (side, name) in [
        ("before", "session-read"),
        ("before", "session-read-with-display-facts"),
        ("after", "session-read"),
    ] {
        assert_eq!(
            fallbacks(side, name),
            [none(1), none(2), none(3), none(4)],
            "{side}/{name}"
        );
    }
    assert_eq!(
        fallbacks("after", "session-read-with-display-facts"),
        [
            none(1),
            none(2),
            none(3),
            (
                4,
                json!({"requested": "stub/m", "answered": "stub/claude-opus-4-8"})
            )
        ]
    );
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    let rows = |side| {
        let page = page(side, "session-read-with-display-facts");
        let mut rows: Vec<Value> = source
            .rows(&page.messages, false, &page.pruned_reasoning)
            .unwrap()
            .into_iter()
            .map(|row| row.display)
            .collect();
        rows[3].as_object_mut().unwrap().remove("modelFallback");
        // Independent runs mint their own message ids and times.
        for row in &mut rows {
            for key in ["messageId", "createdAtMs", "completedAtMs"] {
                row.as_object_mut().unwrap().remove(key);
            }
        }
        rows
    };
    assert_eq!(rows("after"), rows("before"));
}

/// Through the real handlers, the apps' newest page carries the note on the
/// fallback reply.
#[tokio::test]
async fn the_newest_page_carries_the_note() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = page("after", "session-read-with-display-facts").messages;
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(Hosts("broca", "broca")))),
            Arc::new(AnyHarness(route.clone())),
        ),
    )));
    let mut served = Served::new("broca-model-fallback-1113", app);
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": "ses_head", "harness": "broca", "limit": 50}});
    let page = served.read(1, &request).await;
    let rows = decoded(&page)["result"]["messages"].clone();
    let notes: Vec<&Value> = rows
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row.get("modelFallback"))
        .collect();
    assert_eq!(
        notes,
        [&json!({"requested": "stub/m", "answered": "stub/claude-opus-4-8"})]
    );
}
