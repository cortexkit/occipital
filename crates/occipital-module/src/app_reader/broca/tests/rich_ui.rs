//! The rich UI stamp (llm-runner §9.17) reaches the phone on stamped assistant rows.
//!
//! A runner declaring `rich_ui_stamp` adds a stamped assistant entry's stamp to
//! `session.read` when the read sends `include_rich_ui: true`; any other runner refuses
//! that field. The reader asks only where declared, and serves the stamp as
//! `richUi: {dialect, catalogDigest}` on assistant rows, never on others.
use super::*;
use crate::app_reader::tests::serve;
use crate::app_reader::TRANSCRIPT_PAGE;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn route(groups: &[&str]) -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(groups));
    let stamp = json!({"dialect": "1.0", "catalog_digest": DIGEST});
    *route.entries.lock().unwrap() = vec![
        json!({"mid":"m20", "ordinal":20, "written_at":700, "rich_ui": stamp,
            "message":{"role":"user", "content":[{"type":"text", "text":"show me"}]}}),
        json!({"mid":"m21", "ordinal":21, "written_at":701, "rich_ui": stamp,
            "message":{"role":"assistant", "content":[{"type":"text", "text":"stamped"}]}}),
        json!({"mid":"m22", "ordinal":22, "written_at":702,
            "rich_ui": {"dialect": "has space", "catalog_digest": DIGEST},
            "message":{"role":"assistant", "content":[{"type":"text", "text":"malformed"}]}}),
        json!({"mid":"m23", "ordinal":23, "written_at":703,
            "message":{"role":"assistant", "content":[{"type":"text", "text":"plain"}]}}),
    ];
    route
}

/// The `session.read` requests the reader sent.
fn reads(route: &FakeRoute) -> Vec<Value> {
    route
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == "session.read")
        .map(|call| call["params"].clone())
        .collect()
}

async fn rows(route: Arc<FakeRoute>) -> Vec<Value> {
    let (_, app) = serving(route);
    let bytes = serve(
        &app,
        TRANSCRIPT_PAGE,
        &json!({"session":"alfonso:s1", "harness":"broca", "limit":4}),
    )
    .await;
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    let body = reply.get("result").unwrap_or(&reply);
    body["messages"].as_array().unwrap().clone()
}

#[tokio::test]
async fn a_declaring_runner_is_asked_for_stamps_and_stamped_assistant_rows_carry_them() {
    let route = route(&["rich_ui_stamp"]);
    let rows = rows(route.clone()).await;
    let ranged: Vec<Value> = reads(&route)
        .into_iter()
        .filter(|params| params.get("from_ordinal").is_some())
        .collect();
    assert!(!ranged.is_empty(), "no ranged read was sent");
    for params in &ranged {
        assert_eq!(params["include_rich_ui"], true, "{params}");
    }
    let by_mid = |mid: &str| {
        rows.iter()
            .find(|row| row["messageId"] == mid)
            .unwrap_or_else(|| panic!("no row {mid}: {rows:?}"))
    };
    assert_eq!(
        by_mid("m21")["richUi"],
        json!({"dialect": "1.0", "catalogDigest": DIGEST})
    );
    // Never on a user row, never from a stamp the contract's validation refuses, and
    // never invented for an unstamped one.
    for mid in ["m20", "m22", "m23"] {
        assert!(
            by_mid(mid).get("richUi").is_none(),
            "{mid}: {}",
            by_mid(mid)
        );
    }
}

#[tokio::test]
async fn a_runner_without_the_group_is_never_asked_for_stamps() {
    let route = route(&[]);
    rows(route.clone()).await;
    let sent = reads(&route);
    assert!(!sent.is_empty());
    for params in sent {
        assert!(params.get("include_rich_ui").is_none(), "{params}");
    }
}
