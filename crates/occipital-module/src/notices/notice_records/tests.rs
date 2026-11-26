use super::*;
use crate::notices::wakes::Sent;
use crate::notices::Notice;
use cortexkit_test_support::ScratchDir;
use serde_json::json;

/// The contract's own notice vectors, packaged with tool-display.
fn vectors() -> Value {
    let (_, text) = cortexkit_role_tool_display::vectors::FILES
        .iter()
        .find(|(name, _)| *name == "notices.json")
        .unwrap();
    serde_json::from_str(text).unwrap()
}

fn started(dir: &ScratchDir) -> (crate::store::Store, Arc<Notices>) {
    let descriptor = crate::store::tests::descriptor(dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, _task) = Notices::start(file);
    (main, notices)
}

const WRITER: &str = "example-writer";

fn writer(module: &str) -> Option<Principal> {
    Some(Principal::Reserved {
        module_id: module.into(),
    })
}

/// Every view served, with a data schema that takes any data, or none.
fn serving(serves: bool) -> ServesView {
    Arc::new(move |_: &str, _: &str| serves.then(Schema::default))
}

async fn stored(notices: &Arc<Notices>, write: &Value) -> Result<WriteReply, ErrorBody> {
    store(Some(notices), None, writer(WRITER), write).await
}

/// Each of the contract's notice writes is stored once, the same bytes again
/// are already stored, different bytes at the same send are a conflict, and
/// the record is served as the contract says: the view qualified by the
/// attested sender.
#[tokio::test]
async fn the_contract_writes_are_stored_once_and_served_qualified() {
    let dir = ScratchDir::new("occipital-notice-writes");
    let (_main, notices) = started(&dir);
    for case in vectors()["cases"].as_array().unwrap() {
        let (write, tag) = (&case["write"], &case["tag"]);
        assert_eq!(
            stored(&notices, write).await,
            Ok(WriteReply::Stored),
            "{tag}"
        );
        assert_eq!(
            stored(&notices, write).await,
            Ok(WriteReply::AlreadyStored),
            "{tag}"
        );
        let served = notices
            .store
            .notice_record_of(
                write["session"].as_str().unwrap(),
                WRITER,
                write["send_id"].as_str().unwrap(),
                &serving(true),
            )
            .unwrap();
        assert_eq!(served.as_ref(), Some(&case["served"]), "{tag}");
    }
    let mut changed = vectors()["cases"][0]["write"].clone();
    changed["body"]["data"]["title"] = json!("Different words");
    assert_eq!(
        stored(&notices, &changed).await.unwrap_err().code,
        "display_write_conflict"
    );
}

/// A notice record a module announces on the bus and serves with
/// `display.read` is kept with the notices, as the same write pushed is,
/// under the module the read went to, and its view is asked for once stored.
#[tokio::test]
async fn a_pulled_notice_write_is_kept_with_the_notices() {
    use crate::display::pull::{pull, Providers, Pulled, Sink};
    struct Serves(Value);
    #[async_trait::async_trait]
    impl Providers for Serves {
        async fn verified(&self, _: &str) -> Result<bool, String> {
            Ok(true)
        }
        async fn read(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Value, crate::display::daemon::CallFailure> {
            Ok(self.0.clone())
        }
    }
    let dir = ScratchDir::new("occipital-pulled-notice");
    let (_main, notices) = started(&dir);
    let case = &vectors()["cases"][0];
    let (wants, mut wanted) = tokio::sync::mpsc::channel(8);
    // No display store: a notice record must not need one.
    let sink = Sink {
        state: None,
        views: Some(crate::display::views::Fetcher::from_queue(wants)),
        hints: None,
        notices: Some(notices.clone()),
    };
    let provider = Serves(case["write"].clone());
    for _ in 0..2 {
        assert_eq!(
            pull(&provider, &sink, WRITER, "rec-1").await,
            Pulled::Stored
        );
    }
    let served = notices
        .store
        .notice_record_of("session-example", WRITER, "send-finished", &serving(true))
        .unwrap();
    assert_eq!(served.as_ref(), Some(&case["served"]));
    // Asked for once, by the first pull that stored the record.
    assert_eq!(
        wanted.try_recv().unwrap(),
        crate::display::views::Want::Writer(WRITER.into(), "nv1:example-writer:finished:1".into())
    );
    assert!(wanted.try_recv().is_err());
}

/// `value` with `pointer`'s member set to `to`.
fn with(mut value: Value, pointer: &str, to: Value) -> Value {
    let (parent, member) = pointer.rsplit_once('/').unwrap();
    value
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .unwrap()
        .insert(member.to_owned(), to);
    value
}

/// Every refusal the contract lists is refused with its code and field, and
/// every malformed write as `invalid_params`, so a writer gets the same answer
/// from Occipital as from the contract's own checks.
#[tokio::test]
async fn the_contract_refusals_are_refused_as_listed() {
    let dir = ScratchDir::new("occipital-notice-refusals");
    let (_main, notices) = started(&dir);
    let vectors = vectors();
    for case in vectors["refusals"].as_array().unwrap() {
        let tag = &case["tag"];
        let base = match case["base"].as_str() {
            Some(base) => vectors[base].clone(),
            None => vectors["cases"][0]["write"].clone(),
        };
        let write = match (case["pointer"].as_str(), case["append"].as_str()) {
            (Some(pointer), Some(append)) => {
                let grown = format!(
                    "{}{append}",
                    base.pointer(pointer).unwrap().as_str().unwrap()
                );
                with(base, pointer, json!(grown))
            }
            (Some(pointer), None) => with(base, pointer, case["value"].clone()),
            _ => base,
        };
        let by = match &case["writer"] {
            Value::Null => writer(WRITER),
            principal => Some(serde_json::from_value(principal.clone()).unwrap()),
        };
        let refused = store(Some(&notices), None, by, &write).await.unwrap_err();
        assert_eq!(refused.code, case["code"], "{tag}");
        if !case["detail"].is_null() {
            assert_eq!(refused.detail.as_ref(), Some(&case["detail"]), "{tag}");
        }
    }
    for case in vectors["malformed"].as_array().unwrap() {
        let write = with(
            vectors["cases"][0]["write"].clone(),
            case["pointer"].as_str().unwrap(),
            case["value"].clone(),
        );
        let refused = stored(&notices, &write).await.unwrap_err();
        assert_eq!(refused.code, "invalid_params", "{}", case["tag"]);
    }
}

/// The send a page read named for message `mid` of `session`.
fn sent(session: &str, mid: &str, send_id: &str) -> Sent {
    Sent {
        runner: "example-runner".into(),
        session: session.into(),
        mid: mid.into(),
        sender: WRITER.into(),
        send_id: send_id.into(),
    }
}

/// A wake records read that asks for notice records gets each row's record,
/// only when its writer serves the record's view; one that doesn't ask gets
/// the reply it always got.
#[tokio::test]
async fn a_row_gets_its_record_only_when_asked_and_its_writer_serves_the_view() {
    let dir = ScratchDir::new("occipital-notice-rows");
    let (_main, notices) = started(&dir);
    let case = &vectors()["cases"][0];
    stored(&notices, &case["write"]).await.unwrap();
    notices.sent(vec![sent("session-example", "m1", "send-finished")]);
    let read = |params: Value, serves: Option<ServesView>| {
        let notices = notices.clone();
        async move {
            notices
                .wake_records("example-runner", "session-example", &params, serves)
                .await
                .unwrap()
        }
    };
    let asked = json!({"messageIds": ["m1"], "acceptNoticeRecords": true});
    assert_eq!(
        read(asked.clone(), Some(serving(true))).await,
        json!({"messages": {}, "notices": {"m1": case["served"]}})
    );
    // A view its writer doesn't serve joins nothing, and the row draws the
    // folded notice.
    assert_eq!(
        read(asked, Some(serving(false))).await,
        json!({"messages": {}, "notices": {}})
    );
    // Without the opt-in the reply is the one older apps decode.
    assert_eq!(
        read(json!({"messageIds": ["m1"]}), Some(serving(true))).await,
        json!({"messages": {}})
    );
}

/// A row whose history moved here from another session takes the one record
/// its send has, as a wake record's row does, and none when two sessions hold
/// one for that send.
#[tokio::test]
async fn a_moved_row_takes_its_send_s_only_record() {
    let dir = ScratchDir::new("occipital-notice-moved");
    let (_main, notices) = started(&dir);
    let write = vectors()["cases"][0]["write"].clone();
    stored(&notices, &write).await.unwrap();
    notices.sent(vec![sent("moved-here", "m1", "send-finished")]);
    let asked = json!({"messageIds": ["m1"], "acceptNoticeRecords": true});
    let read = || async {
        notices
            .wake_records("example-runner", "moved-here", &asked, Some(serving(true)))
            .await
            .unwrap()["notices"]
            .clone()
    };
    assert_eq!(read().await, json!({"m1": vectors()["cases"][0]["served"]}));
    let elsewhere = with(write, "/session", json!("another-session"));
    stored(&notices, &elsewhere).await.unwrap();
    assert_eq!(read().await, json!({}));
}

/// A notice that rode a tool result gets its send's record on a
/// `session.notices` read that asks for it, beside the notice text.
#[tokio::test]
async fn a_riding_notice_gets_its_record_when_asked() {
    let dir = ScratchDir::new("occipital-notice-riding");
    let (_main, notices) = started(&dir);
    let case = &vectors()["cases"][0];
    stored(&notices, &case["write"]).await.unwrap();
    let mut arrivals = notices.arrivals();
    notices.seen(vec![Notice {
        runner: "example-runner".into(),
        session: "session-example".into(),
        call_key: "call-1".into(),
        send_id: "send-finished".into(),
        sender: WRITER.into(),
        text: "Build example finished.".into(),
        delivered_as: "tool_result".into(),
        speaks_for: None,
        position: Some(0),
    }]);
    tokio::time::timeout(std::time::Duration::from_secs(5), arrivals.recv())
        .await
        .unwrap()
        .unwrap();
    let read = |params: Value| {
        let notices = notices.clone();
        async move {
            notices
                .read(
                    "example-runner",
                    "session-example",
                    &params,
                    Some(serving(true)),
                )
                .await
                .unwrap()["calls"]["call-1"][0]
                .clone()
        }
    };
    let asked = read(json!({"callKeys": ["call-1"], "acceptNoticeRecords": true})).await;
    assert_eq!(asked["noticeRecord"], case["served"]);
    assert_eq!(asked["text"], "Build example finished.");
    let plain = read(json!({"callKeys": ["call-1"]})).await;
    assert!(plain.get("noticeRecord").is_none(), "{plain}");
}

/// The module that wakes the agent in the pointer tests, as prefrontal does.
const WAKER: &str = "example-waker";

/// The contract's wake that points at a notice record
/// (`awaited_work_notice_record`), and the record-key-only notice it points
/// at (`record_key_without_send`), as the contract packages them.
fn pointer_vectors() -> (Value, Value) {
    let (_, text) = cortexkit_role_tool_display::vectors::FILES
        .iter()
        .find(|(name, _)| *name == "wakes.json")
        .unwrap();
    let wakes: Value = serde_json::from_str(text).unwrap();
    let wake = wakes["writes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["tag"] == "awaited_work_notice_record")
        .unwrap()["write"]
        .clone();
    let record = vectors()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["tag"] == "record_key_without_send")
        .unwrap()
        .clone();
    (wake, record)
}

/// Notices with the pointing wake stored by [`WAKER`], and message `m1` named
/// as that wake's row.
async fn woken(dir: &ScratchDir) -> (crate::store::Store, Arc<Notices>, Value) {
    woken_by(dir, pointer_vectors().0).await
}

/// [`woken`] with `wake` as the pointing wake's write.
async fn woken_by(dir: &ScratchDir, wake: Value) -> (crate::store::Store, Arc<Notices>, Value) {
    let (main, notices) = started(dir);
    let outcome = crate::notices::wakes::write(Some(&notices), writer(WAKER), &wake).await;
    assert!(matches!(outcome, HandlerOutcome::Response(_)));
    notices.sent(vec![Sent {
        runner: "example-runner".into(),
        session: "session-example".into(),
        mid: "m1".into(),
        sender: WAKER.into(),
        send_id: "send-awaited-work".into(),
    }]);
    (main, notices, wake["body"].clone())
}

async fn records_of(notices: &Arc<Notices>, serves: ServesView) -> Value {
    let asked = json!({"messageIds": ["m1"], "acceptNoticeRecords": true});
    notices
        .wake_records("example-runner", "session-example", &asked, Some(serves))
        .await
        .unwrap()
}

/// A wake that points at a notice record its writer stored under a record id
/// it never sent gives its row that record, beside the wake record, which
/// stays the fallback; without the opt-in the reply is unchanged.
#[tokio::test]
async fn a_wake_s_row_takes_the_record_its_wake_points_at() {
    let dir = ScratchDir::new("occipital-notice-pointer");
    let (_main, notices, wake) = woken(&dir).await;
    let (_, record) = pointer_vectors();
    stored(&notices, &record["write"]).await.unwrap();
    assert_eq!(
        records_of(&notices, serving(true)).await,
        json!({"messages": {"m1": wake}, "notices": {"m1": record["served"]}})
    );
    let plain = notices
        .wake_records(
            "example-runner",
            "session-example",
            &json!({"messageIds": ["m1"]}),
            Some(serving(true)),
        )
        .await
        .unwrap();
    assert_eq!(plain, json!({"messages": {"m1": wake}}));
}

/// Every way the pointed record fails to qualify leaves the row with the
/// wake's own card alone: no record, a view its writer doesn't serve, data
/// that doesn't fit the view, a record another module wrote under the same
/// id, and a record kept for another session.
#[tokio::test]
async fn a_wake_keeps_its_own_card_when_the_pointed_record_does_not_qualify() {
    let none = json!({});
    let (_, record) = pointer_vectors();
    let write = record["write"].clone();
    let dir = ScratchDir::new("occipital-notice-pointer-missing");
    let (_main, notices, _) = woken(&dir).await;
    assert_eq!(records_of(&notices, serving(true)).await["notices"], none);

    let dir = ScratchDir::new("occipital-notice-pointer-refused");
    let (_main, notices, _) = woken(&dir).await;
    stored(&notices, &write).await.unwrap();
    assert_eq!(records_of(&notices, serving(false)).await["notices"], none);
    let unfit = occipital_registry::view::input(&json!({"type": "object", "required": ["absent"]}));
    let serves_unfit: ServesView = Arc::new(move |_: &str, _: &str| Some(unfit.clone()));
    assert_eq!(records_of(&notices, serves_unfit).await["notices"], none);

    let dir = ScratchDir::new("occipital-notice-pointer-other-writer");
    let (_main, notices, _) = woken(&dir).await;
    store(Some(&notices), None, writer("another-writer"), &write)
        .await
        .unwrap();
    assert_eq!(records_of(&notices, serving(true)).await["notices"], none);

    let dir = ScratchDir::new("occipital-notice-pointer-other-session");
    let (_main, notices, _) = woken(&dir).await;
    let elsewhere = with(write, "/session", json!("another-session"));
    stored(&notices, &elsewhere).await.unwrap();
    assert_eq!(records_of(&notices, serving(true)).await["notices"], none);
}

/// The pointed record is drawn in the wake's place, so a wake that holds
/// anything besides that one notice, a second item or items it left out,
/// keeps its own card even when the record qualifies.
#[tokio::test]
async fn a_wake_with_more_than_its_one_notice_keeps_its_own_card() {
    let (wake, record) = pointer_vectors();
    let item = wake["body"]["items"][0].clone();
    for (tag, body) in [
        (
            "two items",
            with(
                wake["body"].clone(),
                "/items",
                json!([item.clone(), item.clone()]),
            ),
        ),
        (
            "items left out",
            with(wake["body"].clone(), "/omittedItems", json!(3)),
        ),
    ] {
        let dir = ScratchDir::new("occipital-notice-pointer-crowded");
        let (_main, notices, kept) = woken_by(&dir, with(wake.clone(), "/body", body)).await;
        stored(&notices, &record["write"]).await.unwrap();
        assert_eq!(
            records_of(&notices, serving(true)).await,
            json!({"messages": {"m1": kept}, "notices": {}}),
            "{tag}"
        );
    }
}

/// A wake that rode a tool result gets the record its wake points at as the
/// notice's `noticeRecord`, as a wake row does.
#[tokio::test]
async fn a_riding_wake_gets_the_record_its_wake_points_at() {
    let dir = ScratchDir::new("occipital-notice-pointer-riding");
    let (_main, notices, wake) = woken(&dir).await;
    let (_, record) = pointer_vectors();
    stored(&notices, &record["write"]).await.unwrap();
    let mut arrivals = notices.arrivals();
    notices.seen(vec![Notice {
        runner: "example-runner".into(),
        session: "session-example".into(),
        call_key: "call-1".into(),
        send_id: "send-awaited-work".into(),
        sender: WAKER.into(),
        text: "The background command finished.".into(),
        delivered_as: "tool_result".into(),
        speaks_for: None,
        position: Some(0),
    }]);
    tokio::time::timeout(std::time::Duration::from_secs(5), arrivals.recv())
        .await
        .unwrap()
        .unwrap();
    let read = notices
        .read(
            "example-runner",
            "session-example",
            &json!({"callKeys": ["call-1"], "acceptNoticeRecords": true}),
            Some(serving(true)),
        )
        .await
        .unwrap();
    let notice = &read["calls"]["call-1"][0];
    assert_eq!(notice["wake"], wake);
    assert_eq!(notice["noticeRecord"], record["served"]);
}
