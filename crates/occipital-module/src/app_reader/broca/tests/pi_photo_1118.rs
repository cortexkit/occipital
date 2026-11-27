//! A photo in a Pi chat, recorded from real Pi 1.5.2 by Thalamus
//! (`tests/data/pi-photo-display-v2/`, copied unchanged). A Pi session now
//! declares `media_refs`, so Occipital reads it with `media: "media_refs"` and
//! the photo reaches the apps as a file part they fetch with `session.media`.
//! Before, Pi declared no such form, every read was the default one, and the
//! person's photo showed as the marker text that read serves in its place.

use super::served_samples::{decoded, Served};
use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SESSION: &str = "6180c7c34f692799";
const PHOTO: &str = "497790947d4666760ce38f3c00e852c71fdb66cae849bae8e9ede352719e1581";

fn samples() -> PathBuf {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/pi-photo-display-v2");
    let sums = std::fs::read(data.join("SHA256SUMS")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&sums)),
        "f20a92a14e8b6055b27e6cd7539ee4a5e10cc58a361ab34d4822d94986ea1912"
    );
    for line in String::from_utf8(sums).unwrap().lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(data.join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
    data
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(samples().join(name)).unwrap()).unwrap()
}

/// Entorhinal's record of the one Pi chat.
struct OneChat;

#[async_trait]
impl Entorhinal for OneChat {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        assert_eq!(params["session"], SESSION, "{method}");
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref",
            "incarnation":"entorhinal-test", "ended_at_ms":null, "runner":PI_RUNNER_MODULE_ID,
            "session":SESSION, "harness":"pi", "project_root": crate::test_paths::absolute("/workspace/head"),
            "network":"unknown", "network_history":[]}}),
        )
    }
}

struct OneRoute(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for OneRoute {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

/// The Pi chat as the runner recorded it: its own `role.describe`, the photo
/// message as `read` serves it, and the photo's bytes behind `session.media_read`.
fn pi_chat(read: &str) -> (Arc<super::super::super::AppReader>, Arc<FakeRoute>) {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = sample(read)["messages"].as_array().unwrap().clone();
    *route.run_state.lock().unwrap() = Value::Null;
    *route.describe.lock().unwrap() = Some(sample("role-describe.json"));
    let media = sample("media-read.json");
    let bytes = crate::base64::decode(media["data"].as_str().unwrap()).unwrap();
    assert_eq!(route.hold_media("image/png", &bytes), PHOTO);
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(OneChat))),
            Arc::new(OneRoute(route.clone())),
        ),
    )));
    (app, route)
}

/// The person's message on the newest page the apps get.
async fn photo_message(served: &mut Served) -> Value {
    let request = json!({"method": super::super::super::TRANSCRIPT_PAGE,
        "params": {"session": SESSION, "harness": "pi"}});
    let page = decoded(&served.read(1, &request).await)["result"].clone();
    let rows = page["messages"].as_array().unwrap();
    rows.iter()
        .find(|row| row["role"] == "user")
        .unwrap_or_else(|| panic!("{page}"))
        .clone()
}

/// With Pi's `media_refs` read, the person's photo is a file part beside their
/// words, with nothing else in the bubble's text, and its bytes come through
/// `session.media`.
#[tokio::test]
async fn a_pi_photo_reaches_the_apps_as_a_file() {
    let describe = sample("role-describe.json");
    let declares = |group: &str| {
        describe["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|declared| declared == group)
    };
    assert!(declares("media_refs") && !declares("tool_media_refs"));

    let (app, route) = pi_chat("read-media-refs.json");
    // Served only to read the replies; this test keeps no sample set.
    let mut served = Served::new("pi-photo-display-v2", app);
    let message = photo_message(&mut served).await;
    assert_eq!(message["text"], "Look at this photo.", "{message}");
    let files: Vec<&Value> = message["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "file")
        .collect();
    assert_eq!(files.len(), 1, "{message}");
    assert_eq!(files[0]["sha256"], PHOTO, "{message}");
    assert_eq!(files[0]["mime"], "image/png", "{message}");
    assert!(
        !message.to_string().contains("media not shown"),
        "{message}"
    );

    let reads: Vec<Value> = route
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call["method"] == "session.read")
        .cloned()
        .collect();
    assert!(!reads.is_empty());
    for read in &reads {
        assert_eq!(read["params"]["media"], "media_refs", "{read}");
        assert_eq!(read["params"].get("tool_media"), None, "{read}");
    }

    let request = json!({"method": crate::media::OPERATION,
        "params": {"session": SESSION, "harness": "pi", "sha256": PHOTO, "size": "full"}});
    let media = decoded(&served.read(2, &request).await)["result"].clone();
    assert_eq!(media["mime"], "image/png", "{media}");
    assert_eq!(media["data"], sample("media-read.json")["data"], "{media}");
}

/// The default read Pi served before is what put the marker in the bubble:
/// it holds the photo as text, so the row has no file to show.
#[tokio::test]
async fn the_default_read_holds_the_photo_as_marker_text() {
    let read = sample("read.json");
    let content = &read["messages"][0]["message"]["message"]["content"];
    assert!(content[1]["text"]
        .as_str()
        .unwrap()
        .starts_with("[media not shown: kind=image"));
}
