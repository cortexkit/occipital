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

/// The sample set `phone-reader-head-rich-v3`: a reply stamped with the reply
/// catalog's current digest that holds a `Badge`, a `Styled`, a `Callout` and a
/// `LinkCard`, the tags a reply may still hold without references or handles.
/// The chat is invented, in the shape Broca serves it, and is served
/// through Occipital's own handlers as a phone's route reaches them. The digest
/// is the library's own, so a catalog change that moves it fails this test and
/// needs the next set.
#[tokio::test]
async fn the_head_rich_v3_samples_are_what_the_reader_serves() {
    let digest = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../occipital-lang/catalog/catalog-v1.digest"
    ))
    .unwrap();
    let digest = digest.trim();
    assert!(digest.starts_with("a6ea6f5d"), "{digest}");
    let reply = "The nightly backup finished. <Badge tone=\"success\">3 of 3 copied</Badge>\n\n\
        <Styled>Sizes come from the copy run two minutes ago.</Styled>\n\n\
        <Callout title=\"One folder was slow\" tone=\"warning\">\n\
        The photos folder took 14 minutes, twice as long as last week.\n\
        </Callout>\n\n\
        <LinkCard href=\"https://example.com/backups/nightly\" title=\"Nightly backup report\" />";
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["rich_ui_stamp"]));
    *route.run_state.lock().unwrap() = Value::Null;
    *route.entries.lock().unwrap() = vec![
        json!({"mid": "m1", "ordinal": 1, "written_at": 1_795_600_000_000_u64,
            "message": {"role": "user",
                "content": [{"type": "text", "text": "How did last night's backup go?"}]}}),
        json!({"mid": "m2", "ordinal": 2, "written_at": 1_795_600_004_000_u64,
            "rich_ui": {"dialect": "1.0", "catalog_digest": digest},
            "message": {"role": "assistant", "content": [{"type": "text", "text": reply}]}}),
    ];
    let (_, app) = serving(route);
    let mut served = super::served_samples::Served::new("phone-reader-head-rich-v3", app);
    let page = json!({"method": TRANSCRIPT_PAGE,
        "params": {"session": "alfonso:s1", "harness": "broca", "limit": 6}});
    let bytes = served.read(1, &page).await;
    let rows = super::served_samples::decoded(&bytes)["result"]["messages"].clone();
    assert_eq!(rows[0]["richUi"], Value::Null);
    assert_eq!(
        rows[1]["richUi"],
        json!({"dialect": "1.0", "catalogDigest": digest})
    );
    assert_eq!(rows[1]["text"], json!(reply));
    served.keep("01-newest-page", &page, bytes);
    served.check();
}
