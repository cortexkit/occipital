//! A text file a person shared into an OpenCode 2 chat, recorded by Thalamus
//! on a real OpenCode 2.2.3 server once its runner takes `text/plain` uploads
//! into the prompt. Copied byte for byte into `tests/data/thalamus-text-upload-1108/`
//! (its `SHA256SUMS` hashes to `1a669711f171e602…`). The person's message is
//! "Here is a note." with a 43-byte note attached. The runner renders the note's
//! numbered text only into the model's request, so no read puts it in the
//! message's text, and the apps show a file, never the file's text as the
//! person's words.

use super::served_samples::{decoded, Served};
use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/thalamus-text-upload-1108")
}

fn sample(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}

const NOTE_SHA256: &str = "0fe96b34b009c25a56ff43b0d3672ef89d5111edf0f7f148135dc3dabcadab14";

#[test]
fn the_text_upload_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 4);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

/// The person's row, as the reader builds it from a recorded read.
fn person_row(name: &str) -> Value {
    let mut binding = resolved();
    binding.reference.runner = THALAMUS_RUNNER_MODULE_ID.into();
    binding.harness = "opencode2".into();
    let read = sample(name);
    let rows = BrocaSource::new(binding, FakeRoute::new())
        .rows(read["messages"].as_array().unwrap(), false, &[])
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
    rows.into_iter()
        .find(|row| row.mid == "msg_1bb9ed46b0016fyFUcWb0OqPJ6")
        .unwrap_or_else(|| panic!("{name}"))
        .display
}

/// In a read asking for `media: "media_refs"`, as Occipital's page reads do
/// from a runner declaring that group, the note is a file part
/// with its type, size and digest, which `session.media` serves whole, and the
/// row's text is only what the person typed. The default read and raw history
/// show the runner's marker after the typed text instead, as for a photo, but
/// never the note's own text.
#[test]
fn a_shared_text_file_is_a_file_part_not_the_persons_words() {
    let row = person_row("02-session-read-media-refs-display-facts.json");
    assert_eq!(row["text"], "Here is a note.", "{row}");
    assert_eq!(row["clientMessageId"], "occ-app-1", "{row}");
    let files: Vec<&Value> = row["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "file")
        .collect();
    assert_eq!(files.len(), 1, "{row}");
    assert_eq!(files[0]["mime"], "text/plain", "{row}");
    assert_eq!(files[0]["byteCount"], 43, "{row}");
    assert_eq!(files[0]["sha256"], NOTE_SHA256, "{row}");
    let note = crate::base64::decode(
        sample("04-session-media-read.json")["data"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let note = String::from_utf8(note).unwrap();
    for name in ["01-session-read-inline.json", "03-session-history-raw.json"] {
        let row = person_row(name);
        let text = row["text"].as_str().unwrap();
        assert!(
            text.starts_with("Here is a note.[media not shown: kind=file"),
            "{name}: {row}"
        );
        assert!(!text.contains(note.trim()), "{name}: {row}");
    }
}

/// Routes to the recorded session, an OpenCode 2 one.
struct OpenCode2(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for OpenCode2 {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "opencode2");
        Ok(self.0.clone())
    }
}

/// `phone-reader-text-file-v1`, the sample set the apps pin: the recorded
/// chat's newest page, the note read whole through `session.media`, and the
/// thumbnail read refused because a text file is not an image.
/// `served_samples` says how the set is checked and rewritten.
#[tokio::test]
async fn the_text_file_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&[
        "attachments",
        "transcript_reads",
        "media_refs",
    ]));
    let read = sample("02-session-read-media-refs-display-facts.json");
    *route.entries.lock().unwrap() = read["messages"].as_array().unwrap().clone();
    let note = crate::base64::decode(
        sample("04-session-media-read.json")["data"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(route.hold_media("text/plain", &note), NOTE_SHA256);
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                THALAMUS_RUNNER_MODULE_ID,
            ))))),
            Arc::new(OpenCode2(route.clone())),
        ),
    )));
    let mut served = Served::new("phone-reader-text-file-v1", app);
    let session = json!({"session": "ses_head", "harness": "opencode2"});

    let mut page_request =
        json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": session.clone()});
    page_request["params"]["limit"] = json!(50);
    let page = served.read(1, &page_request).await;
    let person = decoded(&page)["result"]["messages"][2].clone();
    assert_eq!(person["text"], "Here is a note.", "{person}");
    let files: Vec<&Value> = person["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["type"] == "file")
        .collect();
    assert_eq!(files.len(), 1, "{person}");
    assert_eq!(
        (
            &files[0]["mime"],
            &files[0]["byteCount"],
            &files[0]["sha256"]
        ),
        (&json!("text/plain"), &json!(43), &json!(NOTE_SHA256))
    );
    served.keep("01-newest-page", &page_request, page);

    let media = |size: &str| {
        let mut request = json!({"method": super::super::super::MEDIA, "params": session.clone()});
        request["params"]["sha256"] = json!(NOTE_SHA256);
        request["params"]["size"] = json!(size);
        request
    };
    let full_request = media("full");
    let full = served.read(2, &full_request).await;
    let result = decoded(&full)["result"].clone();
    assert_eq!(
        crate::base64::decode(result["data"].as_str().unwrap()).unwrap(),
        note
    );
    assert_eq!(
        (&result["mime"], &result["size"], &result["offset"]),
        (&json!("text/plain"), &json!(43), &json!(0))
    );
    served.keep("02-full", &full_request, full);

    let thumb_request = media("thumb");
    served.send(3, &thumb_request).await;
    let (_, refused) = served.next(3, Duration::from_secs(5)).await.unwrap();
    assert_eq!(decoded(&refused)["code"], "media_not_image");
    served.keep("03-refusal-media_not_image", &thumb_request, refused);
    served.check();
}

/// The runner's media read returns the note's exact bytes under the digest the
/// row names.
#[test]
fn the_media_read_returns_the_note_itself() {
    let read = sample("04-session-media-read.json");
    assert_eq!(read["mime"], "text/plain");
    assert_eq!(read["size"], 43);
    let bytes = crate::base64::decode(read["data"].as_str().unwrap()).unwrap();
    assert_eq!(bytes.len(), 43);
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), NOTE_SHA256);
}
