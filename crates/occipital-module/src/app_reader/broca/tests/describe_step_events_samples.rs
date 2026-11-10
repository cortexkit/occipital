//! Broca's `role.describe` from its real describe builder, before and after it
//! always publishes step events: the after side adds the `step_events` group and
//! the contract's `step_events` member. This reader uses neither, so both must
//! decode on both its paths and describe the runner the same way to it: the same
//! groups otherwise, the same stored-media mode and the same send grant.

use super::*;
use std::path::{Path, PathBuf};

const SAMPLES: &str = "tests/data/broca-describe-step-events-1110";
const FILES: &[&str] = &["after-role-describe.json", "before-role-describe.json"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(SAMPLES)
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

#[test]
fn step_events_describe_samples_keep_their_checksums() {
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

#[test]
fn a_describe_with_step_events_reads_as_the_same_runner() {
    let head: cortexkit_role_llm_runner::read::HeadMeta = serde_json::from_value(json!({
        "incarnation": "inc-a", "first_ordinal": 1, "history_version": 1, "updated_at": 1,
        "last_run_state": {"run_id": "r1", "state": "active"},
    }))
    .unwrap();
    let read = |name: &str| {
        let value = sample(name);
        let checked = check_describe(&value)
            .unwrap_or_else(|error| panic!("{name}: check_describe refused: {error:?}"));
        let decoded: RoleDescribe = serde_json::from_value(value.clone())
            .unwrap_or_else(|error| panic!("{name}: plain decode refused: {error}"));
        (value, checked, decoded)
    };
    let (before_value, before, before_plain) = read("before-role-describe.json");
    let (after_value, after, after_plain) = read("after-role-describe.json");
    assert!(
        after_value["step_events"].is_object(),
        "the after side carries the member"
    );
    assert!(before_value.get("step_events").is_none());

    let groups = |value: &Value| -> Vec<String> {
        value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|group| group.as_str().map(str::to_owned))
            .filter(|group| group != "step_events")
            .collect()
    };
    assert_eq!(groups(&before_value), groups(&after_value));
    for group in groups(&before_value) {
        assert!(after.declares(&group), "{group}");
        assert!(after_plain.declares(&group), "{group}");
    }

    assert_eq!(stored_media(Some(&before)), stored_media(Some(&after)));
    assert!(stored_media(Some(&after)).is_some());
    for (b, a) in [(&before, &after), (&before_plain, &after_plain)] {
        assert_eq!(
            send_grant(&head, Some(b), false),
            send_grant(&head, Some(a), false)
        );
    }
}
