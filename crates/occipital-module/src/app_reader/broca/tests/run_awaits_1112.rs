//! Broca's run waits on its new storage (llm-runner 0.8.45 §6.1.1), recorded
//! from its real serializers on synthetic fixtures: before and after bodies
//! for `run.status`, `session.head`, `run.result` and `session.subscribe`,
//! one set per kind of wait (account limit, sign-in, restart, held call,
//! provider retry, terminal errors). Copied unchanged into
//! `tests/data/broca-run-awaits-1112/` from the set Broca published for its
//! readers; its README says what each file is.
//!
//! What this reader takes from them:
//! - `session.head`: the chat's `lastRun`, with `awaits` and the run's error,
//!   on details, the newest page, a claim and a watch.
//! - `run.status`: only `state`, when Stop and a queued send check a run.
//! - `session.subscribe`: whether a frame is a live event, and what it does
//!   to the live view.
//! - `run.result`: never read.
//!
//! The one change to existing bytes is a held tool call, which the old head
//! called `paused` and the new one calls `active` with `reason: tool_held`,
//! so a chat waiting on a held call now reads busy.

use super::session_facts::{assert_last_run_phone_bytes, expected_last_run};
use super::*;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-run-awaits-1112")
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

#[test]
fn the_run_awaits_samples_keep_their_checksums() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&sums)),
        "cb7363ed0205d7bbae3fe6c03153d8215fe4ee7efd4e0439b57211fc1876a34c"
    );
    let sums = String::from_utf8(sums).unwrap();
    let mut listed = 0;
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
        listed += 1;
    }
    assert_eq!(listed, 121);
}

/// The app's spelling of a head's `last_run_error`, written out from the
/// fields the reader passes: class, its waits, and a provider code only when
/// the runner contract names it.
pub(super) fn expected_error(error: &Value) -> Value {
    let mut expected = json!({"class": error["class"]});
    for (wire, app) in [
        ("retry_after_secs", "retryAfterSecs"),
        ("resets_at_ms", "resetsAtMs"),
    ] {
        if !error[wire].is_null() {
            expected[app] = error[wire].clone();
        }
    }
    if let Some(code) = error["provider_code"]
        .as_str()
        .filter(|code| cortexkit_role_llm_runner::errors::provider_codes::CODES.contains(code))
    {
        expected["providerCode"] = json!(code);
    }
    expected
}

/// Every head, before and after, reaches the apps as `lastRun` with its wait
/// and error, byte for byte, on all four reads.
#[tokio::test]
async fn every_head_reaches_the_apps_as_its_last_run() {
    let heads = names(".session.head.json");
    assert_eq!(heads.len(), 26);
    let mut with_awaits = 0;
    for name in &heads {
        let head = body(name);
        let state = head["last_run_state"].clone();
        let error = head.get("last_run_error").cloned();
        let mut expected = expected_last_run(&state);
        if let Some(error) = &error {
            expected["error"] = expected_error(error);
        }
        if expected.get("awaits").is_some() {
            with_awaits += 1;
        }
        let expected = serde_json::to_string(&expected).unwrap();
        assert_last_run_phone_bytes(state, error, &expected).await;
    }
    // Every after head carries a wait except the error with no provider code,
    // and so does `retry.past-floor`, a reconnect after the retry's next
    // attempt time has passed; no before head carries one.
    assert_eq!(with_awaits, 13);
}

/// The held call is the one head whose state changes: `paused` before,
/// `active` with `tool_held` after, so the chat reads busy, and a message the
/// person sends a worker (a steer, which only joins a running turn) is no
/// longer refused as paused.
#[test]
fn a_held_call_now_reads_active() {
    let before = body("held.before.session.head.json")["last_run_state"].clone();
    let after = body("held.after.session.head.json")["last_run_state"].clone();
    assert_eq!(
        (before["state"].as_str(), after["state"].as_str()),
        (Some("paused"), Some("active"))
    );
    assert_eq!(after["reason"], "tool_held");
    assert_eq!(after["awaits"]["reason"], "tool_held");
    assert_eq!(activity_for(after["state"].as_str()), Some("busy"));
}

/// Stop and the queued-send check read only `state` from `run.status`, and
/// every after body keeps the before body's, the held call included (its
/// status was already `active`).
#[test]
fn run_status_keeps_its_state() {
    for name in names(".after.run.status.json") {
        let before = root().join(name.replace(".after.", ".before."));
        let after = body(&name);
        assert!(after["state"].is_string(), "{name}");
        if before.exists() {
            let before: Value = serde_json::from_slice(&std::fs::read(before).unwrap()).unwrap();
            assert_eq!(after["state"], before["state"], "{name}");
        }
    }
}

/// What each subscribe frame does in this reader: control frames stay
/// control frames and map as before, the wait riding beside what the reader
/// reads. A retry wait comes as a display frame (`kind: "display"`, the
/// lossy stream of live words), `run_status`, which resets the live view, so
/// the abandoned attempt's words don't stay on screen.
#[test]
fn subscribe_frames_map_as_before() {
    let frames = names(".json")
        .into_iter()
        .filter(|name| name.contains("subscribe"))
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 43);
    let mapped = |body: &Value| normalize(body.clone()).map(|event| display_events::map(&event));
    for name in &frames {
        let after = body(name);
        if after["kind"] == "display" {
            let reset = matches!(mapped(&after), Some(Some(DisplayInput::Reset)));
            assert!(reset, "{name}");
            continue;
        }
        assert_eq!(after["kind"], "control", "{name}");
        let before = root().join(name.replace(".after.", ".before."));
        if name.contains(".after.") && before.exists() {
            let before: Value = serde_json::from_slice(&std::fs::read(before).unwrap()).unwrap();
            assert_eq!(
                carries_live_event(&after),
                carries_live_event(&before),
                "{name}"
            );
            assert_eq!(mapped(&after), mapped(&before), "{name}");
        }
    }
}

/// Every display `run_status` makes the chat's watches re-read the head,
/// where the wait is recorded; other display frames don't. The samples hold
/// four: one starting a retry wait, one clearing it, one sent to a subscriber
/// that reconnected after the retry's next attempt time had passed, and one
/// whose `awaits.party` (who ends the wait) is a value this reader doesn't
/// know.
#[test]
fn a_display_run_status_rereads_the_head() {
    let statuses: Vec<String> = names(".json")
        .into_iter()
        .filter(|name| {
            name.contains("subscribe.display.") && !name.ends_with("subscribe.display.json")
        })
        .collect();
    assert_eq!(statuses.len(), 4);
    let rereads = |frame: Value| {
        let source = BrocaSource::new(resolved(), FakeRoute::new());
        let mut changes = source.changes();
        changes.mark_unchanged();
        assert!(source.observe(frame));
        changes.has_changed().unwrap()
    };
    for name in &statuses {
        let frame = body(name);
        assert_eq!(frame["event"]["type"], "run_status", "{name}");
        assert!(rereads(frame), "{name}");
    }
    let other = |event: &str| json!({"kind": "display", "event": {"type": event}, "seq": 1});
    assert!(!rereads(other("display_reset")));
    assert!(!rereads(json!({"kind": "display", "seq": 2,
        "event": {"type": "text_delta", "mid": "m1", "delta": "Hi"}})));
}
