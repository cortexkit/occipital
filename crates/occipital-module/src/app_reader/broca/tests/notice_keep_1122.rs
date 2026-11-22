//! Broca's samples for a kept notice (runner contract `notice_keep`): the
//! session's owner sends a notice with `keep: true`, words no compaction
//! provider may drop on its own. Recorded from Broca's real serve loop and
//! copied unchanged into `tests/data/broca-notice-keep-1122/` with its README.
//!
//! Occipital reads three of them: the opted-in `session.read` it builds rows
//! from, `session.history` (raw to find a claimed message's ordinal, display
//! for the app's message ids), and `role.describe`. A kept message carries
//! `keep: true` beside `notice: true`. Keeping is about what a compaction
//! provider may drop, not about how a row draws, so a kept notice reaches the
//! apps exactly as the same notice unkept.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-notice-keep-1122")
}

fn result(name: &str) -> Value {
    let body: Value = serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap();
    runner_result(body).unwrap()
}

/// The same body with every `keep` member removed, as Broca served it before
/// kept notices existed.
fn unkept(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            map.remove("keep");
            for item in map.values_mut() {
                *item = unkept(item.take());
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                *item = unkept(item.take());
            }
        }
        _ => {}
    }
    value
}

#[test]
fn the_notice_keep_samples_keep_their_checksums() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("8dd50cfa"));
    let sums = String::from_utf8(sums).unwrap();
    // Eleven bodies; the README is beside them, outside the list.
    assert_eq!(sums.lines().count(), 11);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

async fn rows_of(read: Value) -> Vec<Value> {
    let page: RawPage = serde_json::from_value(read).unwrap();
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = page.messages;
    let newest = BrocaSource::new(resolved(), route)
        .newest(20)
        .await
        .unwrap();
    newest
        .rows
        .iter()
        .map(|row| json!({"mid": row.mid, "ordinal": row.ordinal, "display": row.display}))
        .collect()
}

/// The newest page from the opted-in read: both kept messages read as
/// prefrontal's notices, and every row is what the read with no `keep`
/// members gives.
#[tokio::test]
async fn a_kept_notice_reads_as_the_same_notice_unkept() {
    let read = result("keep-read.json");
    assert_eq!(read["messages"][1]["keep"], true);
    let kept = rows_of(read.clone()).await;
    assert_eq!(kept.len(), 3);
    for row in &kept[..2] {
        assert!(row["display"].get("notice").is_some(), "{row}");
        assert!(row["display"].get("keep").is_none(), "{row}");
    }
    assert!(kept[2]["display"].get("notice").is_none());
    assert_eq!(kept, rows_of(unkept(read)).await);
}

/// Both `session.history` forms decode as Occipital reads them: the raw form
/// still names the claimed message's ordinal, and the display form still
/// decodes as the contract's display page.
#[test]
fn both_history_forms_decode() {
    let raw: RawPage = serde_json::from_value(result("keep-history-raw.json")).unwrap();
    assert_eq!(raw.messages[1]["keep"], true);
    assert_eq!(raw.messages[1]["ordinal"], 2);
    let display = result("keep-history-display.json");
    assert_eq!(display["messages"][1]["row"]["keep"], true);
    let page: cortexkit_role_llm_runner::display::DisplayPage =
        serde_json::from_value(display).unwrap();
    assert_eq!(page.messages.len(), 3);
}

/// The description declaring `notice_keep` passes the contract's check, and
/// the groups Occipital acts on are still there.
#[test]
fn the_description_with_notice_keep_passes_its_check() {
    let describe =
        cortexkit_role_llm_runner::describe::check_describe(&result("role-describe.json")).unwrap();
    for group in ["notice_keep", "message_marks", "read_display_facts"] {
        assert!(describe.declares(group), "{group}");
    }
}
