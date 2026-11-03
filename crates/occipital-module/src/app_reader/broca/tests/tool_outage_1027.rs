//! Broca tool-outage read-through (`tool-outage-1027`).
//!
//! Broca's real producer now ends a refused tool call with an error object that
//! carries five new optional siblings next to `"message"` (`module`, `code`,
//! `detail`, `nothing_sent`, `retry`). The samples under
//! `occipital-app-reader/tests/fixtures/broca/tool-outage-1027/` capture that
//! shape on a synthetic session. Every sample below goes through the production
//! decode functions that consume it; nothing here re-implements a decoder.
//!
//! Covered paths (file and line are the decode site, not the test):
//! - raw `session.read` entry decoding:
//!   `occipital-app-reader/src/broca_entries.rs` (`rows`, `decode_part`).
//! - transcript page and claim rows served from those entries:
//!   `occipital-module/src/app_reader/broca.rs` (`numbered_window`,
//!   `project_window`, `exact`) and `occipital-app-reader/src/source.rs`
//!   (`TranscriptReader::read`, `Row::value`) with
//!   `occipital-app-reader/src/claim.rs` (`TranscriptPageSnapshot::from_page`).
//! - display reads and watches: `occipital-app-reader/src/display_events.rs`
//!   (`map`) and `occipital-app-reader/src/display.rs` (`DisplayReadRequest`).
//! - live tool events on the subscribe pump:
//!   `occipital-module/src/app_reader/broca.rs`
//!   (`decode_subscription_frame`, `normalize`, `observe`).
//! - `session.history` display rows used for the `clientMessageId` echo:
//!   `occipital-module/src/app_reader/broca.rs`
//!   (`echo_client_message_ids`, via `DisplayPage`).
//! - `tool_result.record.json` is the runner's tool-times record. No decode path
//!   in either reader crate consumes that envelope, so the test shows the inner
//!   result still decodes through the tool-result projection instead.

use super::*;
use crate::app_reader::tests::serve;
use crate::app_reader::{DISPLAY_READ, TRANSCRIPT_PAGE};
use occipital_app_reader::{broca_entries, display_events, transcript_watch};
use sha2::{Digest, Sha256};

const FIXTURES: &str = "../occipital-app-reader/tests/fixtures/broca/tool-outage-1027";
const EXPECTED_MESSAGE: &str = "tool plane unavailable";
const EXPECTED_FILES: [&str; 5] = [
    "session_read.reply.json",
    "session_history.row.json",
    "tool_finished.event.json",
    "tool_finished.frame.json",
    "tool_result.record.json",
];

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture_dir().join(name)).unwrap_or_else(|_| panic!("{name} is present"))
}

fn fixture(name: &str) -> Value {
    serde_json::from_slice::<Value>(&fixture_bytes(name))
        .unwrap_or_else(|_| panic!("{name} parses"))
}

/// The outage session's raw entries, in stored order. Recorded under runner
/// contract 0.7, they are read as a runner on the current contract serves them
/// (`current_contract`).
fn outage_entries() -> Vec<Value> {
    let reply = current_contract(fixture("session_read.reply.json"));
    reply["result"]["messages"]
        .as_array()
        .expect("session.read reply carries messages")
        .clone()
}

/// A fake route serving exactly the outage session's entries.
fn outage_route() -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = outage_entries();
    route
}

fn message_ids(page: &Value) -> Vec<String> {
    transcript_watch::message_ids(page)
}

#[test]
fn the_outage_samples_are_complete_and_their_checksums_verify() {
    let sums = std::fs::read_to_string(fixture_dir().join("SHA256SUMS")).expect("SHA256SUMS");
    let mut listed = Vec::new();
    for line in sums.lines() {
        let (digest, name) = line.split_once("  ").expect("checksum line");
        let bytes = fixture_bytes(name);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest, "{name}");
        let _: Value = serde_json::from_slice::<Value>(&bytes).expect("sample parses");
        listed.push(name.to_string());
    }
    listed.sort();
    let mut expected: Vec<String> = EXPECTED_FILES.map(str::to_string).into();
    expected.sort();
    assert_eq!(listed, expected, "every sample is listed in SHA256SUMS");
    let mut present: Vec<String> = std::fs::read_dir(fixture_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "SHA256SUMS")
        .collect();
    present.sort();
    assert_eq!(present, expected);
}

#[test]
fn the_outage_session_read_decodes_in_compact_and_full_form() {
    let entries = outage_entries();
    assert_eq!(entries.len(), 4, "user, call, tool error, done");
    for full in [false, true] {
        let rows = broca_entries::rows(&entries, full).unwrap();
        assert_eq!(rows.len(), 4, "full={full}");
        assert_eq!(
            rows.iter().map(|row| row.mid.as_str()).collect::<Vec<_>>(),
            ["m0", "m1", "m2", "m3"],
            "full={full}"
        );
        // Nothing else on the page changes: the user text and the closing text
        // stay exactly what the producer stored.
        assert_eq!(rows[0].display["role"], "user");
        assert_eq!(rows[0].display["text"], "hi");
        assert_eq!(rows[3].display["role"], "assistant");
        assert_eq!(rows[3].display["text"], "done");
        assert!(rows[0].display.get("isError").is_none(), "full={full}");
        assert!(rows[3].display.get("isError").is_none(), "full={full}");
    }
    // Compact form folds the tool result into its call: no result body is shown,
    // but the call carries the error flag and the output size.
    let compact = broca_entries::rows(&entries, false).unwrap();
    let call = &compact[1].display["parts"][0];
    assert_eq!(call["tool"], "work");
    assert_eq!(call["callId"], "call-1");
    assert_eq!(call["isError"], true);
    let output = &entries[2]["message"]["content"][0]["output"];
    assert_eq!(output["message"], EXPECTED_MESSAGE);
    assert_eq!(call["outputBytes"], json!(output.to_string().len()));
    assert_eq!(
        compact[1].display["toolCalls"][0]["outputBytes"],
        call["outputBytes"]
    );
    // A tool-only message shows no parts in compact form, and the error text is
    // never pasted into the page body.
    assert_eq!(compact[2].display["parts"], json!([]));
    assert!(
        !serde_json::to_value(compact.iter().map(|row| &row.display).collect::<Vec<_>>())
            .unwrap()
            .to_string()
            .contains(EXPECTED_MESSAGE),
        "compact pages carry the error flag, not the error text"
    );
    // Full form shows the tool result as its own part with the producer's text.
    let full = broca_entries::rows(&entries, true).unwrap();
    let result = &full[2].display["parts"][0];
    assert_eq!(result["type"], "tool");
    assert_eq!(result["callId"], "call-1");
    assert_eq!(result["isError"], true);
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .contains(EXPECTED_MESSAGE),
        "{result}"
    );
}

/// The `session.history` row carries the same tool error through entry decoding.
#[test]
fn the_outage_history_row_decodes_as_a_tool_error() {
    let row = fixture("session_history.row.json");
    assert_eq!(row["mid"], "m2");
    assert_eq!(row["message"]["content"][0]["tool_call_id"], "call-1");
    let decoded = broca_entries::rows(std::slice::from_ref(&row), true).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].mid, "m2");
    let part = &decoded[0].display["parts"][0];
    assert_eq!(part["isError"], true);
    assert!(part["content"].as_str().unwrap().contains(EXPECTED_MESSAGE));
    let compact = broca_entries::rows(std::slice::from_ref(&row), false).unwrap();
    assert_eq!(compact[0].display["parts"], json!([]));
}

/// The live `tool_finished` frame decodes on the subscribe pump and stays
/// ignored by live display mapping, without failing.
#[test]
fn the_outage_tool_finished_event_decodes_and_stays_ignored_by_display() {
    let frame = fixture("tool_finished.frame.json");
    let decoded = decode_subscription_frame(&fixture_bytes("tool_finished.frame.json")).unwrap();
    assert_eq!(decoded["kind"], "control");
    assert_eq!(decoded["unit"]["type"], "tool_result");
    assert_eq!(decoded["event"]["type"], "tool_finished");
    assert_eq!(decoded["event"]["tool_call_id"], "call-1");
    assert_eq!(
        decoded["unit"]["result"]["output"]["message"],
        EXPECTED_MESSAGE
    );
    assert_eq!(
        frame["unit"]["result"]["output"]["message"],
        EXPECTED_MESSAGE
    );
    let normalized = normalize(decoded).expect("an envelope normalizes");
    assert_eq!(normalized["kind"], "control");
    assert_eq!(normalized["event"]["type"], "tool_finished");
    // This `tool_finished` is a call's end on the display lane, never a reset of
    // the live display.
    assert!(is_tool_end(display_events::map(&normalized)));
    let event = fixture("tool_finished.event.json");
    assert_eq!(event["type"], "tool_finished");
    let envelope = json!({"kind": "control", "event": event});
    assert!(is_tool_end(display_events::map(&envelope)));
    // The pump observes the frame without ending the feed.
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    assert!(source.observe(normalized));
}

/// Whether `input` ends call `call-1` with outcome `error`, as the recorded
/// `tool_finished` reported it.
fn is_tool_end(input: Option<display_events::DisplayInput>) -> bool {
    matches!(
        input,
        Some(display_events::DisplayInput::ToolFinished(end))
            if end.call_id == "call-1" && end.outcome == "error"
    )
}

fn outage_page_params() -> Value {
    json!({"session": "alfonso:s1", "harness": "broca", "limit": 4})
}

/// The served newest page shows the tool error on its call and leaves every
/// other row alone; claims and exact reads see the same error.
#[tokio::test]
async fn the_served_outage_page_claim_and_exact_read_show_the_tool_error() {
    let route = outage_route();
    let (_, app) = serving(route);
    let page: Value =
        serde_json::from_slice::<Value>(&serve(&app, TRANSCRIPT_PAGE, &outage_page_params()).await)
            .unwrap()["result"]
            .clone();
    assert_eq!(message_ids(&page), ["m0", "m1", "m2", "m3"]);
    assert_eq!(page["messages"][0]["text"], "hi");
    assert_eq!(page["messages"][3]["text"], "done");
    let call = &page["messages"][1]["parts"][0];
    assert_eq!(call["tool"], "work");
    assert_eq!(call["isError"], true);
    assert!(call["outputBytes"].as_u64().unwrap() > 0);
    // The tool-only row shows no parts on a page, and only the call row carries
    // the error flag.
    assert_eq!(page["messages"][2]["parts"], json!([]));
    for (index, row) in page["messages"].as_array().unwrap().iter().enumerate() {
        if index != 1 {
            assert!(row.get("isError").is_none(), "{row}");
            for part in row["parts"].as_array().unwrap() {
                assert!(part.get("isError").is_none(), "{part}");
            }
        }
    }
    // A claim held at the first message matches with the error among the turns.
    let claim = json!({
        "heldFloorMessageId": "m0",
        "sinceMessageId": "m0",
        "heldVersion": page["version"],
    });
    let mut claimed = outage_page_params();
    claimed["claim"] = claim;
    let reply: Value =
        serde_json::from_slice::<Value>(&serve(&app, TRANSCRIPT_PAGE, &claimed).await).unwrap()
            ["result"]
            .clone();
    assert_eq!(reply["claim"], "matched");
    assert_eq!(
        message_ids(&json!({"messages": reply["newTurns"]})),
        ["m1", "m2", "m3"]
    );
    assert_eq!(reply["newTurns"][0]["parts"][0]["isError"], true);
    // The exact read shows the full tool result with the producer's message.
    let mut exact = outage_page_params();
    exact["messageOrdinal"] = json!(2);
    let read: Value = serde_json::from_slice::<Value>(&serve(&app, TRANSCRIPT_PAGE, &exact).await)
        .unwrap()["result"]
        .clone();
    assert_eq!(read["fullContent"], true);
    let part = &read["messages"][0]["parts"][0];
    assert_eq!(part["callId"], "call-1");
    assert_eq!(part["isError"], true);
    assert!(part["content"].as_str().unwrap().contains(EXPECTED_MESSAGE));
}

/// Display history echo keeps working beside the outage: the tool row decodes
/// in display form and gains no `clientMessageId`; only the user row echoes.
#[tokio::test]
async fn the_outage_display_history_echo_leaves_the_tool_row_alone() {
    use cortexkit_role_llm_runner::display::{DisplayEntry, DisplayRow};
    let route = outage_route();
    *route.describe.lock().unwrap() = Some(describe_with(&["history_display"]));
    for (from, mids) in [(1u64, vec!["m0", "m1"]), (3u64, vec!["m2", "m3"])] {
        let entries = outage_entries();
        let messages: Vec<DisplayEntry> = mids
            .into_iter()
            .map(|mid| {
                let entry = entries.iter().find(|entry| entry["mid"] == mid).unwrap();
                let mut row = DisplayRow::new(entry["message"]["role"].as_str().unwrap(), vec![]);
                if mid == "m0" {
                    row = row.with_client_message_id("phone-abc");
                }
                DisplayEntry::new(
                    entry["ordinal"].as_u64().unwrap(),
                    mid,
                    entry["written_at"].as_u64().unwrap(),
                    row,
                )
            })
            .collect();
        let page = json!({"messages": messages, "incarnation": "inc-a", "history_version": 7});
        // Display form itself is lenient: it decodes without error.
        let decoded: DisplayPage = serde_json::from_value(page.clone()).unwrap();
        assert_eq!(decoded.messages.len(), 2);
        route.history_replies.lock().unwrap().insert(from, Ok(page));
    }
    let (_, app) = serving(route);
    let page: Value =
        serde_json::from_slice::<Value>(&serve(&app, TRANSCRIPT_PAGE, &outage_page_params()).await)
            .unwrap()["result"]
            .clone();
    assert_eq!(page["messages"][0]["clientMessageId"], "phone-abc");
    for row in page["messages"].as_array().unwrap().iter().skip(1) {
        assert!(row.get("clientMessageId").is_none(), "{row}");
    }
    let call = &page["messages"][1]["parts"][0];
    assert_eq!(call["isError"], true);
}

/// A display read with no live projection stays absent, and the outage tool
/// events are a call's end, never a reset.
#[tokio::test]
async fn the_outage_tool_events_create_no_display_state() {
    let frame = fixture("tool_finished.frame.json");
    let envelope = json!({"kind": "control", "event": fixture("tool_finished.event.json")});
    assert!(is_tool_end(display_events::map(&frame)));
    assert!(is_tool_end(display_events::map(&envelope)));
    let route = outage_route();
    let (_, app) = serving(route);
    let reply: Value = serde_json::from_slice::<Value>(
        &serve(&app, DISPLAY_READ, &json!({"session": "alfonso:s1"})).await,
    )
    .unwrap();
    assert_eq!(reply["result"]["absent"], "no_ring_attached");
}

/// The tool-times record envelope has no reader consumer; its inner result
/// still decodes through the tool-result projection with the error intact.
#[test]
fn the_outage_tool_result_record_inner_result_decodes() {
    let record = fixture("tool_result.record.json");
    assert_eq!(record["requires"], json!(["tool-times/v1"]));
    assert_eq!(record["record"]["outcome"], "completed");
    let inner = &record["record"]["result"];
    assert_eq!(inner["tool_call_id"], "call-1");
    assert_eq!(inner["is_error"], true);
    assert_eq!(inner["output"]["message"], EXPECTED_MESSAGE);
    let block = json!({
        "mid": "m2",
        "ordinal": 2,
        "message": {
            "role": "tool",
            "content": [{
                "type": "tool_result",
                "tool_call_id": inner["tool_call_id"],
                "output": inner["output"],
                "is_error": inner["is_error"],
            }],
        },
    });
    let rows = broca_entries::rows(std::slice::from_ref(&block), true).unwrap();
    let part = &rows[0].display["parts"][0];
    assert_eq!(part["isError"], true);
    assert!(part["content"].as_str().unwrap().contains(EXPECTED_MESSAGE));
}
