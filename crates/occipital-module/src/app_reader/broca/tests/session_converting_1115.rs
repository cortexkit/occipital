//! Broca answering a send or read that reaches a session while it converts to
//! its new storage with the contract's retryable `transient` (plus
//! `detail.cause: "session_converting"`), where it used to answer
//! `session_converting`, a code the contract doesn't list as retryable. A
//! session that couldn't be converted keeps the permanent
//! `session_unconverted`. Recorded from its real serializer, before and after,
//! and copied unchanged into `tests/data/broca-session-converting-1115/` from
//! the set Broca published for its readers; its README says when each is sent.
//!
//! This reader meets the change in two places, and these tests run each body
//! through them:
//! - a refused send, which reaches the app as `runner_refused` with a class
//!   saying whether the same send may succeed later: it turns `transient`, so
//!   the app sends again quietly instead of holding the message as failed;
//! - a refused read, which reaches the app as the runner being unavailable
//!   either way, so the app retries as before.
//!
//! The set also carries the body Broca sends when routing has no free capacity
//! for the send (`concurrency_full`), which reaches the app as a wait with the
//! time routing names.

use super::*;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/broca-session-converting-1115")
}

fn body(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap())
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

const CONVERTING: [&str; 3] = [
    "background-converting.json",
    "open-while-converting.json",
    "startup-window-converting.json",
];

#[test]
fn the_session_converting_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    // Four pairs, the capacity body and the README.
    assert_eq!(sums.lines().count(), 10);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The app's answer to a refused send.
fn send_refusal(name: &str) -> occipital_app_reader::send::SendRefusal {
    let refusal = body(name);
    crate::app_reader::send::refused(
        refusal["code"].as_str().unwrap(),
        refusal["message"].as_str().unwrap(),
        refusal.get("detail"),
    )
}

/// A send refused while its session converts becomes one the app may send
/// again; before, its outer code made it final, whatever its detail said. A
/// session that couldn't be converted stays final, before and after.
#[test]
fn a_send_refused_while_the_session_converts_turns_retryable() {
    for name in CONVERTING {
        let before = send_refusal(&format!("before/{name}"));
        let after = send_refusal(&format!("after/{name}"));
        assert_eq!(
            (before.code, after.code),
            ("runner_refused", "runner_refused")
        );
        assert_eq!(
            before.detail,
            Some(json!({"class": "final", "runnerCode": "session_converting"})),
            "{name}"
        );
        assert_eq!(
            after.detail,
            Some(json!({"class": "transient", "runnerCode": "transient"})),
            "{name}"
        );
    }
    for side in ["before", "after"] {
        let permanent = send_refusal(&format!("{side}/startup-window-unconverted.json"));
        assert_eq!(
            (permanent.code, permanent.detail),
            (
                "runner_refused",
                Some(json!({"class": "final", "runnerCode": "session_unconverted"}))
            ),
            "{side}"
        );
    }
}

/// A refused read reaches the app as the runner being unavailable, which it
/// retries, under every code in the set.
#[test]
fn a_read_refused_while_the_session_converts_reads_as_before() {
    for side in ["before", "after"] {
        for name in CONVERTING
            .iter()
            .chain(["startup-window-unconverted.json"].iter())
        {
            let refused = result(body(&format!("{side}/{name}"))).unwrap_err();
            assert_eq!(refused.code, "provider_unavailable", "{side}/{name}");
        }
    }
}

/// A send refused because routing has no free capacity is a wait: the app
/// holds it for the 30 s routing names, then sends it again.
#[test]
fn a_send_refused_for_capacity_waits_the_time_routing_names() {
    let refusal = send_refusal("concurrency-full-0441.json");
    assert_eq!(refusal.code, "runner_refused");
    assert_eq!(
        refusal.detail,
        Some(
            json!({"class": "transient", "runnerCode": "account_selection_refused",
            "retryAfterMs": 30_000})
        )
    );
}
