//! The sample set the apps pin for a message streamed under a `live_id`,
//! `phone-reader-live-ids-v1`; its README says what each file shows.
//!
//! A runner whose host names a message only when storing it (`live_ids`)
//! streams the message's words under a runner-minted `live_id`, then names it
//! with `message_started {mid, live_id}` and ends it with `message_finished`.
//! The apps see the `live_id` as `messageID` from the first word to the
//! message's `display_end`, including words the runner sends under the `mid`
//! after storing it, so an app hands the words over to the stored copy under
//! one key. This test serves every request again and fails if a byte changes;
//! with `OCCIPITAL_WRITE_SAMPLES` set to a folder it writes the set there.

use super::test_support::FakeSession;
use serde_json::{json, Value};

const NOW: u64 = 1_793_500_200_000;

fn control(event: Value) -> Value {
    json!({"kind":"control", "event":event})
}

fn text(key: &str, id: &str, delta: &str) -> Value {
    json!({"kind":"display", "event":{"type":"text_delta", key:id, "delta":delta}})
}

#[test]
fn the_live_id_samples_are_what_the_reader_serves() {
    let session = FakeSession::new("ses_pi", 64);
    let mut files: Vec<(String, Value)> = Vec::new();
    let mut keep = |name: &str, (request, reply): (Value, Value)| {
        files.push((format!("{name}.request.json"), request));
        files.push((format!("{name}.json"), reply));
    };

    for event in [
        control(json!({"type":"run_started", "run_id":"r1"})),
        text("live_id", "live-7f3a", "Reading the notes "),
        text("live_id", "live-7f3a", "first."),
    ] {
        session.ingest(event);
    }
    session.flush();
    keep("01-unstored-message-cold", session.read(None, false, NOW));
    let streamed = session.read(Some(0), false, NOW);
    let seq = streamed.1["result"]["seq"].as_u64().unwrap();
    keep("02-unstored-message-frames", streamed);

    for event in [
        control(json!({"type":"message_started", "mid":"m41", "live_id":"live-7f3a"})),
        // After storing it, the runner names the message's words by its `mid`.
        text("mid", "m41", " Done."),
        control(json!({"type":"message_finished", "mid":"m41"})),
    ] {
        session.ingest(event);
    }
    session.flush();
    let stored = session.read(Some(seq), false, NOW);
    let seq = stored.1["result"]["seq"].as_u64().unwrap();
    keep("03-stored-then-ended", stored);

    for event in [
        text("mid", "m42", "Named at creation."),
        control(json!({"type":"run_finished", "run_id":"r1"})),
    ] {
        session.ingest(event);
    }
    session.flush();
    keep(
        "04-next-message-and-run-end",
        session.read(Some(seq), false, NOW),
    );

    let message_ids: Vec<Value> = files
        .iter()
        .filter(|(name, _)| !name.ends_with(".request.json"))
        .flat_map(|(_, reply)| {
            let result = &reply["result"];
            let frames = result["frames"].as_array().cloned().unwrap_or_default();
            frames
                .into_iter()
                .map(|sequenced| sequenced["frame"].clone())
                .chain(result.get("frame").cloned())
                .filter_map(|frame| frame.get("messageID").cloned())
                .collect::<Vec<_>>()
        })
        .collect();
    // The stored message's words keep the `live_id` to its end; only the next
    // message, named at creation, shows a `mid`.
    assert!(
        message_ids
            .iter()
            .all(|id| id == "live-7f3a" || id == "m42"),
        "{message_ids:?}"
    );
    assert!(!message_ids.iter().any(|id| id == "m41"));
    super::keep_sample_set("phone-reader-live-ids-v1", &files);
}
