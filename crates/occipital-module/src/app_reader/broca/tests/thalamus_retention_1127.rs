//! Thalamus's samples for per-session storage policies and release on its
//! OpenCode 2 and Claude Code hosts (runner contract `session_retention`),
//! recorded from its real dispatch and serializers and copied unchanged into
//! `tests/data/thalamus-session-retention-v1/`, with each host's
//! `role.describe` in `tests/data/thalamus-describe-530d6c9f9/`.
//!
//! As for Broca's (`session_retention_1123`), the new head members change
//! nothing the apps read, and a send to a released session reaches the apps
//! as a final `session_released` with its deletion time. Occipital never
//! sends a storage policy, nor releases a session, so the first-send and
//! release replies are kept only so the checksums verify whole.

use super::session_facts::{details, page, read_only};
use super::*;
use std::path::{Path, PathBuf};

fn data(set: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(set)
}

fn body(set: &str, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(data(set).join(name)).unwrap()).unwrap()
}

const RETENTION: &str = "thalamus-session-retention-v1";
const DESCRIBE: &str = "thalamus-describe-530d6c9f9";

#[test]
fn the_samples_keep_their_checksums() {
    for (set, sums_digest) in [(RETENTION, "3717ab99"), (DESCRIBE, "80c0a899")] {
        let sums = std::fs::read(data(set).join("SHA256SUMS")).unwrap();
        assert!(
            format!("{:x}", Sha256::digest(&sums)).starts_with(sums_digest),
            "{set}"
        );
        for line in String::from_utf8(sums).unwrap().lines() {
            let (expected, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(data(set).join(name)).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                expected,
                "{set} {name}"
            );
        }
    }
}

/// Both heads decode as the contract's closed head type, and a chat whose
/// head carries their storage-policy members reads the same newest page and
/// details as the same chat without them, apart from the activity token,
/// which digests the whole head.
#[tokio::test]
async fn storage_policy_members_change_no_read() {
    let plain = read_only(&FakeRoute::new());
    let newest = json!({"session": "alfonso:s1", "harness": "broca"});
    let without_token = |mut reply: Value| {
        reply.as_object_mut().unwrap().remove("activityToken");
        reply
    };
    let page_before = without_token(page(&plain, newest.clone()).await);
    let details_before = without_token(details(&plain).await);
    for name in ["head.json", "head-released.json"] {
        let head = body(RETENTION, name);
        super::super::head_meta(&head).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let route = FakeRoute::new();
        for member in ["retention_policy", "retention_state", "released"] {
            if let Some(value) = head.get(member) {
                route
                    .head_extra
                    .lock()
                    .unwrap()
                    .insert(member.into(), value.clone());
            }
        }
        let app = read_only(&route);
        assert_eq!(
            without_token(page(&app, newest.clone()).await),
            page_before,
            "{name}"
        );
        assert_eq!(without_token(details(&app).await), details_before, "{name}");
    }
}

/// A send to a released session reaches the apps as a final
/// `session_released` with the deletion time as `deleteAfterMs`, and one
/// naming another storage policy as a final `runner_refused` with the
/// runner's code.
#[test]
fn the_send_refusals_reach_the_apps_as_for_broca() {
    let refused = |name: &str| {
        let body = body(RETENTION, name);
        crate::app_reader::send::refused(
            body["code"].as_str().unwrap(),
            body["message"].as_str().unwrap(),
            body.get("detail"),
        )
    };
    let released = refused("session-released.json");
    assert_eq!(released.code, "session_released");
    assert_eq!(
        released.detail,
        Some(json!({"deleteAfterMs": 1_000_000_u64}))
    );
    let conflict = refused("retention-conflict.json");
    assert_eq!(conflict.code, "runner_refused");
    assert_eq!(
        conflict.detail,
        Some(json!({"class": "final", "runnerCode": "retention_conflict"}))
    );
}

/// Each host's description passes this reader's check. OpenCode 2 and
/// Claude Code declare `session_retention`; Pi doesn't.
#[test]
fn each_hosts_description_passes() {
    for (name, retention) in [
        ("role-describe-opencode2.json", true),
        ("role-describe-claude-code.json", true),
        ("role-describe-pi.json", false),
    ] {
        let describe = check_describe(&body(DESCRIBE, name))
            .unwrap_or_else(|error| panic!("{name}: check_describe refused: {error:?}"));
        assert_eq!(describe.declares("session_retention"), retention, "{name}");
        assert!(describe.declares("send_promote"), "{name}");
    }
}
