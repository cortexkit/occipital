//! Broca serving llm-runner's `send_ids` (0.8.20), recorded from its real
//! serializers on a synthetic session, before and after, for both of its
//! storages (`wal.*` and `store.*`). Copied unchanged into
//! `tests/data/broca-send-ids-1110/` from the set Broca published for its
//! readers (its `SHA256SUMS` hashes to `5b6eace4429f23b0…`), all but
//! `before-capture.log`, the build log of the capture, which names private
//! paths and holds nothing a reader decodes.
//!
//! A row names the send it came from only through these facts: `send_id` beside
//! `sender` on the entry. Wake records are joined to rows that way
//! (`crate::notices::wakes`), so before this change no Broca row ever found its
//! wake record. An entry whose send stored no id carries none, and the reader
//! never infers one.

use super::served_samples::{decoded, Served};
use super::tool_display_samples::AnyHarness;
use super::*;
use cortexkit_test_support::ScratchDir;
use std::path::{Path, PathBuf};
use subc_client_rs::HandlerOutcome;
use subc_protocol::Principal;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-send-ids-1110")
}

fn body(side: &str, name: &str) -> Value {
    let bytes = std::fs::read(root().join(side).join(format!("{name}.json"))).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn page(side: &str, name: &str) -> RawPage {
    let value = runner_result(body(side, name)).unwrap_or_else(|e| panic!("{side}/{name}: {e:?}"));
    serde_json::from_value(value).unwrap_or_else(|e| panic!("{side}/{name}: {e}"))
}

const SIDES: [&str; 2] = ["before", "after"];
const READS: [&str; 4] = [
    "wal.session.read",
    "store.session.read",
    "wal.session.history",
    "store.session.history",
];

/// The sends the after side names, by entry: every kind of send that stores
/// its id (a keyed send, a steer, an ordered steer, a turn-system delivery, an
/// unkeyed send under the id the runner returned, a queued drain, an idle
/// start). `m13`'s send stored none, so it names none.
const NAMED: [(&str, &str); 7] = [
    ("m2", "keyed-send"),
    ("m3", "steer-send"),
    ("m4", "ordered-steer-send"),
    ("m5", "system-send"),
    ("m7", "sub-unkeyed"),
    ("m9", "queued-send"),
    ("m11", "idle-send"),
];
const SENDER: &str = "sample-sender";

#[test]
fn the_send_ids_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 12);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        if name == "./before-capture.log" {
            continue;
        }
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The after describe adds `send_ids` and changes nothing this reader uses.
#[test]
fn the_describe_adds_send_ids_and_reads_the_same_otherwise() {
    let before = check_describe(&body("before", "role.describe")).unwrap();
    let after = check_describe(&body("after", "role.describe")).unwrap();
    assert!(!before.declares("send_ids"));
    assert!(after.declares("send_ids"));
    assert_eq!(stored_media(Some(&before)), stored_media(Some(&after)));
}

/// Every read builds a row for each entry on both sides; only the after side
/// names sends, exactly the ones listed in `NAMED`.
#[test]
fn every_read_builds_its_rows_and_names_exactly_the_stored_sends() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    for side in SIDES {
        for name in READS {
            let page = page(side, name);
            let rows = source
                .rows(&page.messages, false, &page.pruned_reasoning)
                .unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"));
            assert_eq!(rows.len(), page.messages.len(), "{side}/{name}");
            let sends: Vec<(String, String, String)> =
                crate::notices::wakes::sends_of(&page.messages, "broca", "ses_head")
                    .into_iter()
                    .map(|sent| (sent.mid, sent.sender, sent.send_id))
                    .collect();
            let expected: Vec<(String, String, String)> = if side == "before" {
                Vec::new()
            } else {
                NAMED
                    .iter()
                    .map(|(mid, send)| (mid.to_string(), SENDER.into(), send.to_string()))
                    .collect()
            };
            assert_eq!(sends, expected, "{side}/{name}");
        }
    }
}

/// End to end on the after page: the module that sent a message wrote its wake
/// record keyed by its send id, and the apps' records read finds it for that
/// row. A row whose send stored no id, and a send with no record, get none.
#[tokio::test]
async fn a_broca_row_finds_the_wake_record_of_its_send() {
    let dir = ScratchDir::new("occipital-broca-send-ids");
    let descriptor = crate::store::tests::descriptor(&dir);
    let main = crate::store::open(&descriptor).unwrap();
    let file = Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
    let (notices, _task) = crate::notices::Notices::start(file);
    let slot = crate::notices::NoticeSlot::default();
    let _ = slot.set(notices.clone());

    let record = |text: &str| {
        json!({"items": [{"kind": "work_result", "refs": {"task": "bg_example"},
            "at": 1_793_599_990_000_u64, "excerpt": text}]})
    };
    for (send_id, text) in [("keyed-send", "first"), ("idle-send", "second")] {
        let write = json!({"kind": "wake", "runner": {"kind": "reserved", "module_id": "broca"},
            "session": "ses_head", "send_id": send_id, "body": record(text)});
        let reply = crate::notices::wakes::write(
            Some(&notices),
            Some(Principal::Reserved {
                module_id: SENDER.into(),
            }),
            &write,
        )
        .await;
        assert!(
            matches!(reply, HandlerOutcome::Response(_)),
            "{send_id}: {reply:?}"
        );
    }

    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = page("after", "store.session.read").messages;
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(Hosts("broca", "broca")))),
            Arc::new(AnyHarness(route.clone())),
        )
        .with_notices(slot),
    )));
    app.set_notices(notices);
    let mut served = Served::new("broca-send-ids-1110", app);
    let request = |method: &str, name: &str, value: Value| {
        let mut params = json!({"session": "ses_head", "harness": "broca"});
        params[name] = value;
        json!({"method": method, "params": params})
    };

    let page_request = request(super::super::super::TRANSCRIPT_PAGE, "limit", json!(50));
    let page = served.read(1, &page_request).await;
    let ids: Vec<Value> = decoded(&page)["result"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["messageId"].clone())
        .collect();
    assert_eq!(ids.len(), 15);

    let records_request = request(crate::notices::wakes::READ, "messageIds", json!(ids));
    let records = served.read(2, &records_request).await;
    assert_eq!(
        decoded(&records)["result"],
        json!({"messages": {"m2": record("first"), "m11": record("second")}})
    );
}
