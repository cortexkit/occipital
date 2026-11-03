//! A Pi head's runner replies recorded from real Pi 1.4.0, in two sets copied
//! byte for byte from Thalamus's tags of the same names into `tests/data`:
//! - `runner-pages-pi-v1` (annotated, object `4afd77d50272…`, commit
//!   `83e06a6da1ff…`);
//! - `runner-pages-pi-v2` (object `09f85fce8393…`, commit `20070c8ac7e6…`),
//!   recorded after two runner fixes: the run's start comes before the reply,
//!   and a `live_id` reaches only a subscriber that asked for one.
//!
//! Each README, also Thalamus's, says what each file is; "the plan" there is the
//! session's plan from core, which sets the head's system text and the tools it
//! may call. Pi heads run on `thalamus-pi` with `harness: "pi"`. A message's
//! `mid` is the runner's id for it once stored, which the apps see as
//! `messageID`; a `live_id` stands in for it while Pi hasn't stored the message.

use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const V1: &str = "runner-pages-pi-v1";
const V2: &str = "runner-pages-pi-v2";
/// Each set's `SHA256SUMS` as Thalamus published it.
const SETS: [(&str, &str); 2] = [
    (
        V1,
        "5350b1d9cb39e98e0669f243c346bda78c3194c70e1493c7abdee757549ad45e",
    ),
    (
        V2,
        "83880baebd092cda1a3c08a087ed55bec19626d82e0e749b7c9f92b352afcf75",
    ),
];

fn root(set: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(set)
}

fn sample(set: &str, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root(set).join(name)).unwrap()).unwrap()
}

fn messages(set: &str) -> Vec<Value> {
    sample(set, "01-session-read.json")["messages"]
        .as_array()
        .unwrap()
        .clone()
}

fn pi_binding() -> Resolved {
    let mut binding = resolved();
    binding.reference.runner = PI_RUNNER_MODULE_ID.into();
    binding.harness = "pi".into();
    binding
}

#[test]
fn the_pi_pages_are_the_published_sets() {
    for (set, sums_sha256) in SETS {
        let sums = std::fs::read(root(set).join("SHA256SUMS")).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&sums)), sums_sha256, "{set}");
        let sums = String::from_utf8(sums).unwrap();
        assert_eq!(sums.lines().count(), 8, "{set}: the eight replies");
        for line in sums.lines() {
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

/// Occipital's rows for the recorded `session.read` follow the runner's own
/// display rows for the same entries: the same messages, roles and text, and
/// each tool call is one app tool part keyed by the runner's part key, with its
/// result folded in.
#[test]
fn pi_rows_follow_the_runners_display_rows() {
    let source = BrocaSource::new(pi_binding(), FakeRoute::new());
    assert_eq!(source.harness(), "pi");
    for (set, _) in SETS {
        let rows = source.rows(&messages(set), false, &[]).unwrap();
        let display = sample(set, "05-session-history-display.json");
        let expected = display["messages"].as_array().unwrap();
        assert_eq!(rows.len(), 9, "{set}");
        assert_eq!(rows.len(), expected.len(), "{set}");
        for (row, want) in rows.iter().zip(expected) {
            assert_eq!(row.mid, want["mid"].as_str().unwrap(), "{set}");
            assert_eq!(row.ordinal, want["ordinal"].as_u64(), "{set}");
            let want = &want["row"];
            let role = match want["role"].as_str().unwrap() {
                "toolResult" => "tool",
                role => role,
            };
            assert_eq!(row.display["role"], role, "{set}: {}", row.mid);
            assert_eq!(row.display["text"], want["text"], "{set}: {}", row.mid);
            for call in want["parts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|part| part["type"] == "tool_call")
            {
                let tool = row.display["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|part| part["callId"] == call["tool_call_id"])
                    .unwrap();
                assert_eq!(tool["type"], "tool");
                assert_eq!(tool["tool"], call["name"]);
                assert_eq!(tool["partKey"], call["part"]);
                assert!(tool["outputBytes"].is_u64(), "{set}: {tool}");
            }
        }
    }
}

/// Pi's empty system entry at ordinal 0 is left out of pages, which an app would
/// draw as an empty prompt card; the page still reaches the start, and the
/// rows kept keep their ordinals. A system row with text stays.
#[tokio::test]
async fn a_pi_page_leaves_out_the_empty_system_entry() {
    let messages = messages(V1);
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = messages[..2].to_vec();
    let source = BrocaSource::new(pi_binding(), route);
    let page = source.newest(20).await.unwrap();
    let ordinals: Vec<_> = page.rows.iter().map(|row| row.ordinal).collect();
    assert_eq!(ordinals, [Some(1)]);
    assert!(page.older.is_none(), "the page still reaches the start");

    let rows = source.rows(&messages[..1], false, &[]).unwrap();
    assert!(is_empty_system_row(&rows[0].display));
    let mut prompt = rows[0].display.clone();
    prompt["text"] = json!("You are a worker.");
    assert!(!is_empty_system_row(&prompt));
    let mut image = rows[0].display.clone();
    image["parts"] = json!([{"type": "file", "partKey": "0", "partIndex": 0}]);
    assert!(!is_empty_system_row(&image));
}

/// Feed a set's recorded `session.subscribe` stream to a Pi source whose runner
/// declares `groups`, and return whether it asked for `live_id`s and the
/// display frames the apps then read: each frame's kind, `messageID` and words.
async fn display_of(
    set: &str,
    file: &str,
    groups: &[&str],
) -> (Vec<bool>, Vec<(String, String, String)>) {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(groups));
    *route.entries.lock().unwrap() = messages(set);
    let source = BrocaSource::new(pi_binding(), route.clone());
    let mut lease = source.acquire().await.unwrap();
    let asked = route.live_ids.lock().unwrap().clone();
    let hub = Arc::new(DisplayHub::default());
    for frame in sample(set, file).as_array().unwrap() {
        route.emit(frame.clone()).await;
    }
    while let Ok(Ok(input)) =
        tokio::time::timeout(Duration::from_millis(500), lease.events.recv()).await
    {
        hub.ingest("ses_pi", input);
    }
    let read =
        serde_json::to_value(hub.projection("ses_pi").unwrap().display_read(Some(0))).unwrap();
    let frames = read["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sequenced| {
            let frame = &sequenced["frame"];
            let text = |name: &str| frame[name].as_str().unwrap_or_default().to_owned();
            (text("kind"), text("messageID"), text("chunk"))
        })
        .collect();
    (asked, frames)
}

fn frame(kind: &str, message: &str, words: &str) -> (String, String, String) {
    (kind.into(), message.into(), words.into())
}

/// With `live_ids`, each reply's words show under its `live_id` from the first
/// word to its end, and Pi's stored `mid` never appears on them. In v1 the
/// stream opens on the tail of an earlier run; in v2 the reply arrives in two
/// pieces, joined into one frame by the display's batching.
#[tokio::test]
async fn a_pi_reply_streams_under_its_live_id() {
    let groups = ["live_events", "live_ids"];
    let (asked, frames) = display_of(V1, "07-subscribe-live-ids.json", &groups).await;
    assert_eq!(asked, [true]);
    let first = "live-e42e5ae1-2880-4296-92a8-fe95cd2b8e7a";
    let second = "live-d97b5d76-4f9e-4931-a7f9-f6244b42727e";
    assert_eq!(
        frames,
        [
            frame("display_gap", "", ""),
            frame("display", first, "PROBE_DONE"),
            frame("display_end", "", ""),
            frame("display_end", "", ""),
            frame("display", second, "PROBE_DONE"),
            frame("display_end", "", ""),
            frame("display_end", "", ""),
        ]
    );

    let (asked, frames) = display_of(V2, "07-subscribe-live-ids.json", &groups).await;
    assert_eq!(asked, [true]);
    assert_eq!(
        frames,
        [
            frame("display_gap", "", ""),
            frame(
                "display",
                "live-e26f890f-3373-409d-a1c4-e6f49bdd293c",
                "PROBE_DONE"
            ),
            frame("display_end", "", ""),
            frame("display_end", "", ""),
        ]
    );
}

/// Without `live_ids`, Pi's runner holds the words until it stores the reply
/// and then sends them under the `mid`; Occipital shows them under the `mid`.
/// In v1 the runner also named the `live_id` on `message_started`, which v2's
/// runner no longer sends to a subscriber that didn't ask.
#[tokio::test]
async fn without_live_ids_a_pi_reply_shows_under_its_mid() {
    for (set, mid) in [(V1, "568d566a"), (V2, "351aac36")] {
        let (asked, frames) = display_of(set, "08-subscribe-plain.json", &["live_events"]).await;
        assert_eq!(asked, [false], "{set}");
        assert!(
            frames.contains(&frame("display", mid, "PROBE_DONE")),
            "{set}: {frames:?}"
        );
        assert!(
            !frames.iter().any(|(_, id, _)| id.starts_with("live-")),
            "{set}"
        );
    }
    let (_, frames) = display_of(V2, "08-subscribe-plain.json", &["live_events"]).await;
    assert_eq!(
        frames,
        [
            frame("display_gap", "", ""),
            frame("display", "351aac36", "PROBE_DONE"),
            frame("display_end", "", ""),
            frame("display_end", "", ""),
        ]
    );
}
