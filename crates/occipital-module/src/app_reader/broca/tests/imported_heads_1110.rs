//! Broca's reads of chats imported from Thalamus's runners, recorded through
//! its real import and serve path from Thalamus's own session exports: one
//! OpenCode 2 chat and one Pi chat. Copied unchanged into
//! `tests/data/broca-imported-heads-1110/` from the set Broca published for
//! its readers, with its README.
//!
//! An imported call keeps the key the Thalamus runner minted for it and names
//! that runner as its `call_carrier`, on the read's tool times and on the
//! notices that rode it. Occipital files such a call's notices under that
//! runner and key, so the apps find them by the new session and the key.
//!
//! The exports carry no schema pins or call times, so neither do Broca's
//! reads: an imported call reaches the apps with its key but no `schemaPin`
//! or times. Without a pin a call can't be joined to a tool view, and draws as
//! a plain chip, as these tests pin.

use super::*;
use cortexkit_test_support::ScratchDir;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-imported-heads-1110")
}

fn result(host: &str, name: &str) -> Value {
    let bytes = std::fs::read(root().join(host).join(format!("{name}.json"))).unwrap();
    runner_result(serde_json::from_slice(&bytes).unwrap())
        .unwrap_or_else(|error| panic!("{host}/{name}: {error:?}"))
}

/// Each chat's imported calls, as Thalamus's runner minted them, and the
/// riding notice on the first: `(host, carrier, call keys, notice sender,
/// notice text)`.
const CHATS: [(&str, &str, &[&str], &str, &str); 2] = [
    (
        "opencode2",
        THALAMUS_RUNNER_MODULE_ID,
        &[
            "msg_1b5fa8dea0010w1FHaCCIhgG5k/call_probe",
            "msg_1b5fa9c0a001BvMuQLEmMEQeV3/call_each",
        ],
        "magic-context",
        "RIDE_TEXT",
    ),
    (
        "pi",
        "thalamus-pi",
        &["5b9fb962/call_probe_1"],
        "notifier",
        "RIDE_REMINDER",
    ),
];

#[test]
fn the_imported_head_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 7);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Every read builds a row for each entry, and display history decodes.
#[test]
fn every_imported_read_builds_its_rows() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    for (host, ..) in CHATS {
        for name in ["read-default-1", "read-display-facts-1"] {
            let page: RawPage = serde_json::from_value(result(host, name)).unwrap();
            let rows = source
                .rows(&page.messages, false, &page.pruned_reasoning)
                .unwrap_or_else(|error| panic!("{host}/{name}: {error:?}"));
            assert_eq!(rows.len(), page.messages.len(), "{host}/{name}");
            assert!(
                rows.iter()
                    .all(|row| row.display.get("unreadable").is_none()),
                "{host}/{name}"
            );
        }
        let display: DisplayPage =
            serde_json::from_value(result(host, "history-display-1")).unwrap();
        assert!(!display.messages.is_empty(), "{host}");
    }
}

/// The session the chat had on Thalamus's runner before Broca imported it.
const OLD_SESSION: &str = "ses_before_import";

/// On the page the apps read, each imported call keeps its original key and
/// has no pin or times, and the separate module message reads as that
/// module's notice. The note that rode the first call was kept while Thalamus's
/// runner served the chat; once Broca serves it, the note follows the chat to
/// its new session, found there by the call's key, and is one note, not two:
/// Broca names the runner that minted the call, which it was kept under.
#[tokio::test]
async fn an_imported_call_keeps_its_key_and_its_riding_notice() {
    for (host, carrier, keys, sender, text) in CHATS {
        let dir = ScratchDir::new("occipital-imported-heads");
        let descriptor = crate::store::tests::descriptor(&dir);
        let main = crate::store::open(&descriptor).unwrap();
        let file =
            Arc::new(crate::notices::NoticeStore::open(&descriptor, main.data_root()).unwrap());
        let (notices, _task) = crate::notices::Notices::start(file.clone());
        let slot = crate::notices::NoticeSlot::default();
        let _ = slot.set(notices.clone());

        let page: RawPage = serde_json::from_value(result(host, "read-display-facts-1")).unwrap();
        // Kept from the chat's reads on Thalamus's runner, before the import.
        let before = crate::notices::from_entries(&page.messages, carrier, OLD_SESSION);
        assert_eq!(before.len(), 1, "{host}");
        notices.seen(before);
        for _ in 0..1_000 {
            if !file
                .for_call(carrier, OLD_SESSION, keys[0])
                .unwrap()
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let route = FakeRoute::new();
        *route.entries.lock().unwrap() = page.messages;
        *route.run_state.lock().unwrap() = Value::Null;
        let app = Arc::new(super::super::super::AppReader::new(Arc::new(
            BrocaSources::new(
                Arc::new(Resolver::new(Arc::new(Lookup))),
                Arc::new(Routes(route)),
            )
            .with_notices(slot),
        )));
        app.set_notices(notices);
        let mut params = params();
        params["limit"] = json!(50);
        let page = app
            .read(super::super::super::TRANSCRIPT_PAGE, params)
            .await
            .unwrap();
        let rows = page["messages"].as_array().unwrap();
        let calls: Vec<&Value> = rows
            .iter()
            .flat_map(|row| row["parts"].as_array().unwrap())
            .filter(|part| part["type"] == "tool")
            .collect();
        let served: Vec<&str> = calls
            .iter()
            .map(|call| call["callKey"].as_str().unwrap())
            .collect();
        let mut expected: Vec<&str> = keys.to_vec();
        let mut found = served.clone();
        expected.sort_unstable();
        found.sort_unstable();
        assert_eq!(found, expected, "{host}");
        for call in &calls {
            for absent in ["schemaPin", "startedAtMs", "finishedAtMs", "callCarrier"] {
                assert!(call.get(absent).is_none(), "{host}: {absent} on {call}");
            }
        }
        let marked: Vec<&Value> = rows.iter().filter_map(|row| row.get("notice")).collect();
        assert_eq!(marked, [&json!({"sender": sender})], "{host}");

        let ask = json!({"session": "alfonso:s1", "harness": "broca", "callKeys": [keys[0]]});
        let mut read = Value::Null;
        for _ in 0..1_000 {
            read = app.read(crate::notices::READ, ask.clone()).await.unwrap();
            if read["calls"].get(keys[0]).is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(
            read["calls"][keys[0]],
            json!([{"sender": sender, "text": text, "deliveredAs": "tool_result"}]),
            "{host}"
        );
        assert!(
            file.for_call(carrier, OLD_SESSION, keys[0])
                .unwrap()
                .is_empty(),
            "{host}: the note stayed in the old session"
        );
    }
}
