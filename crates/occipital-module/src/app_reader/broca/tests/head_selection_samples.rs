//! Broca's replies before and after it adopts runner contract 0.8.6's head
//! selection, recorded from its serve path, through every reader path that
//! decodes them. Copied unchanged into `tests/data/broca-head-selection-0.8.6/`
//! from the set Broca published for its readers (its `SHA256SUMS` hashes to
//! `9002b907…db39c76`):
//! - `role.describe` adds the group `head_selection`, and nothing else changes.
//! - `session.head`'s `model` is the plan's selection as soon as a refresh is
//!   accepted, and it gains `service_tier` when the plan names one. A missing
//!   tier on a runner declaring `head_selection` means standard. `usage` still
//!   reports the model each run used, so the two can differ.
//!
//! `after/session_refresh_accepted.json` answers a plan refresh, which only
//! core sends, so it is only checksummed here.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-head-selection-0.8.6")
}

/// A recorded reply's `result`.
fn result(side: &str, name: &str) -> Value {
    let body: Value =
        serde_json::from_slice(&std::fs::read(root().join(side).join(name)).unwrap()).unwrap();
    runner_result(body).unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"))
}

#[test]
fn the_head_selection_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 8);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Both describes pass the contract's check the reader requires and the plain
/// decode the send path uses. Only the later one declares head selection, and
/// both still declare what sending and display facts depend on.
#[test]
fn broca_0086_describes_decode_on_both_paths() {
    for (side, selects) in [("before", false), ("after", true)] {
        let value = result(side, "role.describe.json");
        let checked = check_describe(&value).unwrap_or_else(|error| panic!("{side}: {error:?}"));
        let plain: RoleDescribe = serde_json::from_value(value).unwrap();
        for describe in [&checked, &plain] {
            for group in [
                "attachments",
                "read_display_facts",
                "head_sender",
                "send_status",
            ] {
                assert!(describe.declares(group), "{side}: {group}");
            }
            assert_eq!(
                describe.declares(cortexkit_role_llm_runner::capabilities::HEAD_SELECTION),
                selects,
                "{side}"
            );
        }
    }
}

/// Each head passes the contract's rule, and the app's details carry the head's
/// model and tier exactly as Broca states them: the refreshed plan's model with
/// `serviceTier` `fast`, and no tier where the head names none. The spend still
/// names the model the run used, which differs from the refreshed selection.
#[tokio::test]
async fn broca_0086_heads_reach_the_apps_details() {
    for (side, name, tier) in [
        ("before", "session.head.json", None),
        ("before", "session.head-unwritten.json", None),
        ("after", "session.head.json", None),
        ("after", "session.head-unwritten.json", None),
        ("after", "session.head-refreshed.json", Some("fast")),
    ] {
        let head = result(side, name);
        head_meta(&head).unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"));
        let route = FakeRoute::new();
        for (key, value) in head.as_object().unwrap() {
            route
                .head_extra
                .lock()
                .unwrap()
                .insert(key.clone(), value.clone());
        }
        let (_, app) = serving(route);
        let details = app
            .read(
                crate::app_reader::DETAILS,
                json!({"session":"alfonso:s1", "harness":"broca"}),
            )
            .await
            .unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"));
        let context = format!("{side}/{name}: {details}");
        assert_eq!(details.get("model"), head.get("model"), "{context}");
        assert_eq!(
            details.get("serviceTier").and_then(Value::as_str),
            tier,
            "{context}"
        );
        if name == "session.head-refreshed.json" {
            assert_eq!(details["model"]["model"], "test-ultrafast", "{context}");
            assert_eq!(
                details["usage"]["by_model"][0]["model"], "test-standard-only",
                "{context}"
            );
        }
    }
}
