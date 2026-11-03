//! Broca with media on (`attachments` group, `session.media_read`) read through every
//! reader path that decodes the surfaces it changes, from the sample set named below.
//!
//! The samples under `occipital-app-reader/tests/fixtures/broca/media-1030/` are Broca's
//! raw serve-path replies and frames for a synthetic session holding a person's image
//! and a tool result with an image and a PDF, captured with media on and with media
//! off. Each file holds one body per line. The `-default` reads are what this reader
//! actually receives, since it never asks for media. This reader never calls
//! `session.media_read`, `run.status` or `run.result` on Broca, so those files, the
//! `descriptors` reads and the family matrices are checked only for their checksums.
//! The decoding checks go through the production decode functions.

use super::*;
use occipital_app_reader::{broca_entries, display_events};
use sha2::{Digest, Sha256};

const FIXTURES: &str = "../occipital-app-reader/tests/fixtures/broca/media-1030";
const MODES: [&str; 2] = ["on", "off"];

fn path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURES)
        .join(name)
}

/// Every body in a sample file, one per line.
fn lines(name: &str) -> Vec<Vec<u8>> {
    std::fs::read(path(name))
        .unwrap()
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

/// A sample's one body, recorded under runner contract 0.7 and read as a runner
/// on the current contract serves it (`current_contract`).
fn body(name: &str) -> Value {
    let lines = lines(name);
    assert_eq!(lines.len(), 1, "{name} holds one body");
    current_contract(serde_json::from_slice(&lines[0]).unwrap())
}

#[test]
fn the_media_samples_are_complete_and_their_checksums_verify() {
    let sums = std::fs::read_to_string(path("SHA256SUMS")).unwrap();
    let mut listed: Vec<String> = sums
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(path(name)).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), digest, "{name}");
            name.to_owned()
        })
        .collect();
    listed.sort();
    let mut expected = Vec::new();
    for mode in MODES {
        for kind in [
            "describe",
            "family-matrix",
            "media-read-refusal",
            "media-read-success",
            "session-history",
            "session-read",
            "session-read-descriptors",
            "subscribe-live",
            "subscribe-replay",
        ] {
            expected.push(format!("{mode}-{kind}.jsonl"));
        }
    }
    for kind in [
        "run-result",
        "run-status",
        "session-history-default",
        "session-read-default",
    ] {
        expected.push(format!("on-{kind}.jsonl"));
    }
    expected.sort();
    assert_eq!(listed, expected);
}

/// The read this reader actually makes: no `media` member. Broca renders every media
/// part as a plain text placeholder, so the rows decode and the person's message
/// shows its text and the placeholder line as text.
#[test]
fn the_default_read_shows_media_as_placeholder_text() {
    let read = body("on-session-read-default.jsonl");
    let entries = read["result"]["messages"].as_array().unwrap().clone();
    for full in [false, true] {
        let rows = broca_entries::rows(&entries, full)
            .unwrap_or_else(|error| panic!("full={full}: {error:?}"));
        assert_eq!(rows.len(), entries.len(), "full={full}");
        let person = serde_json::to_string(&rows[2].display).unwrap();
        assert!(person.contains("look"), "full={full}: {person}");
        assert!(
            person.contains("[media not shown: kind=image"),
            "full={full}: {person}"
        );
    }
}

/// Both describe decoders accept Broca's answer either way: the reader's checked
/// decode, which caches only a contract-valid answer, and the plain decode the app
/// send uses. Media on adds the `attachments` group.
#[test]
fn both_describe_decoders_accept_media_on_and_off() {
    for mode in MODES {
        let value = body(&format!("{mode}-describe.jsonl"))["result"].clone();
        let checked =
            check_describe(&value).unwrap_or_else(|error| panic!("{mode}: checked: {error:?}"));
        let plain: RoleDescribe =
            serde_json::from_value(value).unwrap_or_else(|error| panic!("{mode}: plain: {error}"));
        for describe in [&checked, &plain] {
            assert_eq!(
                describe
                    .capabilities
                    .iter()
                    .any(|group| group == "attachments"),
                mode == "on",
                "{mode}"
            );
        }
    }
}

/// The transcript rows decode from a read holding a person's image and a tool result
/// with an image and a PDF, for list pages and exact reads, and keep their text.
#[test]
fn transcript_rows_decode_from_a_read_holding_media() {
    for mode in MODES {
        let read = body(&format!("{mode}-session-read.jsonl"));
        let entries = read["result"]["messages"].as_array().unwrap().clone();
        for full in [false, true] {
            let rows = broca_entries::rows(&entries, full)
                .unwrap_or_else(|error| panic!("{mode} full={full}: {error:?}"));
            assert_eq!(rows.len(), entries.len(), "{mode} full={full}");
            let text = serde_json::to_string(&rows.iter().map(|r| &r.display).collect::<Vec<_>>())
                .unwrap();
            for expected in ["ready", "seen"] {
                assert!(text.contains(expected), "{mode} full={full}: {text}");
            }
        }
    }
}

/// The display-form history the `clientMessageId` echo reads decodes with media rows.
#[test]
fn the_display_history_decodes_with_media_rows() {
    for mode in MODES {
        let history = body(&format!("{mode}-session-history.jsonl"));
        let page: DisplayPage = serde_json::from_value(history["result"].clone())
            .unwrap_or_else(|error| panic!("{mode}: {error}"));
        assert_eq!(
            page.messages.len(),
            history["result"]["messages"].as_array().unwrap().len(),
            "{mode}"
        );
    }
}

/// Every live and replay frame decodes, normalizes and keeps the feed, including the
/// tool result frame carrying media references.
#[test]
fn every_subscribe_frame_keeps_the_feed() {
    for mode in MODES {
        for kind in ["subscribe-live", "subscribe-replay"] {
            let name = format!("{mode}-{kind}.jsonl");
            let source = BrocaSource::new(resolved(), FakeRoute::new());
            for (index, bytes) in lines(&name).iter().enumerate() {
                let frame = decode_subscription_frame(bytes)
                    .unwrap_or_else(|error| panic!("{name}:{index}: {error:?}"));
                if let Some(normalized) = normalize(frame.clone()) {
                    let _ = display_events::map(&normalized);
                }
                assert!(source.observe(frame), "{name}:{index} ended the feed");
            }
        }
    }
}
