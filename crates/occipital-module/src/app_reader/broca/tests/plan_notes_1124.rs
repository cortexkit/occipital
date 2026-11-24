//! Broca declaring `plan_notes` (llm-runner §10.2), from its serializers, copied
//! unchanged into `tests/data/broca-plan-notes-1124/`. The only reply this
//! reader reads is `role.describe`, which gains the group. Plan notes are
//! runner records that never reach a transcript read, and this reader never
//! calls `session.baseline`, so the baseline files are checked only for their
//! checksums.

use super::*;
use std::path::{Path, PathBuf};

fn path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/broca-plan-notes-1124")
        .join(name)
}

#[test]
fn the_plan_notes_samples_keep_their_checksums() {
    let sums = std::fs::read(path("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("926af963"));
    let sums = String::from_utf8(sums).unwrap();
    assert_eq!(sums.lines().count(), 4);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(path(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Broca's `role.describe` with `plan_notes` passes llm-runner's
/// `check_describe`, and still declares every capability group Occipital
/// acts on.
#[test]
fn the_description_with_plan_notes_passes() {
    let raw: Value =
        serde_json::from_slice(&std::fs::read(path("role-describe.json")).unwrap()).unwrap();
    let describe = check_describe(&raw).unwrap_or_else(|problems| panic!("{problems:?}"));
    assert!(describe.declares("plan_notes"));
    for group in [
        "read_display_facts",
        "media_refs",
        "tool_media_refs",
        "plan_view",
    ] {
        assert!(describe.declares(group), "{group}");
    }
}
