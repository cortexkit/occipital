//! Broca's replies once its runner contract 0.8.21 writer runs on contract
//! 0.8.28 (Broca 0.3.329), recorded from its real serve path, through every
//! reader path that decodes them. Copied unchanged into
//! `tests/data/broca-0821-writer-1108/` from the set Broca published for its
//! readers (its `SHA256SUMS` hashes to `738f1d197b292a22…`). Only the reply
//! bodies Occipital decodes are copied, each with Broca's own checksum line;
//! the set's logs and refusal captures are left out. What changes for
//! Occipital:
//! - `role.describe` declares `message_marks` and `media_refs`, so page reads
//!   ask for references and a stored photo arrives as a file part.
//! - With display facts, a module's message names its sender and either the
//!   agent it speaks for or that it's the module's own notice.
//! - A tool result keeps the flattened text of the default read, markers
//!   included, under `media_refs` too.
//! - A live `notice_delivered` names its form.

use super::*;
use cortexkit_role_llm_runner::send::SendReply;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-0821-writer-1108")
}

fn body(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap())
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// A recorded reply's `result`.
fn result(name: &str) -> Value {
    runner_result(body(name)).unwrap_or_else(|error| panic!("{name}: {error:?}"))
}

/// A recorded page's rows, as the reader builds them.
fn rows(name: &str) -> Vec<Value> {
    let page: RawPage =
        serde_json::from_value(result(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
    BrocaSource::new(resolved(), FakeRoute::new())
        .rows(&page.messages, false, &page.pruned_reasoning)
        .unwrap_or_else(|error| panic!("{name}: {error:?}"))
        .into_iter()
        .map(|row| row.display)
        .collect()
}

#[test]
fn the_0821_writer_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 16);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Both describes pass the contract's check and the plain decode the send path
/// uses. Only the writer's declares the marks and media references, and the
/// page read asks for references only from it.
#[test]
fn the_0821_writer_describes_decode_and_choose_the_media_form() {
    for (name, writer) in [
        ("role-describe.before.json", false),
        ("role-describe.after.json", true),
    ] {
        let value = result(name);
        let checked = check_describe(&value).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let plain: RoleDescribe = serde_json::from_value(value).unwrap();
        for describe in [&checked, &plain] {
            for group in ["read_display_facts", "notices", "attachments"] {
                assert!(describe.declares(group), "{name}: {group}");
            }
            for group in ["message_marks", "media_refs"] {
                assert_eq!(describe.declares(group), writer, "{name}: {group}");
            }
        }
        assert_eq!(
            super::super::stored_media(Some(&checked)).is_some(),
            writer,
            "{name}"
        );
    }
}

/// With display facts, a head's words to its worker read as that agent's and
/// core's own message reads as a notice. The default read carries neither
/// mark, so its rows are left unmarked.
#[test]
fn the_0821_writer_marks_reach_the_rows() {
    let marked = rows("marks-read.after.json");
    assert_eq!(
        marked[1]["speaksFor"],
        json!({"agentId": "Ada"}),
        "{}",
        marked[1]
    );
    let notice = rows("notice-read.after.json");
    assert_eq!(
        notice[1]["speaksFor"],
        json!({"agentId": "Ada"}),
        "{}",
        notice[1]
    );
    assert_eq!(
        notice[3]["notice"],
        json!({"sender": "prefrontal-core"}),
        "{}",
        notice[3]
    );
    for row in [&notice[2], &notice[4]] {
        for mark in ["speaksFor", "notice", "sentBy"] {
            assert!(row.get(mark).is_none(), "{row}");
        }
    }
    for row in rows("marks-default-read.after.json") {
        for mark in ["speaksFor", "notice", "sentBy"] {
            assert!(row.get(mark).is_none(), "{row}");
        }
    }
}

/// The raw history page builds rows, and the display history decodes.
#[test]
fn the_0821_writer_history_pages_decode() {
    assert_eq!(rows("marks-history-raw.after.json").len(), 3);
    let display: DisplayPage =
        serde_json::from_value(result("marks-history-display.after.json")).unwrap();
    assert_eq!(display.messages.len(), 3);
}

/// Under `media_refs` the uploaded test image is a file part with its digest,
/// type, size and pixel size, which `session.media` serves. Before the
/// writer, Broca's read served only the marker text.
#[test]
fn the_0821_writer_photo_arrives_as_a_file_part() {
    let parts = |name: &str| rows(name)[0]["parts"].clone();
    let file = &parts("photo-media-refs.after.json")[1];
    let shown: Value = ["type", "mime", "byteCount", "width", "height", "sha256"]
        .into_iter()
        .map(|field| (field.to_owned(), file[field].clone()))
        .collect::<serde_json::Map<_, _>>()
        .into();
    assert_eq!(
        shown,
        json!({"type": "file", "mime": "image/png", "byteCount": 3592,
            "width": 640, "height": 160,
            "sha256": "2d317c5c185e947275cc6005f0da44adb3430cea4c01f5ee6d1eafce0fc017f3"}),
        "{file}"
    );
    let before = parts("photo-media-refs.before.json");
    assert!(
        before
            .as_array()
            .unwrap()
            .iter()
            .all(|part| part["type"] == "text"),
        "{before}"
    );
}

/// A tool result that carries media serves the same flattened text under
/// `media_refs` as under the default read, so its rows, output size included,
/// are the same.
#[test]
fn the_0821_writer_tool_result_reads_the_same_in_both_media_forms() {
    let line = |name: &str| -> Value {
        let text = std::fs::read_to_string(root().join(name)).unwrap();
        serde_json::from_str(text.lines().next().unwrap()).unwrap()
    };
    let rows_of = |body: Value| {
        let page: RawPage = serde_json::from_value(runner_result(body).unwrap()).unwrap();
        BrocaSource::new(resolved(), FakeRoute::new())
            .rows(&page.messages, false, &[])
            .unwrap()
            .into_iter()
            .map(|row| row.display)
            .collect::<Vec<_>>()
    };
    let inline = rows_of(line("runner-0821-tool-result-inline.jsonl"));
    let refs = rows_of(line("runner-0821-tool-result-media-refs.jsonl"));
    assert_eq!(inline, refs);
    let call = inline
        .iter()
        .flat_map(|row| row["parts"].as_array().cloned().unwrap_or_default())
        .find(|part| part["type"] == "tool")
        .unwrap();
    assert!(
        call["outputBytes"].as_u64().is_some_and(|bytes| bytes > 0),
        "{call}"
    );
}

/// A live `notice_delivered` names its form now; before, the form was the
/// default. Either way it's kept as a notice that rode the tool result
/// (`tool_result`), naming the agent it speaks for.
#[test]
fn the_0821_writer_live_notice_names_its_form() {
    for name in [
        "notice-delivered.before.json",
        "notice-delivered.after.json",
    ] {
        let notice = crate::notices::from_event(&body(name), "broca", "alfonso:s1")
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(notice.delivered_as, "tool_result", "{name}");
        assert_eq!(notice.speaks_for.as_deref(), Some("Ada"), "{name}");
        assert_eq!(notice.sender, "prefrontal-core", "{name}");
    }
    assert_eq!(
        body("notice-delivered.after.json")["delivered_as"],
        "tool_result"
    );
}

/// The send path decodes each send reply, marked or not.
#[test]
fn the_0821_writer_send_replies_decode() {
    for name in [
        "marks-send.after.json",
        "notice-send.after.json",
        "unmarked-send.json",
    ] {
        let reply: SendReply =
            serde_json::from_value(result(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(reply.state, "active", "{name}");
    }
}
