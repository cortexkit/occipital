//! The sample set the apps pin for sending a photo into the model's prompt,
//! `phone-uploads-prompt-v2`: each request served through the reader's frame
//! loop on a paired phone's route, with the reply bytes the phone receives. Its
//! README says what each file shows. `phone-uploads-prompt-v1` is the same flow
//! before the begin reply carried `media_only` and the image limits; it stays
//! published unchanged, checked by its checksums only.
//!
//! The pieces of each file are sent outside the set, since `upload.chunk` is
//! sampled in `phone-upload-v1`. The project's temporary folder is written as
//! `/path/to/project`, so the set doesn't depend on where the test ran.

use super::*;
use crate::app_reader::{broca::tests::served_samples::Served, AppReader};
use std::time::Duration;
use subc_protocol::{BindIdentity, Principal, RouteTarget};

const SET: &str = "phone-uploads-prompt-v2";
/// The image limits the test session's model reports.
const MAX_IMAGE_BYTES: u64 = 5_242_880;
const MAX_IMAGE_DIMENSION: u64 = 8_000;
pub(super) const PROJECT: &str = "/path/to/project";
const SCREENSHOT: &[u8] = b"a screenshot, as a phone would send its bytes\n";
const PHOTO: &[u8] = b"a second photo\n";
const CLIP: &[u8] = b"a screen recording\n";

/// The paired phone's route, as Callosum binds it.
pub(super) fn phone_route() -> crate::app_reader::RouteBinding {
    (
        Some(Principal::Reserved {
            module_id: "callosum".into(),
        }),
        RouteTarget::ManagementSurface {
            module_id: crate::MODULE_ID.into(),
        },
        None,
        Some(BindIdentity::new(format!("fed-phone:{PHONE}"), "caller")),
        None,
    )
}

pub(super) struct Recorder {
    pub(super) served: Served,
    pub(super) root: String,
    pub(super) corr: u64,
}

impl Recorder {
    /// Serve `method` with `params`, keep request and reply as sample `name`, and
    /// answer the reply's `result`, or its refusal.
    pub(super) async fn keep(&mut self, name: &str, method: &str, params: Value) -> Value {
        self.corr += 1;
        let request = json!({"method": method, "params": params});
        self.served.send(self.corr, &request).await;
        let (_, reply) = self
            .served
            .next(self.corr, Duration::from_secs(5))
            .await
            .expect("the module answers");
        let normal = |text: String| text.replace(&self.root, PROJECT).into_bytes();
        let request = serde_json::from_slice(&normal(request.to_string())).unwrap();
        let reply = normal(String::from_utf8(reply).unwrap());
        self.served.keep(name, &request, reply.clone());
        let reply: Value = serde_json::from_slice(&reply).unwrap();
        if reply.get("result").is_some() {
            reply["result"].clone()
        } else {
            reply
        }
    }
}

pub(super) fn file_part(id: &str, mime: &str, filename: &str) -> Value {
    json!({"type": "file", "attachment_id": id, "mime": mime, "filename": filename})
}

#[tokio::test]
async fn the_prompt_upload_samples_are_what_the_reader_serves() {
    let p = Prompt::with_fit(crate::app_reader::PromptFit {
        media_only: false,
        max_image_bytes: Some(MAX_IMAGE_BYTES),
        max_image_dimension: Some(MAX_IMAGE_DIMENSION),
    });
    let app = Arc::new(
        AppReader::with_sender(p.resolver.clone(), p.sender.clone())
            .with_uploads(Some(p.uploads.clone())),
    );
    let mut r = Recorder {
        served: Served::new(SET, app).on_route(phone_route()),
        root: p.resolver.0.to_string_lossy().into_owned(),
        corr: 0,
    };

    // A screenshot goes into the prompt; a recording, which the model can't see,
    // goes to the project.
    let begun = r
        .keep(
            "01-begin-photo",
            uploads::BEGIN,
            begin_params("screenshot.png", "image/png", SCREENSHOT),
        )
        .await;
    assert_eq!(
        (
            &begun["destination"],
            &begun["media_only"],
            &begun["max_image_bytes"],
            &begun["max_image_dimension"]
        ),
        (
            &json!("prompt"),
            &json!(false),
            &json!(MAX_IMAGE_BYTES),
            &json!(MAX_IMAGE_DIMENSION)
        )
    );
    let screenshot = begun["upload_id"].as_str().unwrap().to_owned();
    p.chunk(&screenshot, SCREENSHOT).await;
    let finished = r
        .keep(
            "02-finish-photo",
            uploads::FINISH,
            json!({"upload_id": screenshot}),
        )
        .await;
    assert_eq!(finished["attachment_id"], screenshot.as_str());
    let begun = r
        .keep(
            "03-begin-video",
            uploads::BEGIN,
            begin_params("recording.mov", "video/quicktime", CLIP),
        )
        .await;
    assert_eq!(begun["destination"], "folder");
    assert!(begun.get("media_only").is_none(), "{begun}");

    // The message naming the screenshot, and its repeat after a lost reply.
    let send = json!({"session": "s", "text": "What's wrong in this screenshot?",
        "clientMessageId": "8b0e7d52-2b59-4ad5-9a37-0f1c6a3c1d01",
        "parts": [file_part(&screenshot, "image/png", "screenshot.png")]});
    let receipt = r
        .keep("04-send", "session.enqueue_user_message", send.clone())
        .await;
    assert_eq!(receipt["duplicate"], false, "{receipt}");
    let repeat = r
        .keep("05-send-repeat", "session.enqueue_user_message", send)
        .await;
    assert_eq!(repeat["duplicate"], true, "{repeat}");

    // A model that turns the photo down: the phone places it in the project and
    // sends its path under a new message id.
    let photo = p.upload_named("photo.jpg", "image/jpeg", PHOTO).await;
    p.runner
        .scripted
        .lock()
        .unwrap()
        .push_back(Err(RunnerError::Refused {
            code: "media_unsupported".into(),
            message: "the model takes no image/jpeg".into(),
            detail: Some(json!({"mime": "image/jpeg"})),
        }));
    let refused = r
        .keep(
            "06-send-media_unsupported",
            "session.enqueue_user_message",
            json!({"session": "s", "text": "And this one?",
                "clientMessageId": "8b0e7d52-2b59-4ad5-9a37-0f1c6a3c1d02",
                "parts": [file_part(&photo, "image/jpeg", "photo.jpg")]}),
        )
        .await;
    assert_eq!(refused["code"], MEDIA_UNSUPPORTED);
    let placed = r
        .keep("07-place", uploads::PLACE, json!({"upload_id": photo}))
        .await;
    let path = placed["path"].as_str().unwrap().replace(PROJECT, &r.root);
    let resent = r
        .keep(
            "08-resend-with-path",
            "session.enqueue_user_message",
            json!({"session": "s", "text": format!("And this one?\nUploaded file: {path}"),
                "clientMessageId": "8b0e7d52-2b59-4ad5-9a37-0f1c6a3c1d03"}),
        )
        .await;
    assert_eq!(resent["ok"], true, "{resent}");

    // A photo above the model's limit: the runner names the limit, and the phone
    // encodes it smaller or places it, as for `media_unsupported`.
    let large = p
        .upload_named("large.jpg", "image/jpeg", b"a photo above the limit\n")
        .await;
    p.runner
        .scripted
        .lock()
        .unwrap()
        .push_back(Err(RunnerError::Refused {
            code: "media_too_large".into(),
            message: "the image is above the model's limit".into(),
            detail: Some(json!({"max_image_bytes": MAX_IMAGE_BYTES})),
        }));
    let too_large = r
        .keep(
            "09-send-media_too_large",
            "session.enqueue_user_message",
            json!({"session": "s", "text": "This one is big",
                "clientMessageId": "8b0e7d52-2b59-4ad5-9a37-0f1c6a3c1d07",
                "parts": [file_part(&large, "image/jpeg", "large.jpg")]}),
        )
        .await;
    assert_eq!(too_large["code"], MEDIA_TOO_LARGE);

    // Refusals the phone handles by code.
    let alone = r
        .keep(
            "refusal-invalid_request-files-alone",
            "session.enqueue_user_message",
            json!({"session": "s", "text": "",
                "clientMessageId": "8b0e7d52-2b59-4ad5-9a37-0f1c6a3c1d04",
                "parts": [file_part(&screenshot, "image/png", "screenshot.png")]}),
        )
        .await;
    assert_eq!(alone["code"], "invalid_request");

    // A runner that stores a message of files alone: begin says so, and a photo
    // with no text is sent.
    p.resolver.1.lock().unwrap().media_only = true;
    p.runner.media_only.store(true, Ordering::SeqCst);
    let begun = r
        .keep(
            "10-begin-photo-alone",
            uploads::BEGIN,
            begin_params("alone.png", "image/png", PHOTO),
        )
        .await;
    assert_eq!(begun["media_only"], true);
    let alone = begun["upload_id"].as_str().unwrap().to_owned();
    p.chunk(&alone, PHOTO).await;
    p.serve(uploads::FINISH, json!({"upload_id": alone}), None)
        .await;
    let sent_alone = r
        .keep(
            "11-send-photo-alone",
            "session.enqueue_user_message",
            json!({"session": "s", "text": "",
                "clientMessageId": "8b0e7d52-2b59-4ad5-9a37-0f1c6a3c1d08",
                "parts": [file_part(&alone, "image/png", "alone.png")]}),
        )
        .await;
    assert_eq!(sent_alone["ok"], true, "{sent_alone}");
    let sent = r
        .keep(
            "refusal-upload_sent",
            uploads::PLACE,
            json!({"upload_id": screenshot}),
        )
        .await;
    assert_eq!(sent["code"], uploads::codes::UPLOAD_SENT);
    let open = p
        .serve(
            uploads::BEGIN,
            begin_params("unfinished.png", "image/png", PHOTO),
            None,
        )
        .await["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let unfinished = r
        .keep(
            "refusal-upload_not_finished",
            "session.enqueue_user_message",
            json!({"session": "s", "text": "Too soon",
                "clientMessageId": "8b0e7d52-2b59-4ad5-9a37-0f1c6a3c1d05",
                "parts": [file_part(&open, "image/png", "unfinished.png")]}),
        )
        .await;
    assert_eq!(unfinished["code"], uploads::codes::UPLOAD_NOT_FINISHED);
    let late = p
        .upload_named("late.png", "image/png", b"a photo sent a day late\n")
        .await;
    p.now.fetch_add(uploads::OPEN_LIFE_MS, Ordering::SeqCst);
    let expired = r
        .keep(
            "refusal-upload_expired",
            "session.enqueue_user_message",
            json!({"session": "s", "text": "Too late",
                "clientMessageId": "8b0e7d52-2b59-4ad5-9a37-0f1c6a3c1d06",
                "parts": [file_part(&late, "image/png", "late.png")]}),
        )
        .await;
    assert_eq!(expired["code"], uploads::codes::UPLOAD_EXPIRED);

    r.served.check();
}
