//! Broca's reads of one session before and after Broca serves replies without
//! the tags the model wrote in them (`§2§`, the numbers Magic Context shows the
//! model), recorded from its real serve path and copied unchanged into
//! `tests/data/broca-reply-tags-1112/`. `base/` is the build before, `fix/` the
//! build after; the README there lists every file.
//!
//! Occipital builds rows from the stored text these reads serve, so after the
//! fix the apps get the replies without the tags, with no change here. Broca
//! raises the session's history counter by one at the same time, which changes
//! the page version the apps hold, so each app reloads such a chat once and
//! drops the copy that still shows the tags.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-reply-tags-1112")
}

/// A recorded reply's `result`.
fn result(side: &str, name: &str) -> Value {
    let path = root().join(side).join(format!("{name}.json"));
    let body: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    runner_result(body).unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"))
}

/// The reads Occipital builds rows from.
const READS: &[&str] = &[
    "session-read-inline",
    "session-read-blocks",
    "session-read-media_refs",
    "session-read-descriptors",
    "session-history-raw",
];

/// Each row's role and text, as the apps get them.
fn rows(side: &str, name: &str) -> Vec<(String, String)> {
    let page: RawPage = serde_json::from_value(result(side, name))
        .unwrap_or_else(|error| panic!("{side}/{name}: {error}"));
    BrocaSource::new(resolved(), FakeRoute::new())
        .rows(&page.messages, false, &page.pruned_reasoning)
        .unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"))
        .into_iter()
        .map(|row| {
            (
                row.display["role"].as_str().unwrap_or_default().to_owned(),
                row.display["text"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_reply_tag_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 22);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Every read builds the same rows before and after, except the replies: before,
/// three of them carry the model's tags; after, none does, and each reads as the
/// model wrote it without them.
#[test]
fn replies_reach_the_apps_without_the_models_tags() {
    let replies = [
        "Done.",
        "let me look",
        "It says file contents.",
        "As  shows, the file  holds it.",
    ];
    for name in READS {
        let before = rows("base", name);
        let after = rows("fix", name);
        assert_eq!(before.len(), after.len(), "{name}");
        let tagged = |text: &str| text.contains('§');
        assert_eq!(
            before.iter().filter(|(_, text)| tagged(text)).count(),
            3,
            "{name}: {before:?}"
        );
        assert!(
            !after.iter().any(|(_, text)| tagged(text)),
            "{name}: {after:?}"
        );
        let answered: Vec<&str> = after
            .iter()
            .filter(|(role, _)| role == "assistant")
            .map(|(_, text)| text.as_str())
            .collect();
        assert_eq!(answered, replies, "{name}");
        for ((role, before), (_, after)) in before.iter().zip(&after) {
            if role != "assistant" {
                assert_eq!(before, after, "{name}");
            }
        }
    }
}

/// The session's history counter, on `session.head` and every read, rises by
/// one, so the page version every app holds for the chat changes and the app
/// reloads it once.
#[test]
fn the_history_counter_rises_once_so_the_apps_reload() {
    let before = head_meta(&result("base", "session-head")).unwrap();
    let after = head_meta(&result("fix", "session-head")).unwrap();
    assert_eq!((before.history_version, after.history_version), (0, 1));
    for name in READS {
        assert_eq!(result("fix", name)["history_version"], 1, "{name}");
    }
}
