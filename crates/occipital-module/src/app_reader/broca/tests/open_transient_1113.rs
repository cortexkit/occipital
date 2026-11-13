//! Broca answering a session it can't open yet with the contract's retryable
//! `transient` (plus `detail.cause`: `key_unavailable` while its new storage's
//! key loads after a start, else `open_failed`), where it used to answer
//! `open_failed` for that kind of failure too. A permanent open failure keeps
//! `open_failed`. Recorded from its real serializers, before and after, and
//! copied unchanged into `tests/data/broca-open-transient-1113/` from the set
//! Broca published for its readers; its README says where each body travels.
//!
//! This reader meets the change in three places, and these tests run each
//! body through them:
//! - a refused send, which reaches the app as `runner_refused` with a class
//!   saying whether the same send may succeed later: it turns `transient`, so
//!   the app sends again quietly instead of showing a failure;
//! - a refused read, which reaches the app as the runner being unavailable
//!   either way, so the app retries as before;
//! - a queued send whose turn ended in this error, whose `provider_code` the
//!   reader passes on only when the contract names it as a provider code;
//!   neither `open_failed` nor `transient` is one, so the entry is listed
//!   unsent with its reason alone, as before.
//!
//! Broca's `session.import` refusal is in the set too; this reader never
//! imports a session.

use super::*;
use cortexkit_role_llm_runner::errors::provider_codes;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-open-transient-1113")
}

fn body(side: &str, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(side).join(name)).unwrap())
        .unwrap_or_else(|e| panic!("{side}/{name}: {e}"))
}

#[test]
fn the_open_transient_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    // Six pairs and the README.
    assert_eq!(sums.lines().count(), 13);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The app's answer to a refused send.
fn send_refusal(side: &str, name: &str) -> occipital_app_reader::send::SendRefusal {
    let refusal = body(side, name);
    crate::app_reader::send::refused(
        refusal["code"].as_str().unwrap(),
        refusal["message"].as_str().unwrap(),
        refusal.get("detail"),
    )
}

/// A send refused while the session can't open yet becomes one the app may
/// send again; a permanent open failure stays final, before and after.
#[test]
fn a_send_refused_while_the_session_opens_turns_retryable() {
    let before = send_refusal("before", "send-key-unavailable.json");
    let after = send_refusal("after", "send-key-unavailable.json");
    assert_eq!(
        (before.code, after.code),
        ("runner_refused", "runner_refused")
    );
    assert_eq!(
        before.detail,
        Some(json!({"class": "final", "runnerCode": "open_failed"}))
    );
    assert_eq!(
        after.detail,
        Some(json!({"class": "transient", "runnerCode": "transient"}))
    );
    for side in ["before", "after"] {
        let permanent = send_refusal(side, "send-permanent-unchanged.json");
        assert_eq!(
            (permanent.code, permanent.detail),
            (
                "runner_refused",
                Some(json!({"class": "final", "runnerCode": "open_failed"}))
            ),
            "{side}"
        );
    }
}

/// A refused read reaches the app as the runner being unavailable, which it
/// retries, under either code.
#[test]
fn a_read_refused_while_the_session_opens_reads_as_before() {
    for side in ["before", "after"] {
        let refused = result(body(side, "read-temporary-wal-read.json")).unwrap_err();
        assert_eq!(refused.code, "provider_unavailable", "{side}");
    }
}

/// A queued send whose turn ended in this error is listed unsent with its
/// reason alone, before and after: neither code is a provider code the
/// contract names, so none reaches the app.
#[test]
fn a_queued_send_ended_by_an_open_failure_names_no_code() {
    for name in ["queued-lease-held.json", "queued-panicked.json"] {
        let (before, after) = (body("before", name), body("after", name));
        assert_eq!(before["provider_code"], "open_failed", "{name}");
        assert_eq!(after["provider_code"], "transient", "{name}");
        for code in [&before["provider_code"], &after["provider_code"]] {
            assert!(
                !provider_codes::CODES.contains(&code.as_str().unwrap()),
                "{name}: {code}"
            );
        }
    }
}
