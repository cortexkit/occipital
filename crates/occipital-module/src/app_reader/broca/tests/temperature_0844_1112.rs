//! Broca's description before and after it declares `temperature` (runner
//! contract 0.8.44), recorded from its real serializer and copied unchanged
//! into `tests/data/broca-temperature-0844-1112/`. This reader never sends a
//! temperature, so the description must only still pass its check and read
//! as the same runner.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-temperature-0844-1112")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

#[test]
fn the_temperature_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 2);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Both descriptions pass this reader's check, only the after one declares
/// `temperature`, and everything this reader takes from a description (the
/// other groups, the stored-media form, the send grant) is the same in both.
#[test]
fn the_temperature_describes_read_as_the_same_runner() {
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
    let groups = |value: &Value| -> Vec<String> {
        value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|group| group.as_str().map(str::to_owned))
            .filter(|group| group != "temperature")
            .collect()
    };
    let (before_value, before) = read("role-describe-before.json");
    let (after_value, after) = read("role-describe-after.json");
    assert!(!before.declares("temperature"));
    assert!(after.declares("temperature"));
    assert_eq!(groups(&before_value), groups(&after_value));
    assert_eq!(stored_media(Some(&before)), stored_media(Some(&after)));
    assert_eq!(
        send_grant(&head, Some(&before), false),
        send_grant(&head, Some(&after), false)
    );
}
