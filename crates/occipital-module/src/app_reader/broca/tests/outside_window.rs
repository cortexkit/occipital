use super::*;
use crate::app_reader::TRANSCRIPT_PAGE;

fn request() -> Value {
    json!({"session":"alfonso:s1", "harness":"broca", "limit":50})
}

fn held(page: &Value, since: &str) -> Value {
    let mut request = request();
    request["claim"] = json!({"heldFloorMessageId":"m0", "sinceMessageId":since,
        "heldVersion":page["version"]});
    request["acceptClaims"] = json!(["future_verdict", "outside_window"]);
    request
}

#[tokio::test]
async fn legacy_claim_clients_resync_gaps_without_outside_window() {
    let route = super::bounded_reads::populated(200);
    let (_, app) = serving(route);
    let newest = app.read(TRANSCRIPT_PAGE, request()).await.unwrap();
    let opted_in = app
        .read(TRANSCRIPT_PAGE, held(&newest, "m1"))
        .await
        .unwrap();
    assert_eq!(opted_in["claim"], "outside_window");
    for accepted in [
        None,
        Some(json!([])),
        Some(json!(["future_verdict"])),
        Some(json!(["matched", "diverged", "cursor_expired"])),
    ] {
        let mut gap = held(&newest, "m1");
        let mut in_window = held(&newest, "m198");
        for params in [&mut gap, &mut in_window] {
            if let Some(accepted) = &accepted {
                params["acceptClaims"] = accepted.clone();
            } else {
                params.as_object_mut().unwrap().remove("acceptClaims");
            }
        }
        let reply = app.read(TRANSCRIPT_PAGE, gap).await.unwrap();
        assert_eq!(reply["claim"], "cursor_expired", "{accepted:?}: {reply}");
        assert_eq!(reply["resync"], opted_in["resync"]);
        assert_eq!(reply["resync"]["messages"], newest["messages"]);
        assert_eq!(
            reply["resync"]["nextBeforeOrdinal"],
            newest["nextBeforeOrdinal"]
        );
        let matched = app.read(TRANSCRIPT_PAGE, in_window).await.unwrap();
        assert_eq!(matched["claim"], "matched");
        assert_eq!(matched["newTurns"].as_array().unwrap().len(), 1);
        assert_eq!(matched["newTurns"][0]["messageId"], "m199");
    }
}

#[tokio::test]
async fn malformed_accept_claims_uses_the_invalid_claim_reply() {
    let (_, app) = serving(FakeRoute::new());
    for accepted in [
        Value::Null,
        json!(true),
        json!(3),
        json!("outside_window"),
        json!({}),
        json!(["outside_window", 7]),
        json!([false]),
    ] {
        let mut params = request();
        params["acceptClaims"] = accepted;
        for claim in [
            None,
            Some(
                json!({"heldFloorMessageId":"m20", "sinceMessageId":"m21", "heldVersion":"history:7:lineage:cut-a"}),
            ),
        ] {
            if let Some(claim) = claim {
                params["claim"] = claim;
            }
            let reply = app.read(TRANSCRIPT_PAGE, params.clone()).await.unwrap();
            assert_eq!(
                reply,
                json!({"error":{"code":"invalid_claim",
                "message":"acceptClaims must be an array of strings", "terminal":true}})
            );
        }
    }
}

#[tokio::test]
async fn outside_window_resync_pages_back_to_the_held_anchor() {
    let route = super::bounded_reads::populated(20_000);
    route
        .head_extra
        .lock()
        .unwrap()
        .insert("model".into(), json!({"provider":"p", "model":"m"}));
    let (_, app) = serving(route.clone());
    let newest = app.read(TRANSCRIPT_PAGE, request()).await.unwrap();
    let gap = app
        .read(TRANSCRIPT_PAGE, held(&newest, "m19889"))
        .await
        .unwrap();
    assert_eq!(gap["claim"], "outside_window");
    assert_eq!(gap["version"], newest["version"]);
    assert_eq!(
        gap["resync"]["nextBeforeOrdinal"],
        newest["nextBeforeOrdinal"]
    );
    assert_eq!(gap["resync"]["messages"], newest["messages"]);
    assert_eq!(gap["model"], newest["model"]);
    assert_eq!(gap["lastRun"], json!({"runId":"r1", "state":"active"}));
    assert!(gap["resync"].get("lastRun").is_none());
    let mut page = gap["resync"].clone();
    let mut boundary = 20_000;
    loop {
        let rows = page["messages"].as_array().unwrap();
        assert!(rows
            .iter()
            .all(|row| row["ordinal"].as_u64().unwrap() < boundary));
        boundary = rows[0]["ordinal"].as_u64().unwrap();
        if rows.iter().any(|row| row["messageId"] == "m19889") {
            break;
        }
        let mut older = request();
        older["beforeOrdinal"] = page["nextBeforeOrdinal"].clone();
        page = app.read(TRANSCRIPT_PAGE, older).await.unwrap();
    }
    let matched = app
        .read(TRANSCRIPT_PAGE, held(&newest, "m19998"))
        .await
        .unwrap();
    assert_eq!(matched["claim"], "matched");
    assert_eq!(matched["newTurns"].as_array().unwrap().len(), 1);
    assert_eq!(matched["newTurns"][0]["messageId"], "m19999");
    route.history.store(8, Ordering::Relaxed);
    let expired = app
        .read(TRANSCRIPT_PAGE, held(&newest, "m19889"))
        .await
        .unwrap();
    assert_eq!(expired["claim"], "cursor_expired");
    assert_eq!(expired["lastRun"], newest["lastRun"]);
    assert_eq!(
        route
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call["method"] == "session.history" && call["params"]["form"] == "raw")
            .count(),
        1,
        "the version mismatch must not add another anchor lookup"
    );
}

async fn anchor_case(mid: &str, answer: Option<Result<Value, ReadError>>) {
    let route = super::bounded_reads::populated(200);
    if let Some(answer) = answer {
        route
            .anchor_replies
            .lock()
            .unwrap()
            .insert(mid.into(), answer);
    }
    let (_, app) = serving(route.clone());
    let newest = app.read(TRANSCRIPT_PAGE, request()).await.unwrap();
    let reply = app.read(TRANSCRIPT_PAGE, held(&newest, mid)).await.unwrap();
    assert_eq!(reply["claim"], "cursor_expired", "{mid}: {reply}");
    assert_eq!(reply["resync"]["messages"], newest["messages"]);
    let calls = route.calls.lock().unwrap();
    let anchors: Vec<_> = calls
        .iter()
        .filter(|call| call["method"] == "session.history" && call["params"]["form"] == "raw")
        .collect();
    assert_eq!(anchors.len(), 1);
    assert_eq!(
        anchors[0]["params"],
        json!({"from_mid":mid, "limit":1, "form":"raw"})
    );
}

#[tokio::test]
async fn an_unknown_gap_anchor_expires() {
    anchor_case("missing", None).await;
}

#[tokio::test]
async fn an_unnumbered_gap_anchor_expires() {
    anchor_case(
        "old",
        Some(Ok(
            json!({"messages":[entry("old", None, 1)], "lineage_id":"cut-a", "history_version":7}),
        )),
    )
    .await;
}

#[tokio::test]
async fn a_different_returned_gap_anchor_expires() {
    anchor_case("m1", Some(Ok(json!({"messages":[entry("m2", Some(2), 1)], "lineage_id":"cut-a", "history_version":7})))).await;
}

#[tokio::test]
async fn every_gap_anchor_refusal_or_malformed_answer_expires() {
    for code in [
        "unknown_mid",
        "history_unrepresentable",
        "provider_unavailable",
        "invalid_provider_reply",
    ] {
        anchor_case(
            "m1",
            Some(Err(ReadError::new(code, "no positive anchor proof"))),
        )
        .await;
    }
    anchor_case("m1", Some(Ok(json!({"messages":[]})))).await;
}
