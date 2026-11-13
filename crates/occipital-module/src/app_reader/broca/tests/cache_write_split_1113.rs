//! Broca filling the cache-write lifetime split on run usage (llm-runner
//! 0.8.49): `cache_write_5m_tokens` and `cache_write_1h_tokens` beside
//! `cache_write_tokens`, on `run.status` and `run.result` usage and on each
//! `by_model` entry, only when every counted step reported both. Recorded
//! from its real serializers on synthetic sessions, for both storages, with a
//! complete and an incomplete run each, and copied unchanged into
//! `tests/data/broca-cache-write-split-1113/` (its `SHA256SUMS` hashes to
//! `680c037deaa82635…`).
//!
//! This reader never reads usage. `run.status` is read as raw JSON for its
//! `state` (Stop) and, for a queued send whose turn ended in an error, its
//! `error.provider_code` (`send.rs` `ended_run`); `run.result` is never
//! called; `session.head` carries no split. So the risk is only a decoder
//! that refuses unknown members, and these tests check that the after bodies
//! differ from the before bodies by exactly the two new members.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-cache-write-split-1113")
}

fn body(side: &str, store: &str, run: &str, file: &str) -> Value {
    let path = root().join(side).join(store).join(run).join(file);
    serde_json::from_slice(&std::fs::read(&path).unwrap())
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn the_cache_write_split_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    // 24 bodies and the README.
    assert_eq!(sums.lines().count(), 25);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Takes the split out of a usage object, checking that the two parts sum to
/// the cache-write total, and answers whether it was there.
fn strip_split(usage: &mut Value) -> bool {
    let Some(object) = usage.as_object_mut() else {
        return false;
    };
    let five = object.remove("cache_write_5m_tokens");
    let hour = object.remove("cache_write_1h_tokens");
    match (five, hour) {
        (Some(five), Some(hour)) => {
            let total = object["cache_write_tokens"].as_u64().unwrap();
            assert_eq!(five.as_u64().unwrap() + hour.as_u64().unwrap(), total);
            true
        }
        (None, None) => false,
        pair => panic!("a half split: {pair:?}"),
    }
}

/// On both storages, a complete run's `run.status` and `run.result` gain the
/// split on their usage and on every `by_model` entry; an incomplete run's
/// usage and every `session.head`'s usage stay as they were. `state` and
/// `error`, the members this reader reads, never move. Only `usage`, `state`
/// and `error` are compared: the two captures were separate sessions, so
/// ids, log identities and times differ, as the samples' README says.
#[test]
fn the_split_changes_nothing_this_reader_reads() {
    for store in ["legacy", "store"] {
        for run in ["complete", "incomplete"] {
            for file in ["run.status.json", "run.result.json", "session.head.json"] {
                let before = body("before", store, run, file);
                let mut after = body("after", store, run, file);
                let label = format!("{store}/{run}/{file}");
                assert_eq!(after["state"], before["state"], "{label}");
                assert_eq!(after["error"], before["error"], "{label}");
                let mut stripped = false;
                if let Some(usage) = after.get_mut("usage") {
                    stripped |= strip_split(usage);
                    for entry in usage["by_model"].as_array_mut().into_iter().flatten() {
                        stripped |= strip_split(entry);
                    }
                }
                let expected = run == "complete" && file != "session.head.json";
                assert_eq!(stripped, expected, "{label}");
                assert_eq!(after["usage"], before["usage"], "{label}");
            }
        }
    }
}
