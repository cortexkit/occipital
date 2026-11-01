//! A Pi head's runner replies recorded from real Pi 1.4.0, `runner-pages-pi-v1`:
//! Thalamus's tag of that name (annotated, object `4afd77d50272…`, commit
//! `83e06a6da1ff…`), copied byte for byte into `tests/data`. Its README, also
//! Thalamus's, says what each file is; "the plan" there is the session's plan
//! from core, which sets the head's system text and the tools it may call.
//! Pi heads run on `thalamus-pi` with `harness: "pi"`.

use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SET: &str = "tests/data/runner-pages-pi-v1";
/// The set's `SHA256SUMS` as Thalamus published it.
const SUMS_SHA256: &str = "5350b1d9cb39e98e0669f243c346bda78c3194c70e1493c7abdee757549ad45e";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(SET)
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

fn pi_binding() -> Resolved {
    let mut binding = resolved();
    binding.reference.runner = PI_RUNNER_MODULE_ID.into();
    binding.harness = "pi".into();
    binding
}

#[test]
fn the_pi_pages_are_the_published_set() {
    let sums = std::fs::read(root().join("SHA256SUMS")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&sums)), SUMS_SHA256);
    let sums = String::from_utf8(sums).unwrap();
    assert_eq!(sums.lines().count(), 8, "the eight replies");
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// Occipital's rows for the recorded `session.read` are the runner's own
/// display rows for the same entries, apart from `sender`, which the runner
/// adds from its send record and Occipital doesn't read.
#[test]
fn pi_rows_are_the_runners_display_rows() {
    let source = BrocaSource::new(pi_binding(), FakeRoute::new());
    assert_eq!(source.harness(), "pi");
    let read = sample("01-session-read.json");
    let entries = read["messages"].as_array().unwrap();
    let rows = source.rows(entries, false, &[]).unwrap();
    let display = sample("05-session-history-display.json");
    let expected = display["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 9);
    assert_eq!(rows.len(), expected.len());
    for (row, want) in rows.iter().zip(expected) {
        assert_eq!(row.mid, want["mid"].as_str().unwrap());
        assert_eq!(row.ordinal, want["ordinal"].as_u64());
        let mut want = want["row"].clone();
        want.as_object_mut().unwrap().remove("sender");
        assert_eq!(row.display, want, "{}", row.mid);
    }
}

/// Pi's empty system entry at ordinal 0 is left out of pages, which an app would
/// draw as an empty prompt card; the page still reaches the start, and the
/// rows kept keep their ordinals. A system row with text stays.
#[tokio::test]
async fn a_pi_page_leaves_out_the_empty_system_entry() {
    let messages = sample("01-session-read.json")["messages"]
        .as_array()
        .unwrap()
        .clone();
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
    image["parts"] = json!([{"type": "image", "part": "0"}]);
    assert!(!is_empty_system_row(&image));
}

/// Feed a recorded `session.subscribe` stream to a Pi source whose runner
/// declares `groups`, and return the display frames the apps then read:
/// each frame's kind, `messageID` and words.
async fn display_of(file: &str, groups: &[&str]) -> (Vec<bool>, Vec<(String, String, String)>) {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(groups));
    *route.entries.lock().unwrap() = sample("01-session-read.json")["messages"]
        .as_array()
        .unwrap()
        .clone();
    let source = BrocaSource::new(pi_binding(), route.clone());
    let mut lease = source.acquire().await.unwrap();
    let asked = route.live_ids.lock().unwrap().clone();
    let hub = Arc::new(DisplayHub::default());
    for frame in sample(file).as_array().unwrap() {
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
/// word to its end, and Pi's stored `mid` never appears on them.
#[tokio::test]
async fn a_pi_reply_streams_under_its_live_id() {
    let (asked, frames) =
        display_of("07-subscribe-live-ids.json", &["live_events", "live_ids"]).await;
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
}

/// Without `live_ids`, Pi's runner holds the words until it stores the reply
/// and then sends them under the `mid`, after a `message_started` that names
/// both; Occipital shows them under the `mid`.
#[tokio::test]
async fn without_live_ids_a_pi_reply_shows_under_its_mid() {
    let (asked, frames) = display_of("08-subscribe-plain.json", &["live_events"]).await;
    assert_eq!(asked, [false]);
    assert!(
        frames.contains(&frame("display", "568d566a", "PROBE_DONE")),
        "{frames:?}"
    );
    assert!(!frames.iter().any(|(_, id, _)| id.starts_with("live-")));
}
