//! Broca's replies before and after its reads name each message's role on the
//! read envelope (`role`: `user`, `assistant` or `tool` here), recorded from its
//! real serve path for two fresh sessions holding the same four messages.
//! Copied unchanged into `tests/data/broca-read-role-1108/` from the set Broca
//! published for its readers, each body with Broca's own checksum line. The
//! `run.result` control is left out, since Occipital never reads it.
//!
//! The envelope member is new to Occipital's decoders: rows take a message's
//! role from the stored message itself, so every row must read exactly as it
//! did before. `session.head`, display history and live frames don't change,
//! and are decoded as controls.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-read-role-1108")
}

fn bytes(side: &str, form: &str) -> Vec<u8> {
    let name = format!("{side}.read-role.{form}.json");
    std::fs::read(root().join(&name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// A recorded reply's `result`.
fn result(side: &str, form: &str) -> Value {
    let body: Value = serde_json::from_slice(&bytes(side, form)).unwrap();
    runner_result(body).unwrap_or_else(|error| panic!("{side}/{form}: {error:?}"))
}

/// The forms that carry read envelopes, and so gain `role`.
const READ_FORMS: &[&str] = &[
    "read-default",
    "read-head",
    "read-range",
    "read-page-1",
    "read-page-2",
    "read-page-3",
    "read-page-4",
    "read-facts",
    "read-descriptors",
    "history-raw",
];

/// What a reader sees of a row, without the ids and times that differ between
/// the two recorded sessions.
fn shape(row: &Value) -> Value {
    let parts: Vec<Value> = row["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|part| {
            json!({"type": part["type"], "text": part["text"], "tool": part["tool"],
                "content": part["content"], "isError": part["isError"]})
        })
        .collect();
    json!({"role": row["role"], "text": row["text"], "parts": parts})
}

fn rows(side: &str, form: &str) -> (Vec<Value>, Vec<Value>) {
    let page: RawPage = serde_json::from_value(result(side, form))
        .unwrap_or_else(|error| panic!("{side}/{form}: {error}"));
    let envelope_roles = page
        .messages
        .iter()
        .map(|message| message["role"].clone())
        .collect();
    let rows = BrocaSource::new(resolved(), FakeRoute::new())
        .rows(&page.messages, false, &page.pruned_reasoning)
        .unwrap_or_else(|error| panic!("{side}/{form}: {error:?}"))
        .into_iter()
        .map(|row| shape(&row.display))
        .collect();
    (rows, envelope_roles)
}

#[test]
fn the_read_role_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 56);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Every read form decodes and builds the same rows before and after, while
/// the after side does carry the new member, so the check isn't vacuous.
#[test]
fn rows_read_the_same_with_the_envelope_role() {
    let whole = ["user", "assistant", "tool", "assistant"];
    for form in READ_FORMS {
        let (before, before_roles) = rows("before", form);
        let (after, after_roles) = rows("after", form);
        assert!(!after.is_empty(), "{form}");
        assert_eq!(before, after, "{form}");
        assert!(before_roles.iter().all(Value::is_null), "{form}");
        let expected: &[&str] = match *form {
            "read-page-1" => &whole[0..1],
            "read-page-2" => &whole[1..2],
            "read-page-3" => &whole[2..3],
            "read-page-4" => &whole[3..4],
            _ => &whole,
        };
        assert_eq!(after_roles, expected.to_vec(), "{form}");
    }
    let (rows, _) = rows("after", "read-default");
    assert_eq!(
        rows.iter()
            .map(|row| row["role"].clone())
            .collect::<Vec<_>>(),
        ["user", "assistant", "tool", "assistant"]
    );
}

/// The unchanged surfaces still decode: the head passes the runner contract's
/// consistency check (`head_meta`: a history counter and log identity the pages
/// can be read against), the display history reads, and every live frame maps
/// and keeps the feed.
#[test]
fn the_control_replies_still_decode() {
    for side in ["before", "after"] {
        head_meta(&result(side, "head-control"))
            .unwrap_or_else(|error| panic!("{side}: {error:?}"));
        let display: DisplayPage = serde_json::from_value(result(side, "history-display"))
            .unwrap_or_else(|error| panic!("{side}: {error}"));
        assert_eq!(display.messages.len(), 4, "{side}");
        let source = BrocaSource::new(resolved(), FakeRoute::new());
        let mut index = 0;
        while root()
            .join(format!("{side}.read-role.subscribe-control-{index}.json"))
            .exists()
        {
            let form = format!("subscribe-control-{index}");
            let frame = decode_subscription_frame(&bytes(side, &form))
                .unwrap_or_else(|error| panic!("{side}/{form}: {error:?}"));
            if let Some(normalized) = normalize(frame.clone()) {
                let _ = display_events::map(&normalized);
            }
            assert!(source.observe(frame), "{side}/{form} ended the feed");
            index += 1;
        }
        assert_eq!(index, 16, "{side}");
    }
}
