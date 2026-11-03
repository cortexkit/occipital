//! Runners on runner contract 0.8, read by a module that still reads runners on
//! 0.7. A 0.8 head has no `lineage_id`, numbers entries from 1 and always
//! carries `history_version` and `incarnation`; 0.7's head rule refuses such a
//! head as a partial snapshot, so the reader holds each head to the rule of the
//! contract it was written for (`head_consistent`).
//!
//! The head samples are the contract's own: `head.json` and
//! `head-incarnation.json` from cortexkit-role-llm-runner 0.8.0's
//! `test-vectors/llm-runner-v1`, with the checksums it publishes for them.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/llm-runner-0.8.0")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

fn consistent(head: &Value) -> bool {
    let meta = serde_json::from_value(head.clone()).unwrap();
    head_consistent(head, &meta)
}

#[test]
fn the_contract_0_8_head_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 2);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Every head the contract calls consistent is read, including a never-written
/// session and cuts that keep nothing, and the one it calls inconsistent is
/// refused.
#[test]
fn contract_0_8_heads_are_held_to_its_own_rule() {
    let heads = sample("head.json");
    let answers = heads["answers"].as_array().unwrap();
    assert_eq!(answers.len(), 6);
    for case in answers {
        assert!(consistent(&case["answer"]), "{}", case["name"]);
    }
    assert!(consistent(&sample("head-incarnation.json")));
    let inconsistent = heads["inconsistent"].as_array().unwrap();
    assert_eq!(inconsistent.len(), 1);
    for case in inconsistent {
        assert!(!consistent(&case["answer"]), "{}", case["name"]);
    }
}

/// A runner on 0.7 that keeps a lineage keeps 0.7's rule, and a head without a
/// lineage that isn't a whole 0.8 head is still a partial 0.7 snapshot.
#[test]
fn contract_0_7_heads_keep_their_rule() {
    let lineage =
        json!({"lineage_id":"l1", "first_ordinal":0, "history_version":1, "updated_at":1});
    assert!(consistent(&lineage));
    assert!(!consistent(
        &json!({"lineage_id":"l1", "first_ordinal":1, "history_version":1, "updated_at":1})
    ));
    // A never-written 0.7 session carries nothing, or only its log identity.
    assert!(consistent(&json!({})));
    assert!(consistent(&json!({"incarnation":"log-1"})));
    // Without `incarnation` it isn't an 0.8 head, so its counters are partial.
    assert!(!consistent(
        &json!({"first_ordinal":1, "history_version":1, "updated_at":1})
    ));
}

fn route(contract: &str) -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = vec![
        entry("m1", Some(1), 705),
        entry("m2", Some(2), 710),
        entry("m3", Some(3), 720),
    ];
    if contract == "0.8" {
        route.lineage.lock().unwrap().clear();
        let mut head = route.head_extra.lock().unwrap();
        head.insert("incarnation".into(), json!("log-1"));
        head.insert("last_ordinal".into(), json!(3));
    }
    route
}

/// The same transcript reads the same from a runner on either contract.
#[tokio::test]
async fn a_runner_on_contract_0_8_serves_the_same_rows_as_one_on_0_7() {
    let old = BrocaSource::new(resolved(), route("0.7"));
    let new = BrocaSource::new(resolved(), route("0.8"));
    let (old_page, new_page) = (old.newest(3).await.unwrap(), new.newest(3).await.unwrap());
    assert_eq!(new_page.rows.len(), 3);
    for (old, new) in old_page.rows.iter().zip(&new_page.rows) {
        assert_eq!(
            serde_json::to_vec(&old.display).unwrap(),
            serde_json::to_vec(&new.display).unwrap()
        );
    }
    let exact = new.exact(&Anchor::Ordinal(2)).await.unwrap().unwrap();
    assert_eq!(exact.mid, "m2");
}

/// An 0.8 head is still checked: one whose last ordinal is below its first is
/// refused, as 0.8's rule says.
#[tokio::test]
async fn an_inconsistent_contract_0_8_head_is_refused() {
    let route = route("0.8");
    route
        .head_extra
        .lock()
        .unwrap()
        .insert("first_ordinal".into(), json!(5));
    let error = BrocaSource::new(resolved(), route)
        .newest(3)
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_provider_reply");
    assert_eq!(
        error.message,
        "Broca session.head has an inconsistent snapshot"
    );
}
