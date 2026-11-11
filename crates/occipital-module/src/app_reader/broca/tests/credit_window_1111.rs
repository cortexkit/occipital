//! Broca reporting a subscription spending wall as `limit_window: "credit"`
//! (routing 0.7.10), recorded from its real serializers on a synthetic
//! session: one before/after pair per surface the error object travels on
//! (`run.status`, `run.result`, `session.read`'s `lineage_state`, and a
//! `session.subscribe` error unit, replayed and live). Copied unchanged into
//! `tests/data/broca-credit-window-1111/` from the set Broca published for its
//! readers (its `SHA256SUMS` hashes to `86de2287d0c9ad54…`).
//!
//! The only change is a new value of an existing string member, so the risk is
//! a reader that checks it against a closed list. This reader never reads the
//! member: a chat's last run error reaches the apps from `session.head`'s
//! `last_run_error` (class and timing only), which this change leaves alone.
//! These tests run each after body through every path the reader takes over
//! the same surface and check the outcome is the before body's.

use super::*;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-credit-window-1111")
}

const PAIRS: &str = "xai-credit-caller-serializer-pairs.jsonl";

/// Each surface's before and after body, parsed.
fn pairs() -> Vec<(String, Value, Value)> {
    std::fs::read_to_string(root().join(PAIRS))
        .unwrap()
        .lines()
        .map(|line| {
            let pair: Value = serde_json::from_str(line).unwrap();
            let side = |name: &str| serde_json::from_str(pair[name].as_str().unwrap()).unwrap();
            (
                pair["surface"].as_str().unwrap().to_owned(),
                side("before"),
                side("after"),
            )
        })
        .collect()
}

#[test]
fn the_credit_window_samples_keep_their_checksum() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    let (expected, name) = sums.trim_end().split_once("  ").unwrap();
    assert_eq!(name, PAIRS);
    let bytes = std::fs::read(root().join(name)).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected);
}

/// Every surface reads the same before and after, by the path the reader
/// takes over it:
/// - `run.status`: Stop reads only `state` (`send/stop.rs`).
/// - `session.read`: the page decodes to the same messages, history counter
///   and log; `lineage_state` isn't part of what the reader keeps.
/// - `session.subscribe`: the frame is or isn't a live event, normalizes, and
///   maps to a display input, the same each time.
/// - `run.result`: the reader never calls it; the after body still carries its
///   error under the same class and code.
#[test]
fn a_credit_window_changes_nothing_this_reader_reads() {
    let pairs = pairs();
    let surfaces: Vec<&str> = pairs
        .iter()
        .map(|(surface, _, _)| surface.as_str())
        .collect();
    assert_eq!(
        surfaces,
        [
            "run.status",
            "run.result",
            "session.read",
            "session.subscribe/replay",
            "session.subscribe/live"
        ]
    );
    for (surface, before, after) in &pairs {
        let error = |body: &Value| match surface.as_str() {
            "session.read" => body["lineage_state"]["error"].clone(),
            "session.subscribe/replay" | "session.subscribe/live" => body["unit"]["error"].clone(),
            _ => body["error"].clone(),
        };
        // The samples differ by exactly the new member.
        let mut stripped = after.clone();
        let target = match surface.as_str() {
            "session.read" => &mut stripped["lineage_state"]["error"],
            "session.subscribe/replay" | "session.subscribe/live" => &mut stripped["unit"]["error"],
            _ => &mut stripped["error"],
        };
        assert_eq!(
            target.as_object_mut().unwrap().remove("limit_window"),
            Some(json!("credit")),
            "{surface}"
        );
        assert_eq!(&stripped, before, "{surface}");
        assert_eq!(
            error(after)["provider_code"],
            "account_limit_exhausted",
            "{surface}"
        );
        match surface.as_str() {
            "run.status" | "run.result" => assert_eq!(after["state"], before["state"], "{surface}"),
            "session.read" => {
                let decode = |body: &Value| {
                    let page: RawPage = serde_json::from_value(body.clone())
                        .unwrap_or_else(|e| panic!("{surface}: {e}"));
                    (page.messages, page.history_version, page.incarnation)
                };
                assert_eq!(decode(after), decode(before));
            }
            _ => {
                assert_eq!(
                    carries_live_event(after),
                    carries_live_event(before),
                    "{surface}"
                );
                let mapped = |body: &Value| {
                    normalize(body.clone())
                        .map(|event| (display_events::map(&event), event["event"].clone()))
                };
                assert_eq!(mapped(after), mapped(before), "{surface}");
            }
        }
    }
}
