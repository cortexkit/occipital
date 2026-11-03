//! Broca's `role.describe`, recorded through its real serve path before and after
//! 0.3.261 adds the `host_history`, `history_display` and `output_refs` groups and
//! their operations. Both ways this reader decodes it must still accept it and still
//! find `head_sender`, because that group is what makes a head without `sender` mean
//! "the app may not send" rather than "unknown". The samples were recorded under
//! runner contract 0.7 and are read as a runner on the current contract serves
//! them (`current_contract`).

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
    current_contract(serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap())
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
        "incarnation": "inc-a", "first_ordinal": 1, "history_version": 1, "updated_at": 1,
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

/// Broca 0.3.299's `role.describe` with its service-tier switch on, captured
/// from its serve path: it adds the `service_tier` group and a `service_tiers`
/// member. Both decoders must still accept it and keep the groups the reader
/// relies on, or the reader would stop caching it and treat sends as unknown.
#[test]
fn the_service_tier_describe_decodes_on_both_paths() {
    const TIER: &str = "tests/data/broca-describe-0.3.299";
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(TIER);
    let bytes = std::fs::read(root.join("role.describe.json")).unwrap();
    let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
    assert_eq!(
        sums,
        format!("{:x}  role.describe.json\n", Sha256::digest(&bytes))
    );
    let value =
        current_contract(serde_json::from_slice::<Value>(&bytes).unwrap()["result"].clone());
    let checked = check_describe(&value).unwrap_or_else(|error| panic!("{error:?}"));
    let plain: RoleDescribe = serde_json::from_value(value).unwrap();
    let head: cortexkit_role_llm_runner::read::HeadMeta = serde_json::from_value(json!({
        "incarnation": "inc-a", "first_ordinal": 1, "history_version": 1, "updated_at": 1,
        "last_run_state": {"run_id": "r1", "state": "active"},
    }))
    .unwrap();
    for describe in [&checked, &plain] {
        for group in ["service_tier", "head_sender", "history_display", "retract"] {
            assert!(describe.declares(group), "{group}");
        }
        assert_eq!(
            send_grant(&head, Some(describe), false),
            SendGrant::MayNotSend
        );
    }
}

/// Broca's turn-scoped system messages (0.3.300) add `turn_system` to
/// `role.describe`'s `delivery_order_modes` and to a head's `sender.modes`.
/// These are Broca's serve-path replies before and after, for one describe and
/// seven heads that differ in who asks and what the plan grants. Every decoder
/// here accepts them: the reader's and the send path's head decode, and both
/// describe decoders. A head's grant, which the apps receive as is, comes out
/// as before with only the new mode added.
#[test]
fn turn_system_describe_and_heads_decode_as_before() {
    use cortexkit_role_llm_runner::read::HeadMeta;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-turn-system-0.3.300");
    let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 16);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
    let load = |side: &str, name: &str| -> Value {
        let bytes = std::fs::read(root.join(side).join(name)).unwrap();
        current_contract(serde_json::from_slice::<Value>(&bytes).unwrap()["result"].clone())
    };
    let describe = |side: &str| {
        let value = load(side, "role-describe.response.json");
        let checked = check_describe(&value).unwrap_or_else(|error| panic!("{side}: {error:?}"));
        let plain: RoleDescribe = serde_json::from_value(value).unwrap();
        assert!(checked.declares("head_sender") && plain.declares("head_sender"));
        (checked, plain)
    };
    let (before_describe, _) = describe("before");
    let (checked, plain) = describe("after");

    let mut heads: Vec<String> = std::fs::read_dir(root.join("before"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("session-head-"))
        .collect();
    heads.sort();
    assert_eq!(heads.len(), 7);
    for name in &heads {
        let before: HeadMeta = serde_json::from_value(load("before", name)).unwrap();
        let after: HeadMeta = serde_json::from_value(load("after", name))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let was = send_grant(&before, Some(&before_describe), false);
        for describe in [&checked, &plain] {
            match (&was, send_grant(&after, Some(describe), false)) {
                (SendGrant::Granted(was), SendGrant::Granted(now)) => {
                    let mut expected = was.clone();
                    let modes = expected
                        .get_mut("modes")
                        .and_then(Value::as_array_mut)
                        .unwrap();
                    let at = modes.iter().position(|mode| mode == "tool_result").unwrap() + 1;
                    modes.insert(at, json!("turn_system"));
                    assert_eq!(now, expected, "{name}");
                }
                (was, now) => assert_eq!(&now, was, "{name}"),
            }
        }
    }
}

#[test]
fn broca_0_7_describes_preserve_sender_grants_on_both_decoders() {
    for file in ["role.describe.before.json", "role.describe.after.json"] {
        for (name, reply) in samples_0_7::samples(file) {
            let reply = current_contract(reply);
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
                        let reply = current_contract(reply);
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
