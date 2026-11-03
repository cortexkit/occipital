//! The empty-chat samples the apps pin, `phone-reader-empty-claim-v2`. Its
//! README says what each file shows and what changed from v1. The replies come from a test Broca session
//! rather than a live one, because a live session can't be held empty on
//! demand; `served_samples` says how the set is checked and rewritten.

use super::served_samples::{decoded, Served};
use super::*;
use subc_protocol::FrameType;

#[tokio::test(start_paused = true)]
async fn the_empty_chat_samples_are_what_the_reader_serves() {
    // A session that has never run: no messages and no last run.
    let route = FakeRoute::new();
    route.entries.lock().unwrap().clear();
    *route.run_state.lock().unwrap() = Value::Null;
    let (_, app) = serving(route.clone());
    let mut served = Served::new("phone-reader-empty-claim-v2", app);

    // A session with no messages: an empty newest page that says why.
    let page_request = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": "alfonso:s1", "harness": "broca", "limit": 50}});
    let page = served.read(1, &page_request).await;
    let result = &decoded(&page)["result"];
    assert_eq!(result["messages"], json!([]));
    assert_eq!(result["reachedStart"], true);
    assert_eq!(result["emptyReason"], "empty");
    let version = result["version"].clone();
    served.keep("01-empty-page", &page_request, page);

    // The same page claimed with `heldVersion` alone: unchanged.
    let mut claim_request = page_request.clone();
    claim_request["params"]["claim"] = json!({"heldVersion": version});
    let unchanged = served.read(2, &claim_request).await;
    assert_eq!(decoded(&unchanged)["result"]["unchanged"], true);
    served.keep("02-empty-claim-unchanged", &claim_request, unchanged);

    // A watch holding the empty page says nothing while it stays empty, past the
    // slow sweep, and answers the first message as a replaced page.
    let mut watch_request = claim_request.clone();
    watch_request["method"] = json!(super::super::super::TRANSCRIPT_WATCH);
    served.send(3, &watch_request).await;
    let sweep = occipital_app_reader::transcript_watch::TRANSCRIPT_SWEEP;
    assert!(
        served.next(3, sweep * 2).await.is_none(),
        "the watch spoke early"
    );
    route
        .entries
        .lock()
        .unwrap()
        .push(json!({"mid": "m1", "ordinal": 1,
        "created_at_ms": 705, "message": {"role": "user",
        "content": [{"type": "text", "text": "Hello"}]}}));
    let (ty, notice) = served.next(3, sweep * 2).await.unwrap();
    assert_eq!(ty, FrameType::StreamData);
    let first = decoded(&notice);
    assert_eq!(first["claim"], "cursor_expired", "{first}");
    assert_eq!(first["resync"]["messages"][0]["messageId"], "m1");
    served.keep("03-empty-claim-watch-first-message", &watch_request, notice);
    served.cancel(3);

    // The same claim read once the message is there: the replaced page.
    let replaced = served.read(4, &claim_request).await;
    let result = &decoded(&replaced)["result"];
    assert_eq!(result["claim"], "cursor_expired", "{result}");
    assert_eq!(result["resync"]["messages"][0]["messageId"], "m1");
    assert!(result.get("validThroughMessageId").is_none());
    assert!(result.get("vouchedFromMessageId").is_none());
    served.keep("04-empty-claim-first-message", &claim_request, replaced);

    served.check();
}
