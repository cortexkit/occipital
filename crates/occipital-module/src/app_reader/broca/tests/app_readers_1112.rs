//! Broca serving reads to the principals a plan lists as `readers` (runner
//! contract 0.8.42, the `app_readers` group), recorded from its real
//! serializers and serve path on synthetic sessions, copied unchanged into
//! `tests/data/broca-app-readers-1112/`. The description adds only the group,
//! a listed reader gets the owner's page byte for byte, and anyone else is
//! refused `scope_owner_mismatch`.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-app-readers-1112")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

#[test]
fn the_app_readers_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    // Six bodies and the README.
    assert_eq!(sums.lines().count(), 7);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Both descriptions pass this reader's check, and everything it takes from
/// one (the other groups, the stored-media form, the send grant) is the same.
#[test]
fn the_app_readers_describe_reads_as_the_same_runner() {
    let head: cortexkit_role_llm_runner::read::HeadMeta = serde_json::from_value(json!({
        "incarnation": "inc-a", "first_ordinal": 1, "history_version": 1, "updated_at": 1,
        "last_run_state": {"run_id": "r1", "state": "completed"},
    }))
    .unwrap();
    let read = |name: &str| {
        let value = sample(name);
        let checked = check_describe(&value)
            .unwrap_or_else(|error| panic!("{name}: check_describe refused: {error:?}"));
        (value, checked)
    };
    let (before_value, before) = read("role-describe-before.json");
    let (after_value, after) = read("role-describe-after.json");
    assert!(!before.declares("app_readers"));
    assert!(after.declares("app_readers"));
    let groups = |value: &Value| -> Vec<String> {
        value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|group| group.as_str().map(str::to_owned))
            .filter(|group| group != "app_readers")
            .collect()
    };
    assert_eq!(groups(&before_value), groups(&after_value));
    assert_eq!(stored_media(Some(&before)), stored_media(Some(&after)));
    assert_eq!(
        send_grant(&head, Some(&before), false),
        send_grant(&head, Some(&after), false)
    );
}

/// A listed reader's page is the owner's, so it builds the same rows. An
/// unlisted caller's refusal reaches the apps as the runner being unavailable,
/// which they retry, with the runner's code kept in the message; a malformed
/// reader list (a plan fault, not the reader's) is the same.
#[test]
fn the_app_readers_pages_and_refusals_read_as_before() {
    assert_eq!(
        std::fs::read(root().join("session-read-listed.json")).unwrap(),
        std::fs::read(root().join("session-read-owner.json")).unwrap()
    );
    let page: RawPage =
        serde_json::from_value(sample("session-read-listed.json")["result"].clone()).unwrap();
    assert_eq!(page.messages.len(), 2);
    for (name, runner_code) in [
        ("session-read-unlisted-refusal.json", "scope_owner_mismatch"),
        ("plan-readers-invalid-refusal.json", "invalid_params"),
    ] {
        let body = sample(name);
        assert_eq!(body["code"], runner_code, "{name}");
        let error = super::super::read_call_error(
            ReadOp::Read,
            BROCA_RUNNER_MODULE_ID,
            CallError::Module(subc_protocol::ErrorBody {
                code: runner_code.into(),
                message: body["message"].as_str().unwrap().into(),
                detail: body.get("detail").cloned(),
            }),
        );
        assert_eq!(error.code, "provider_unavailable", "{name}");
        assert!(
            error.message.contains(runner_code),
            "{name}: {}",
            error.message
        );
    }
}
