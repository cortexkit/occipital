//! Broca's replies on runner contract 0.8.42, from its real serializers
//! (`tests/data/broca-runner-0842-1112/`, copied unchanged). Two default replies
//! change. `role.describe` adds the `plan_view` group and op, so the context
//! view is asked of Broca heads. `session.head`'s `last_run_state` gains
//! `resume_not_before_ms` while a run is paused for an account limit. Both sides
//! of each must decode on this reader's paths, and only those facts may differ.
//! The samples' README speaks of Broca's two session stores: the "new
//! storage", where every head lives, and "today's storage", the older one.

use super::*;
use std::path::{Path, PathBuf};

const SAMPLES: &str = "tests/data/broca-runner-0842-1112";
const FILES: &[&str] = &[
    "README.md",
    "role-describe-after.json",
    "role-describe-before.json",
    "session-head-account-limit-after.json",
    "session-head-account-limit-before.json",
    "session-head-completed.json",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(SAMPLES)
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

#[test]
fn runner_0842_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    let mut listed = Vec::new();
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
        listed.push(name.to_string());
    }
    listed.sort_unstable();
    assert_eq!(listed, FILES, "every sample has one checksum");
}

/// The after side declares the plan view; everything else this reader takes
/// from a description (the other groups, the stored-media form, the send
/// grant) is the same on both sides.
#[test]
fn the_0842_describe_adds_only_the_plan_view() {
    let head: cortexkit_role_llm_runner::read::HeadMeta =
        serde_json::from_value(sample("session-head-completed.json")).unwrap();
    let read = |name: &str| {
        let value = sample(name);
        let checked = check_describe(&value)
            .unwrap_or_else(|error| panic!("{name}: check_describe refused: {error:?}"));
        (value, checked)
    };
    let (before_value, before) = read("role-describe-before.json");
    let (after_value, after) = read("role-describe-after.json");
    assert!(!before.declares(capabilities::PLAN_VIEW));
    assert!(after.declares(capabilities::PLAN_VIEW));

    let groups = |value: &Value| -> Vec<String> {
        value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|group| group.as_str().map(str::to_owned))
            .filter(|group| group != capabilities::PLAN_VIEW)
            .collect()
    };
    assert_eq!(groups(&before_value), groups(&after_value));
    assert_eq!(stored_media(Some(&before)), stored_media(Some(&after)));
    assert_eq!(
        send_grant(&head, Some(&before), false),
        send_grant(&head, Some(&after), false)
    );
}

/// A head paused for an account limit reaches the apps with the reason, and
/// on the after side with when the runner resumes it; an idle head is
/// unchanged. Served on newest pages, claims, details and watches alike.
#[tokio::test]
async fn the_0842_head_names_when_an_account_paused_run_resumes() {
    let state = |name: &str| sample(name)["last_run_state"].clone();
    super::session_facts::assert_last_run_phone_bytes(
        state("session-head-account-limit-before.json"),
        None,
        r#"{"reason":"account_limit","runId":"run-1","state":"paused"}"#,
    )
    .await;
    super::session_facts::assert_last_run_phone_bytes(
        state("session-head-account-limit-after.json"),
        None,
        r#"{"reason":"account_limit","resumeNotBeforeMs":1790000600000,"runId":"run-1","state":"paused"}"#,
    )
    .await;
    super::session_facts::assert_last_run_phone_bytes(
        state("session-head-completed.json"),
        None,
        r#"{"runId":"run-1","state":"completed"}"#,
    )
    .await;
}
