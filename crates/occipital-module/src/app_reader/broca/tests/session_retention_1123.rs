//! Broca's samples for per-session storage policies and release (runner
//! contract `session_retention`), recorded from its real serve loop and head
//! serializer and copied unchanged into `tests/data/broca-session-retention-1123/`
//! with its README.
//!
//! A written head now carries `retention_policy` and `retention_state`, and a
//! released one also `released`. None of them changes what the apps get: a
//! chat reads and its details are as before. A deleted session's head refuses
//! `unknown_session`, which reaches the apps as `session_unknown`. A send
//! refused `retention_conflict` or `session_released` reaches them as
//! `runner_refused` with the runner's code, final for the first and transient
//! for the second, as the contract classes them.

use super::session_facts::{details, page, read_only};
use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-session-retention-1123")
}

fn body(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

/// A head sample's own head, from inside the reply's `result`.
fn head(name: &str) -> Value {
    body(name)["result"].clone()
}

const HEADS: [&str; 3] = [
    "retention.assigned.head.json",
    "retention.starter.head.json",
    "retention.released.head.json",
];

const MEMBERS: [&str; 3] = ["retention_policy", "retention_state", "released"];

#[test]
fn the_session_retention_samples_keep_their_checksums() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("a6a1bb32"));
    let sums = String::from_utf8(sums).unwrap();
    // Nine bodies and the README.
    assert_eq!(sums.lines().count(), 10);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Every head decodes as the contract's closed head type, which every read of
/// `session.head` goes through.
#[test]
fn every_head_decodes() {
    for name in HEADS {
        let head = head(name);
        assert!(head.get("retention_policy").is_some(), "{name}");
        super::super::head_meta(&head).unwrap_or_else(|error| panic!("{name}: {error:?}"));
    }
    assert!(head("retention.released.head.json")
        .get("released")
        .is_some());
}

/// `reply` without its `activityToken`, and the token (`null` in details,
/// which carry none).
fn split_token(mut reply: Value) -> (Value, Value) {
    let token = reply
        .as_object_mut()
        .unwrap()
        .remove("activityToken")
        .unwrap_or(Value::Null);
    (reply, token)
}

/// A chat whose head carries a sample's storage-policy members reads the same
/// newest page and details as the same chat without them. Only the activity
/// token differs: it digests the whole head, so when the members first appear
/// or change, an app holding the chat reads its newest page once more.
#[tokio::test]
async fn storage_policy_members_change_no_read() {
    let plain = FakeRoute::new();
    let plain_app = read_only(&plain);
    let newest = json!({"session": "alfonso:s1", "harness": "broca"});
    let (page_before, token_before) = split_token(page(&plain_app, newest.clone()).await);
    let (details_before, _) = split_token(details(&plain_app).await);
    for name in HEADS {
        let head = head(name);
        let route = FakeRoute::new();
        for member in MEMBERS {
            if let Some(value) = head.get(member) {
                route
                    .head_extra
                    .lock()
                    .unwrap()
                    .insert(member.into(), value.clone());
            }
        }
        let app = read_only(&route);
        let (page_after, token_after) = split_token(page(&app, newest.clone()).await);
        assert_eq!(page_after, page_before, "{name}");
        assert_ne!(token_after, token_before, "{name}");
        assert_eq!(split_token(details(&app).await).0, details_before, "{name}");
    }
}

/// A deleted session's head refusal reaches the apps as an unknown session.
#[tokio::test]
async fn a_deleted_session_reads_as_unknown() {
    let refused = body("retention.deleted.head.json");
    let route = FakeRoute::new();
    *route.refusal.lock().unwrap() = Some(subc_protocol::ErrorBody {
        code: refused["code"].as_str().unwrap().into(),
        message: refused["message"].as_str().unwrap().into(),
        detail: refused.get("detail").cloned(),
    });
    let error = read_only(&route)
        .read(
            super::super::super::DETAILS,
            json!({"session": "alfonso:s1", "harness": "broca"}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "session_unknown");
}

/// A send naming a storage policy other than the session's frozen one
/// (`retention_conflict`) passes on as a final `runner_refused` with the
/// runner's code. A
/// release reaches the apps as its own final `session_released`, with the
/// deletion time as `deleteAfterMs`: the app keeps the words as a draft and
/// never resends, since a send after the deletion would start an empty chat.
#[test]
fn the_new_send_refusals_reach_the_apps_with_their_class() {
    let conflict = body("retention.conflict.send.json");
    let refused = crate::app_reader::send::refused(
        conflict["code"].as_str().unwrap(),
        conflict["message"].as_str().unwrap(),
        conflict.get("detail"),
    );
    assert_eq!(refused.code, "runner_refused");
    assert_eq!(
        refused.detail,
        Some(json!({"class": "final", "runnerCode": "retention_conflict"}))
    );
    let released = body("retention.released.send.json");
    assert!(released["detail"]["delete_after_ms"].is_u64());
    let released = crate::app_reader::send::refused(
        released["code"].as_str().unwrap(),
        released["message"].as_str().unwrap(),
        released.get("detail"),
    );
    assert_eq!(released.code, "session_released");
    assert_eq!(
        released.detail,
        Some(json!({"deleteAfterMs": 1_795_394_242_735_u64}))
    );
}
