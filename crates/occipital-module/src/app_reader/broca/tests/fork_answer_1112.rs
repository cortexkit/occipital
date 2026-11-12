//! Broca's descriptions once it serves forked answers (runner contract 0.8.43,
//! the `fork_answer` group: `session.fork_prepare`, `session.fork_ask` and
//! `session.fork_end`, with `fork_idle_s`), recorded from its real serializers
//! and copied unchanged into `tests/data/broca-fork-answer-1112/`. This reader
//! never forks, so only the descriptions concern it: each must still pass its
//! check and read as the same runner. The fork replies in the set are kept for
//! the checksums only.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-fork-answer-1112")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

#[test]
fn the_fork_answer_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    // 21 bodies and the README.
    assert_eq!(sums.lines().count(), 22);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Both before-and-after pairs (with and without service tiers) pass this
/// reader's check, only the after description declares `fork_answer`, and
/// everything this reader takes from a description (the other groups, the
/// stored-media form, the send grant) is the same in both.
#[test]
fn the_fork_answer_describes_read_as_the_same_runner() {
    let head: cortexkit_role_llm_runner::read::HeadMeta = serde_json::from_value(json!({
        "incarnation": "inc-a", "first_ordinal": 1, "history_version": 1, "updated_at": 1,
        "last_run_state": {"run_id": "r1", "state": "completed"},
    }))
    .unwrap();
    let read = |name: &str| {
        let value = sample(&format!("role-describe/{name}"));
        let checked = check_describe(&value)
            .unwrap_or_else(|error| panic!("{name}: check_describe refused: {error:?}"));
        (value, checked)
    };
    let groups = |value: &Value| -> Vec<String> {
        value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|group| group.as_str().map(str::to_owned))
            .filter(|group| group != "fork_answer")
            .collect()
    };
    for pair in ["", "service-tier-"] {
        let (before_value, before) = read(&format!("role-describe-{pair}before.json"));
        let (after_value, after) = read(&format!("role-describe-{pair}after.json"));
        assert!(!before.declares("fork_answer"), "{pair}");
        assert!(after.declares("fork_answer"), "{pair}");
        assert_eq!(groups(&before_value), groups(&after_value), "{pair}");
        assert_eq!(stored_media(Some(&before)), stored_media(Some(&after)), "{pair}");
        assert_eq!(
            send_grant(&head, Some(&before), false),
            send_grant(&head, Some(&after), false),
            "{pair}"
        );
    }
}
