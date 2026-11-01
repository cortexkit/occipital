//! Broca declaring `session_change` (llm-runner) read through every reader path that
//! decodes the surfaces it changes (`session-change-1030`).
//!
//! The samples under `occipital-app-reader/tests/fixtures/broca/session-change-1030/`
//! are Broca's serve-path replies once it declares the group: `role.describe` with the
//! three new operations, and `session.send` replies whose `baseline` gains `rungs`,
//! before and after. This reader never calls `session.baseline`, `session.refresh`,
//! `session.refresh_policy` or `session.flush_prefix` on Broca, so those files are
//! checked only for their checksums.

use super::*;
use cortexkit_role_llm_runner::send::SendReply;
use sha2::{Digest, Sha256};

const FIXTURES: &str = "../occipital-app-reader/tests/fixtures/broca/session-change-1030";

fn path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURES)
        .join(name)
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(path(name)).unwrap()).unwrap()
}

/// Every file each SHA256SUMS lists is present and verifies.
#[test]
fn the_session_change_samples_are_complete_and_their_checksums_verify() {
    let mut count = 0;
    for dir in ["", "send-reply-goldens/"] {
        let sums = std::fs::read_to_string(path(&format!("{dir}SHA256SUMS"))).unwrap();
        for line in sums.lines() {
            let (digest, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(path(&format!("{dir}{name}"))).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&bytes)),
                digest,
                "{dir}{name}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 16);
}

/// Both describe decoders accept the declaration and see the group.
#[test]
fn both_describe_decoders_accept_session_change() {
    let value = sample("role.describe.session_change.json")["result"].clone();
    let checked = check_describe(&value).unwrap_or_else(|error| panic!("checked: {error:?}"));
    let plain: RoleDescribe =
        serde_json::from_value(value).unwrap_or_else(|error| panic!("plain: {error}"));
    for describe in [&checked, &plain] {
        assert!(describe.capabilities.iter().any(|g| g == "session_change"));
    }
}

/// The app send decodes Broca's send reply the same way before and after `baseline`
/// gains `rungs`: same state and run.
#[test]
fn the_app_send_decodes_send_replies_with_rungs() {
    for case in ["admission", "session_class"] {
        let decode = |side: &str| -> SendReply {
            let name = format!("send-reply-goldens/send_reply.{case}.json.{side}");
            serde_json::from_value(sample(&name)).unwrap_or_else(|error| panic!("{name}: {error}"))
        };
        let (before, after) = (decode("before"), decode("after"));
        assert_eq!(after.state, before.state, "{case}");
        assert_eq!(after.run_id, before.run_id, "{case}");
    }
}
