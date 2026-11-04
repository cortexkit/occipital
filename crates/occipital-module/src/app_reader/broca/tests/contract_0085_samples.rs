//! Broca's replies after it adopts runner contract 0.8.4 and 0.8.5, recorded
//! from its serve path, through every reader and send path that decodes them.
//! Copied unchanged into `tests/data/broca-contract-0.8.5/` from the set Broca
//! published for its readers (its `SHA256SUMS` hashes to `065b544add6fc8a3…`),
//! with Broca's README, which says how each pair was captured; its "pin-only
//! step" is Broca moving to the new contract crate with no new behaviour
//! switched on, whose replies matched the earlier ones. Before and after each
//! change:
//! - `role.describe` declares `media_only_send` beside `attachments`.
//! - `session.head`'s input names the model's image limits, or leaves them out
//!   where the provider documents none.
//! - `session.read` with display facts gains the reply's `completed_at`, a GPT
//!   reasoning summary, and each message's recorded `created_at`.
//! - A send of a photo alone is accepted, and its stored message holds only the
//!   photo; an image over a limit is refused `media_too_large` naming it.
//! - A plan that widens a frozen session's senders is refused naming
//!   `plan.senders`. This module never sends a plan, so those two files are
//!   checked only as refusals the send path may meet.

use super::*;
use cortexkit_role_llm_runner::send::SendReply;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-contract-0.8.5")
}

/// A recorded reply body as the runner sent it.
fn body(side: &str, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(side).join(name)).unwrap()).unwrap()
}

/// A recorded reply's `result`.
fn result(side: &str, name: &str) -> Value {
    runner_result(body(side, name)).unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"))
}

#[test]
fn the_contract_0085_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 25);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Both describes pass the contract's check the reader requires and the plain
/// decode the send path uses. Only the later one declares photos alone, and
/// both still declare what sending and display facts depend on.
#[test]
fn broca_0085_describes_decode_on_both_paths() {
    for (side, media_only) in [("before", false), ("after", true)] {
        let value = result(side, "runner-role.describe.jsonl");
        let checked = check_describe(&value).unwrap_or_else(|error| panic!("{side}: {error:?}"));
        let plain: RoleDescribe = serde_json::from_value(value).unwrap();
        for describe in [&checked, &plain] {
            for group in ["attachments", "read_display_facts", "head_sender"] {
                assert!(describe.declares(group), "{side}: {group}");
            }
            assert_eq!(describe.declares("media_only_send"), media_only, "{side}");
        }
    }
}

/// Each head passes the contract's rule, and the reader's input facts carry the
/// model's image limits where Broca reports them and leave them unknown, never
/// zero, where it doesn't.
#[tokio::test]
async fn broca_0085_heads_carry_image_limits_where_known() {
    for side in ["before", "after"] {
        for (name, limits) in [
            (
                "runner-session.head.known-limits.jsonl",
                Some((7_500_000, 8_000)),
            ),
            ("runner-session.head.unknown-limits.jsonl", None),
        ] {
            let head = result(side, name);
            head_meta(&head).unwrap_or_else(|error| panic!("{side}/{name}: {error:?}"));
            let route = FakeRoute::new();
            route
                .head_extra
                .lock()
                .unwrap()
                .insert("input".into(), head["input"].clone());
            let input = BrocaSource::new(resolved(), route)
                .head()
                .await
                .unwrap()
                .input;
            let served = input.max_image_bytes.zip(input.max_image_dimension);
            let expected = if side == "after" { limits } else { None };
            assert_eq!(served, expected, "{side}/{name}");
            assert_eq!(input.accepts("image/png"), Some(true), "{side}/{name}");
        }
    }
}

/// With display facts, a message's recorded creation time is its row's start,
/// as recorded, even when later than the reply's finish; the reply's finish
/// and its GPT reasoning summary reach the row, and the encrypted reasoning
/// never does.
#[test]
fn broca_0085_display_facts_reach_the_rows() {
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    let rows_of = |side: &str, name: &str| {
        let page: RawPage = serde_json::from_value(result(side, name)).unwrap();
        source.rows(&page.messages, false, &[]).unwrap()
    };
    let dated = rows_of("after", "session.read.created_at.json");
    assert_eq!(dated[0].written_at, Some(250));
    assert_eq!(dated[1].written_at, Some(600));
    assert_eq!(dated[1].display["completedAtMs"], 505);

    let facts = rows_of("after", "session.read.display_facts.json");
    let reply = &facts[1].display;
    assert_eq!(reply["completedAtMs"], 505);
    let reasoning: Vec<&Value> = reply["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "reasoning")
        .collect();
    assert_eq!(reasoning.len(), 1, "{reply}");
    assert_eq!(reasoning[0]["text"], "I checked the arithmetic.");
    assert!(!reply.to_string().contains("synthetic-opaque"), "{reply}");
    let without = rows_of("before", "session.read.display_facts.json");
    assert!(
        !without[1]
            .display
            .to_string()
            .contains("I checked the arithmetic."),
        "{}",
        without[1].display
    );
}

/// A photo sent alone is accepted as any send is, and its stored message reads
/// as a person's row holding only the photo, with its digest, and no text part.
#[test]
fn a_photo_sent_alone_is_accepted_and_reads_as_a_file() {
    let reply: SendReply = serde_json::from_value(result("after", "runner-photo-only.send.jsonl"))
        .unwrap_or_else(|error| panic!("send reply: {error}"));
    assert_eq!(reply.state, "active");
    let page: RawPage =
        serde_json::from_value(result("after", "runner-photo-only.stored-message.jsonl")).unwrap();
    let source = BrocaSource::new(resolved(), FakeRoute::new());
    let rows = source.rows(&page.messages, false, &[]).unwrap();
    assert_eq!(rows.len(), 1);
    let display = &rows[0].display;
    assert_eq!(display["role"], "user");
    let parts = display["parts"].as_array().unwrap();
    assert!(parts.iter().all(|part| part["type"] != "text"), "{display}");
    let file = &display["files"][0];
    assert_eq!(
        file["sha256"],
        "2d317c5c185e947275cc6005f0da44adb3430cea4c01f5ee6d1eafce0fc017f3"
    );
    assert_eq!(
        (&file["mime"], &file["byteCount"]),
        (&json!("image/png"), &json!(3592))
    );
}

/// An image over a model limit reaches the app as `media_too_large`, naming
/// only the limits it exceeded. A refused plan is a final runner refusal, never
/// mistaken for a delivery the session's plan doesn't grant.
#[test]
fn broca_0085_send_refusals_keep_their_meaning() {
    for (name, detail) in [
        (
            "runner-media_too_large.bytes.jsonl",
            json!({"max_image_bytes": 7_500_000}),
        ),
        (
            "runner-media_too_large.dimension.jsonl",
            json!({"max_image_dimension": 8_000}),
        ),
        (
            "runner-media_too_large.both.jsonl",
            json!({"max_image_bytes": 7_500_000, "max_image_dimension": 8_000}),
        ),
    ] {
        let refusal = app_refusal("after", name);
        assert_eq!(refusal.code, "media_too_large", "{name}");
        assert_eq!(refusal.detail, Some(detail), "{name}");
    }
    for name in ["plan.senders.send.json", "plan.senders.refresh.json"] {
        let refusal = app_refusal("after", name);
        assert_eq!(refusal.code, "runner_refused", "{name}");
        assert_eq!(
            refusal.detail,
            Some(json!({"class": "final", "runnerCode": "invalid_params"})),
            "{name}"
        );
    }
}

/// What the app receives for a recorded refusal.
fn app_refusal(side: &str, name: &str) -> occipital_app_reader::send::SendRefusal {
    match runner_result(body(side, name)) {
        Err(crate::app_reader::send::RunnerError::Refused {
            code,
            message,
            detail,
        }) => crate::app_reader::send::refused(&code, &message, detail.as_ref()),
        other => panic!("{side}/{name}: not a refusal: {other:?}"),
    }
}
