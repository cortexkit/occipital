//! Broca's samples for a module message that carries another agent's words
//! with the agent's display name (runner contract 0.8.34: `notice: true`
//! together with `speaks_for {agent_id, name}`), recorded from its real
//! serializers. Copied unchanged into `tests/data/broca-speaks-for-name-1109/`
//! from the set Broca published for its readers, with its README; Occipital
//! decodes only the `session.read`, and the other bodies are kept so the
//! checksums verify whole.
//!
//! Occipital takes a row's marks from the read entry as JSON, so the new
//! `name` is no new shape for it: a message with both marks reads as the
//! agent's words, `speaksFor: {agentId}`, as it did before names existed.
//! The apps resolve an agent's name from its id, so the name isn't passed on.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-speaks-for-name-1109")
}

#[test]
fn the_speaks_for_name_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 7);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The newest page: core's two messages carrying agent-a's words read as
/// agent-a's, with no notice or sender label beside it, and the reply as the
/// agent's own.
#[tokio::test]
async fn a_named_speaker_reads_as_that_agents_words() {
    let read: Value =
        serde_json::from_slice(&std::fs::read(root().join("named-marks-read.json")).unwrap())
            .unwrap();
    let page: RawPage = serde_json::from_value(runner_result(read).unwrap()).unwrap();
    assert_eq!(page.messages[1]["speaks_for"]["name"], "Ada");
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = page.messages;
    let newest = BrocaSource::new(resolved(), route)
        .newest(20)
        .await
        .unwrap();
    let marks: Vec<(&str, Value, Value, Value)> = newest
        .rows
        .iter()
        .map(|row| {
            (
                row.mid.as_str(),
                row.display["speaksFor"].clone(),
                row.display["notice"].clone(),
                row.display["sentBy"].clone(),
            )
        })
        .collect();
    let agent = json!({"agentId": "agent-a"});
    assert_eq!(
        marks,
        [
            ("m0", agent.clone(), Value::Null, Value::Null),
            ("m1", agent, Value::Null, Value::Null),
            ("m2", Value::Null, Value::Null, Value::Null),
        ]
    );
}
