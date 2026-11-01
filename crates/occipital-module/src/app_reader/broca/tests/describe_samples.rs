//! Broca's `role.describe`, recorded through its real serve path before and after
//! 0.3.261 adds the `host_history`, `history_display` and `output_refs` groups and
//! their operations. Both ways this reader decodes it must still accept it and still
//! find `head_sender`, because that group is what makes a head without `sender` mean
//! "the app may not send" rather than "unknown".

use super::*;
use std::path::{Path, PathBuf};

const SAMPLES: &str = "tests/data/broca-describe-0.3.261";
const FILES: &[&str] = &[
    "role_describe.after.json",
    "role_describe.before.json",
    "role_describe_reader.after.json",
    "role_describe_reader.before.json",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(SAMPLES)
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

#[test]
fn broca_describe_samples_keep_their_checksums() {
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
fn every_broca_describe_decodes_on_both_paths_and_still_declares_head_sender() {
    // A head with no `sender`, as the send path reads it.
    let head: cortexkit_role_llm_runner::read::HeadMeta = serde_json::from_value(json!({
        "lineage_id": "l1", "first_ordinal": 0, "history_version": 1, "updated_at": 1,
        "last_run_state": {"run_id": "r1", "state": "active"},
    }))
    .unwrap();
    for name in FILES {
        let value = sample(name);
        // The reader caches only an answer that passes the contract's own check.
        let checked = check_describe(&value)
            .unwrap_or_else(|error| panic!("{name}: check_describe refused: {error:?}"));
        // The app send decodes it without the check.
        let decoded: RoleDescribe = serde_json::from_value(value)
            .unwrap_or_else(|error| panic!("{name}: plain decode refused: {error}"));
        for describe in [&checked, &decoded] {
            assert_eq!(
                send_grant(&head, Some(describe), false),
                SendGrant::MayNotSend,
                "{name}: a head without sender must read as no grant, not unknown"
            );
        }
    }
}

#[test]
fn broca_0_7_describes_preserve_sender_grants_on_both_decoders() {
    for file in ["role.describe.before.json", "role.describe.after.json"] {
        for (name, reply) in samples_0_7::samples(file) {
            let value = result(reply).unwrap();
            let checked = check_describe(&value).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let plain: RoleDescribe = serde_json::from_value(value).unwrap();
            for describe in [&checked, &plain] {
                assert!(describe.declares("head_sender"), "{name}");
                assert_eq!(describe.declares("call_identity"), file.contains("after"));
                assert_eq!(
                    describe.declares("system_text_default"),
                    file.contains("after")
                );
                for head_file in [
                    "session.head.completed.json",
                    "session.head.error.fractional.json",
                    "session.head.error.missing-delay.json",
                ] {
                    for (head_name, reply) in samples_0_7::samples(head_file) {
                        let mut head = result(reply).unwrap();
                        let meta = serde_json::from_value(head.clone()).unwrap();
                        assert_eq!(
                            send_grant(&meta, Some(describe), false),
                            SendGrant::Granted(head["sender"].as_object().unwrap().clone()),
                            "{name}, {head_name}"
                        );
                        // A declaration makes an absent sender a known refusal.
                        head.as_object_mut().unwrap().remove("sender");
                        let meta = serde_json::from_value(head).unwrap();
                        assert_eq!(
                            send_grant(&meta, Some(describe), false),
                            SendGrant::MayNotSend,
                            "{name}, {head_name}"
                        );
                    }
                }
            }
        }
    }
}
