//! Runner heads under the runner contract: a head numbers entries from 1 and
//! always carries `history_version` and `incarnation`, and the reader holds it
//! to the contract's own consistency rule (`head_meta`).
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

#[test]
fn the_contract_head_samples_keep_their_checksums() {
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
/// refused. So is a head without the log identity every head must carry.
#[test]
fn heads_are_held_to_the_contract_rule() {
    let heads = sample("head.json");
    let answers = heads["answers"].as_array().unwrap();
    assert_eq!(answers.len(), 6);
    for case in answers {
        assert!(head_meta(&case["answer"]).is_ok(), "{}", case["name"]);
    }
    assert!(head_meta(&sample("head-incarnation.json")).is_ok());
    let inconsistent = heads["inconsistent"].as_array().unwrap();
    assert_eq!(inconsistent.len(), 1);
    for case in inconsistent {
        let error = head_meta(&case["answer"]).unwrap_err();
        assert_eq!(error.code, "invalid_provider_reply", "{}", case["name"]);
    }
    let without_log = json!({"first_ordinal":1, "history_version":1, "updated_at":1});
    assert_eq!(
        head_meta(&without_log).unwrap_err().code,
        "invalid_provider_reply"
    );
}

/// A session whose newest compaction marker keeps only entries 3 and up for the
/// model.
fn cut_route() -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["history_display"]));
    *route.entries.lock().unwrap() = vec![
        entry("m1", Some(1), 705),
        entry("m2", Some(2), 710),
        entry("m3", Some(3), 720),
    ];
    route
        .head_extra
        .lock()
        .unwrap()
        .insert("first_ordinal".into(), json!(3));
    route
}

/// The head's `first_ordinal` is where the newest compaction marker starts what
/// the model sees, not where reads start: every entry stays readable from 1, so
/// pages, page-backs and a page around a message all reach entries before it.
#[tokio::test]
async fn entries_before_the_models_first_ordinal_stay_readable() {
    let source = BrocaSource::new(resolved(), cut_route());
    let mids = |page: &HistoryPage| {
        page.rows
            .iter()
            .map(|row| row.mid.clone())
            .collect::<Vec<_>>()
    };
    let newest = source.newest(3).await.unwrap();
    assert_eq!(mids(&newest), ["m1", "m2", "m3"]);
    assert!(newest.older.is_none(), "m1 is the first entry");
    let page_back = source.older(&Anchor::Ordinal(3), 5).await.unwrap();
    assert_eq!(mids(&page_back), ["m1", "m2"]);
    let around = source.around("m1", 0).await.unwrap().unwrap();
    assert_eq!(around.anchor, Some(1));
    assert_eq!(mids(&around.page), ["m1"]);
    let exact = source.exact(&Anchor::Ordinal(1)).await.unwrap().unwrap();
    assert_eq!(exact.mid, "m1");
}

/// A head is still checked: one whose last ordinal is below its first is
/// refused, as the contract's rule says.
#[tokio::test]
async fn an_inconsistent_head_is_refused() {
    let route = cut_route();
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
