//! The attached-file samples the apps pin, `phone-reader-files-v2`. Its README
//! says what each file shows. The chat is a test OpenCode 2 session on a runner
//! that takes files. Its first two messages are the runner contract's own stored
//! bodies, from its `display-opencode-v2.json` (see
//! `occipital-app-reader/tests/display_profiles.rs`): a message recorded from
//! OpenCode 2.2.3 with text and a one-pixel PNG, and the contract's message
//! holding that image alone. The other two are written here in the same shape: a
//! photo taken sideways, whose EXIF says to turn it a quarter, and a PDF.
//! `served_samples` says how the set is checked and rewritten.
//!
//! `phone-reader-files-v1` is the same first page before image parts carried
//! their digest and size. It stays published unchanged, checked by its
//! checksums only.

use super::served_samples::{decoded, Served};
use super::*;

/// Routes to the test session, an OpenCode 2 one.
struct OpenCode2(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for OpenCode2 {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "opencode2");
        Ok(self.0.clone())
    }
}

/// The contract's stored body for its case `name`.
fn body(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../occipital-app-reader/tests/fixtures/llm-runner-0.8.0/display-opencode-v2.json");
    let set: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    set["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap()["body"]
        .clone()
}

/// An OpenCode 2 user message with `text` and one inline file, in the shape the
/// contract's recorded body has.
fn with_file(id: &str, text: &str, name: &str, mime: &str, bytes: &[u8]) -> Value {
    json!({"type": "user", "id": id, "text": text, "time": {"created": 1_793_365_570_000_u64},
        "files": [{"data": crate::base64::encode(bytes), "mime": mime,
            "source": {"type": "inline"}, "name": name}]})
}

/// An 8 by 4 pixel JPEG whose EXIF says to turn it a quarter clockwise, as a
/// phone held upright writes a photo: drawn, it is 4 wide and 8 tall.
fn sideways_photo() -> Vec<u8> {
    use image::{codecs::jpeg::JpegEncoder, DynamicImage, Rgb, RgbImage};
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode_image(&DynamicImage::ImageRgb8(RgbImage::from_fn(8, 4, |x, _| {
            Rgb([(x * 30) as u8, 90, 160])
        })))
        .unwrap();
    // Orientation 6 in a big-endian TIFF block, right after start-of-image.
    let mut exif =
        b"Exif\0\0MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0".to_vec();
    let mut out = jpeg[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(((exif.len() + 2) as u16).to_be_bytes());
    out.append(&mut exif);
    out.extend(&jpeg[2..]);
    out
}

/// The pixel in the contract's recorded message.
fn pixel() -> Vec<u8> {
    let data = body("2.2.3-seq-54")["files"][0]["data"].clone();
    crate::base64::decode(data.as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn the_attached_file_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.describe.lock().unwrap() = Some(describe_with(&["attachments"]));
    let photo = sideways_photo();
    let pdf = b"%PDF-1.7\n% notes the person attached\n".to_vec();
    *route.entries.lock().unwrap() = vec![
        json!({"mid": "msg_text_and_file", "ordinal": 1, "written_at": 1_793_365_553_098_u64,
            "message": body("2.2.3-seq-54")}),
        json!({"mid": "msg_file_alone", "ordinal": 2, "written_at": 1_793_365_560_000_u64,
            "message": body("synthetic-existing-image-ref")}),
        json!({"mid": "msg_photo", "ordinal": 3, "written_at": 1_793_365_570_000_u64,
            "message": with_file("msg_photo", "Taken holding the phone upright", "photo.jpg", "image/jpeg", &photo)}),
        json!({"mid": "msg_pdf", "ordinal": 4, "written_at": 1_793_365_580_000_u64,
            "message": with_file("msg_pdf", "And the notes", "notes.pdf", "application/pdf", &pdf)}),
    ];
    let pixel_sha = route.hold_media("image/png", &pixel());
    let photo_sha = route.hold_media("image/jpeg", &photo);
    let pdf_sha = route.hold_media("application/pdf", &pdf);
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                OPENCODE_RUNNER_MODULE_ID,
            ))))),
            Arc::new(OpenCode2(route.clone())),
        ),
    )));
    let mut served = Served::new("phone-reader-files-v2", app);
    let session = json!({"session": "ses_head", "harness": "opencode2"});
    let file = |message: &Value| -> Value {
        let files: Vec<&Value> = message["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "file")
            .collect();
        assert_eq!(files.len(), 1, "{message}");
        files[0].clone()
    };

    let mut page_request =
        json!({"method": super::super::super::TRANSCRIPT_PAGE, "params": session});
    page_request["params"]["limit"] = json!(50);
    let page = served.read(1, &page_request).await;
    let messages = decoded(&page)["result"]["messages"].clone();
    assert_eq!(
        file(&messages[0]),
        json!({"type": "file", "partKey": "1", "partIndex": 1, "mime": "image/png",
            "filename": "pixel.png", "byteCount": 69, "sha256": pixel_sha,
            "width": 1, "height": 1})
    );
    assert_eq!(file(&messages[1])["sha256"], pixel_sha.as_str());
    // Drawn upright: the EXIF turns the 8 by 4 picture a quarter.
    assert_eq!(
        (
            file(&messages[2])["width"].clone(),
            file(&messages[2])["height"].clone()
        ),
        (json!(4), json!(8))
    );
    assert_eq!(messages[2]["files"][0]["height"], 8);
    // A PDF has a digest but no size.
    let notes = file(&messages[3]);
    assert_eq!(
        (notes["sha256"].as_str(), notes.get("width")),
        (Some(pdf_sha.as_str()), None)
    );
    served.keep("01-files-newest-page", &page_request, page);

    // The image sizes are kept: a second page reads no image again.
    let reads = || {
        route
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call["method"] == "session.media_read")
            .count()
    };
    let before = reads();
    served.read(2, &page_request).await;
    assert_eq!(reads(), before);

    let media = |corr_session: &Value, sha256: &str, size: &str| {
        let mut request =
            json!({"method": super::super::super::MEDIA, "params": corr_session.clone()});
        request["params"]["sha256"] = json!(sha256);
        request["params"]["size"] = json!(size);
        request
    };
    let thumb_request = media(&session, &photo_sha, "thumb");
    let thumb = served.read(3, &thumb_request).await;
    let result = decoded(&thumb)["result"].clone();
    assert_eq!(
        (&result["mime"], &result["width"], &result["height"]),
        (&json!("image/jpeg"), &json!(4), &json!(8))
    );
    served.keep("02-thumb", &thumb_request, thumb);

    let full_request = media(&session, &pixel_sha, "full");
    let full = served.read(4, &full_request).await;
    let result = decoded(&full)["result"].clone();
    assert_eq!(
        crate::base64::decode(result["data"].as_str().unwrap()).unwrap(),
        pixel()
    );
    assert_eq!(
        (&result["size"], &result["offset"]),
        (&json!(69), &json!(0))
    );
    served.keep("03-full", &full_request, full);

    for (corr, name, sha256, code) in [
        (
            5,
            "refusal-media_not_image",
            pdf_sha.clone(),
            "media_not_image",
        ),
        (6, "refusal-unknown_media", "0".repeat(64), "unknown_media"),
    ] {
        let request = media(&session, &sha256, "thumb");
        served.send(corr, &request).await;
        let (_, reply) = served.next(corr, Duration::from_secs(5)).await.unwrap();
        assert_eq!(decoded(&reply)["code"], code, "{name}");
        served.keep(name, &request, reply);
    }
    served.check();
}

/// A runner that takes no files serves no media: its pages carry each image's
/// digest but no size, and nothing asks it for bytes.
#[tokio::test]
async fn a_runner_without_files_is_never_asked_for_media() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = vec![json!({"mid": "msg_text_and_file", "ordinal": 1,
        "written_at": 1_793_365_553_098_u64, "message": body("2.2.3-seq-54")})];
    route.hold_media("image/png", &pixel());
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                OPENCODE_RUNNER_MODULE_ID,
            ))))),
            Arc::new(OpenCode2(route.clone())),
        ),
    )));
    let page = app
        .read(
            super::super::super::TRANSCRIPT_PAGE,
            json!({"session": "ses_head", "harness": "opencode2", "limit": 50}),
        )
        .await
        .unwrap();
    let part = &page["messages"][0]["parts"][1];
    assert!(
        part["sha256"].is_string() && part.get("width").is_none(),
        "{part}"
    );
    assert!(!route
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| call["method"] == "session.media_read"));
    let refusal = app
        .read(
            super::super::super::MEDIA,
            json!({"session": "ses_head", "harness": "opencode2",
                "sha256": part["sha256"], "size": "thumb"}),
        )
        .await
        .unwrap_err();
    assert_eq!(refusal.code, "unsupported");
}

/// A runner declaring `media_refs` is asked for media references on every
/// `session.read`, newest, older and exact pages alike, so its photos come as
/// file parts rather than text markers. A runner that takes files without
/// declaring it, or one whose description is unknown, is never sent the value,
/// which it would refuse. `session.history`, which finds a message by id, asks
/// for no media form.
#[tokio::test]
async fn only_a_runner_declaring_media_refs_is_asked_for_media_references() {
    for (groups, asked) in [
        (
            Some(vec!["attachments", "transcript_reads", "media_refs"]),
            true,
        ),
        (Some(vec!["attachments"]), false),
        (None, false),
    ] {
        let route = FakeRoute::new();
        *route.describe.lock().unwrap() = groups.as_deref().map(describe_with);
        *route.entries.lock().unwrap() = vec![
            json!({"mid": "msg_file_alone", "ordinal": 1, "written_at": 1_793_365_560_000_u64,
                "message": body("synthetic-existing-image-ref")}),
            json!({"mid": "msg_reply", "ordinal": 2, "written_at": 1_793_365_561_000_u64,
                "message": {"id": "msg_reply", "type": "assistant",
                    "time": {"created": 1_793_365_560_500_u64},
                    "content": [{"type": "text", "text": "A single pixel."}]}}),
        ];
        *route.run_state.lock().unwrap() = Value::Null;
        let app = Arc::new(super::super::super::AppReader::new(Arc::new(
            BrocaSources::new(
                Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                    OPENCODE_RUNNER_MODULE_ID,
                ))))),
                Arc::new(OpenCode2(route.clone())),
            ),
        )));
        let page = app
            .read(
                super::super::super::TRANSCRIPT_PAGE,
                json!({"session": "ses_head", "harness": "opencode2", "limit": 50}),
            )
            .await
            .unwrap();
        assert_eq!(page["messages"][0]["parts"][0]["type"], "file", "{page}");
        let older = app
            .read(
                super::super::super::TRANSCRIPT_PAGE,
                json!({"session": "ses_head", "harness": "opencode2", "limit": 1,
                    "beforeOrdinal": 2}),
            )
            .await
            .unwrap();
        assert_eq!(older["messages"][0]["ordinal"], 1, "{older}");
        app.read(
            super::super::super::TRANSCRIPT_PAGE,
            json!({"session": "ses_head", "harness": "opencode2", "messageOrdinal": 2}),
        )
        .await
        .unwrap();
        let calls = route.calls.lock().unwrap().clone();
        let media = |method: &str| -> Vec<Option<Value>> {
            calls
                .iter()
                .filter(|call| call["method"] == method)
                .map(|call| call["params"].get("media").cloned())
                .collect()
        };
        let reads = media("session.read");
        assert!(reads.len() >= 3, "{groups:?}: {calls:?}");
        let expected = asked.then(|| json!("media_refs"));
        assert!(
            reads.iter().all(|media| *media == expected),
            "{groups:?}: {reads:?}"
        );
        let found = media("session.history");
        assert!(found.iter().all(Option::is_none), "{groups:?}: {found:?}");
    }
}

/// A media read's own refusals reach the app under their own code, as
/// `refusal-unknown_media` shows, so it doesn't ask again for a file the chat
/// doesn't hold. The same codes on any other read, and any other refusal of a
/// media read, still reach it as the runner being unavailable.
#[test]
fn a_media_reads_own_refusals_keep_their_code() {
    let code = super::super::media_refusal;
    assert_eq!(
        code(ReadOp::MediaRead, Some("unknown_media")),
        Some("unknown_media")
    );
    assert_eq!(
        code(ReadOp::MediaRead, Some("invalid_params")),
        Some("invalid_params")
    );
    assert_eq!(code(ReadOp::MediaRead, Some("scope_owner_mismatch")), None);
    assert_eq!(code(ReadOp::MediaRead, None), None);
    assert_eq!(code(ReadOp::Read, Some("unknown_media")), None);
}
