//! Agent Runner's wave-3 samples: Pi and Claude Code serving capability groups
//! OpenCode 2 already declares. Recorded from real Pi 1.6.0 and Claude Code
//! 2.1.330 against the runner's committed tree, and copied unchanged into
//! `tests/data/thalamus-host-parity-wave3/`, one folder per set with its README.

use super::*;
use crate::notices::wakes;
use occipital_app_reader::display_events;
use std::path::{Path, PathBuf};

const SETS: [(&str, &str); 4] = [
    ("pi-shared-groups-v1", "1d5f4d00"),
    ("pi-delivery-groups-v1", "80e3931b"),
    ("pi-run-limits-v1", "a95c08ae"),
    ("claude-code-groups-v1", "75b8b68c"),
];

fn root(set: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-host-parity-wave3")
        .join(set)
}

fn read(set: &str, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root(set).join(name)).unwrap()).unwrap()
}

#[test]
fn the_wave3_samples_keep_their_checksums() {
    for (set, prefix) in SETS {
        let sums = std::fs::read(root(set).join("SHA256SUMS")).unwrap();
        assert!(
            format!("{:x}", Sha256::digest(&sums)).starts_with(prefix),
            "{set}"
        );
        for line in String::from_utf8(sums).unwrap().lines() {
            let (expected, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(root(set).join(name)).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                expected,
                "{set}/{name}"
            );
        }
    }
}

/// Every set's description passes the contract's check, and each declares the
/// groups Occipital acts on: `rich_ui_stamp` (where recorded), the display
/// facts, photo references and "Send now".
#[test]
fn each_description_passes_and_declares_what_occipital_reads() {
    for (set, _) in SETS {
        let describe = check_describe(&read(set, "role-describe.json")).unwrap();
        for group in [
            "read_display_facts",
            "media_refs",
            "send_promote",
            "plan_view",
        ] {
            assert!(describe.declares(group), "{set}: {group}");
        }
    }
    for set in ["pi-shared-groups-v1", "claude-code-groups-v1"] {
        let describe = check_describe(&read(set, "role-describe.json")).unwrap();
        assert!(describe.declares("rich_ui_stamp"), "{set}");
        assert!(describe.declares("head_sender"), "{set}");
    }
}

/// A plan sender's head carries its grant; Occipital, a reader with none, may
/// not send, which `head_sender` makes a definite answer, not an unknown.
#[test]
fn a_reader_without_a_grant_may_not_send() {
    let describe = check_describe(&read("pi-shared-groups-v1", "role-describe.json")).unwrap();
    let grant = |name: &str| {
        let head = read("pi-shared-groups-v1", &format!("head-sender-{name}.json"));
        send_grant(&head_meta(&head).unwrap(), Some(&describe), false)
    };
    assert!(matches!(grant("occipital"), SendGrant::MayNotSend));
    for name in ["prefrontal-core", "stopper", "notifier"] {
        assert!(matches!(grant(name), SendGrant::Granted(_)), "{name}");
    }
}

/// The stamp reaches the run's assistant rows only on a read that asks for it,
/// and the run's start carries it for live text.
#[test]
fn the_rich_ui_stamp_reaches_stamped_rows_and_the_run_start() {
    let stamps = |name: &str| -> Vec<Value> {
        let page = read("pi-shared-groups-v1", name);
        broca_entries::display_rows(
            page["messages"].as_array().unwrap(),
            false,
            broca_entries::DisplayProfile::Pi,
            &[],
        )
        .unwrap()
        .into_iter()
        .filter_map(|row| row.display.get("richUi").cloned())
        .collect()
    };
    assert!(stamps("rich-ui-read-default.json").is_empty());
    let stamp = json!({"catalogDigest": "a".repeat(64), "dialect": "1.0"});
    assert_eq!(
        stamps("rich-ui-read-opted-in.json"),
        vec![stamp.clone(), stamp.clone()]
    );
    let started = read("pi-shared-groups-v1", "rich-ui-run-started.json");
    match display_events::map(&json!({"kind": "control", "event": started})) {
        Some(display_events::DisplayInput::RunStart { rich_ui, .. }) => {
            assert_eq!(rich_ui, Some(stamp))
        }
        other => panic!("{other:?}"),
    }
}

/// No live frame in the sets is one the live view reads as unknown, which
/// would clear the words on screen: Pi's control frames and Claude Code's
/// interim `tool_finished` all decode.
#[test]
fn no_live_frame_clears_the_live_view() {
    let streaming = read("pi-delivery-groups-v1", "streaming.json");
    let mut frames: Vec<Value> = streaming["control_frames"].as_array().unwrap().clone();
    for name in [
        "cc-call-yield-tool-finished.json",
        "cc-notice-yield-tool-finished.json",
    ] {
        frames.push(json!({"kind": "control", "event": read("claude-code-groups-v1", name)}));
    }
    for frame in &frames {
        assert!(
            !matches!(
                display_events::map(frame),
                Some(display_events::DisplayInput::Reset)
            ),
            "{frame}"
        );
    }
}

/// Each user entry names its own sender's send id, which joins wake records;
/// assistant entries name none.
#[test]
fn claude_code_reads_name_each_senders_send_id() {
    let page = read("claude-code-groups-v1", "send-ids-read.json");
    let sent: Vec<(String, String)> = wakes::sends_of(
        page["messages"].as_array().unwrap(),
        "thalamus-harness",
        "s",
    )
    .into_iter()
    .map(|sent| (sent.sender, sent.send_id))
    .collect();
    assert_eq!(
        sent,
        [("prefrontal-core", "shared-key"), ("aft", "shared-key")]
            .map(|(sender, id)| (sender.to_owned(), id.to_owned()))
    );
}

/// The late result of a handed-back call arrives as AFT's own notice message,
/// not a ride, so the row is marked as AFT's notice.
#[test]
fn a_handed_back_calls_late_result_is_aft_notice_row() {
    for name in [
        "cc-call-yield-late-read.json",
        "cc-notice-yield-late-read.json",
    ] {
        let page = read("claude-code-groups-v1", name);
        let entries = page["messages"].as_array().unwrap().clone();
        assert!(crate::notices::from_entries(&entries, "thalamus-harness", "s").is_empty());
        let mut rows = broca_entries::display_rows(
            &entries,
            false,
            broca_entries::DisplayProfile::ClaudeCode,
            &[],
        )
        .unwrap();
        mark_module_messages(&mut rows, &entries);
        let late = rows
            .iter()
            .find(|row| {
                row.display["text"]
                    .as_str()
                    .unwrap_or("")
                    .contains("HANDED-BACK CALL FINISHED")
            })
            .unwrap();
        assert_eq!(late.display["notice"]["sender"], "aft", "{name}");
    }
}
