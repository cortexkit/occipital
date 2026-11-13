//! Broca giving every run error a `provider_code`, as the runner contract has
//! always required: an error with no code now carries one derived from its
//! class (`transient`, `auth_required`, `context_overflow`, and
//! `provider_error` for anything else). Recorded from its real serializers on
//! both of its storages (`legacy` and the new `store`), before and after, for
//! a codeless permanent ending, a codeless transient ending and a sign-in
//! pause. Copied unchanged into `tests/data/broca-run-provider-code-1113/`
//! from the set Broca published for its readers; its README lists every byte
//! that changed.
//!
//! This reader decodes run errors in three places, and these tests run each
//! after body through them:
//! - a chat's `lastRun.error`, from `session.head`'s `last_run_error`, which
//!   passes `providerCode` only for a code the contract names;
//! - a queued send whose turn ended in an error, which takes its code from
//!   `run.status` on the same terms;
//! - subscribe frames, of which it reads only whether they are live events.

use super::run_awaits_1112::expected_error;
use super::session_facts::{assert_last_run_phone_bytes, expected_last_run};
use super::*;
use cortexkit_role_llm_runner::errors::provider_codes;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-run-provider-code-1113")
}

fn body(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap())
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn names(suffix: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(suffix))
        .collect();
    names.sort();
    names
}

/// The before body beside an after one.
fn before(after: &str) -> Value {
    body(&after.replace(".after.", ".before."))
}

#[test]
fn the_provider_code_samples_keep_their_checksums() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert!(format!("{:x}", Sha256::digest(&sums)).starts_with("0e9104369e5da486"));
    let sums = String::from_utf8(sums).unwrap();
    let mut listed = 0;
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
        listed += 1;
    }
    assert_eq!(listed, 57);
}

/// Each error that had no code now has the one its class names.
#[test]
fn every_codeless_error_gets_the_code_its_class_names() {
    let mut errors = 0;
    for name in names(".json")
        .iter()
        .filter(|name| name.contains(".after."))
    {
        let after = body(name);
        let error = [
            &after["error"],
            &after["last_run_error"],
            &after["unit"]["error"],
        ]
        .into_iter()
        .find(|error| error.is_object());
        let Some(error) = error else { continue };
        let code = match error["class"].as_str() {
            Some("transient") => "transient",
            Some("auth_required") => "auth_required",
            Some("context_overflow") => "context_overflow",
            _ => "provider_error",
        };
        assert_eq!(error["provider_code"], code, "{name}");
        errors += 1;
    }
    assert_eq!(errors, 22);
}

/// Every head reaches the apps as `lastRun` with its wait and error, byte for
/// byte, on details, the newest page, a claim and a watch. The derived codes
/// aren't ones this contract version names, so `providerCode` stays out and a
/// chat's error reads as it did before; the new store's waits are added.
#[tokio::test]
async fn every_head_reaches_the_apps_as_its_last_run() {
    let heads = names(".session.head.json");
    assert_eq!(heads.len(), 12);
    for name in &heads {
        let head = body(name);
        let state = head["last_run_state"].clone();
        let error = head.get("last_run_error").cloned();
        let mut expected = expected_last_run(&state);
        if let Some(error) = &error {
            expected["error"] = expected_error(error);
            if let Some(code) = error["provider_code"].as_str() {
                assert!(!provider_codes::CODES.contains(&code), "{name}: {code}");
            }
        }
        let expected = serde_json::to_string(&expected).unwrap();
        assert_last_run_phone_bytes(state, error, &expected).await;
        if name.contains(".after.") {
            let before = before(name);
            let error = |head: &Value| head.get("last_run_error").map(expected_error);
            assert_eq!(error(&head), error(&before), "{name}");
        }
    }
}

/// A queued send's ended turn takes `run.status`'s code only when the
/// contract names it, so these derived codes leave it listed with its reason
/// alone, as before; and Stop's `state` is unchanged.
#[test]
fn run_status_reads_as_before() {
    for name in names(".after.run.status.json") {
        let after = body(&name);
        let before = before(&name);
        assert_eq!(after["state"], before["state"], "{name}");
        let named = |status: &Value| {
            status["error"]["provider_code"]
                .as_str()
                .filter(|code| provider_codes::CODES.contains(code))
                .map(str::to_owned)
        };
        assert_eq!(named(&after), None, "{name}");
        assert_eq!(named(&after), named(&before), "{name}");
    }
}

/// Every subscribe frame is a live event exactly when its before frame was,
/// and does the same to the live view.
#[test]
fn subscribe_frames_map_as_before() {
    let frames: Vec<String> = names(".json")
        .into_iter()
        .filter(|name| name.contains(".after.subscribe."))
        .collect();
    assert_eq!(frames.len(), 10);
    let mapped = |body: &Value| normalize(body.clone()).map(|event| display_events::map(&event));
    for name in &frames {
        let after = body(name);
        let before = before(name);
        assert_eq!(
            carries_live_event(&after),
            carries_live_event(&before),
            "{name}"
        );
        assert_eq!(mapped(&after), mapped(&before), "{name}");
    }
}
