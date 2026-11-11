//! The sample set the apps pin for how many files a message may carry,
//! `phone-attachments-v2`: the `attachments` a session's details report, and
//! the sends held to them, each served through the reader's frame loop on a
//! paired phone's route. Its README says what each file shows.
//!
//! The limits come from fusiform's own catalog replies
//! (`display::model_names` fixtures): a first-party model stating its image
//! count and bounds, and a gateway model stating none. The uploads themselves
//! are made outside the set, since `phone-uploads-prompt-v2` samples them.

use super::samples::{file_part, phone_route, Recorder};
use super::*;
use crate::app_reader::{broca::tests::served_samples::Served, AppReader};
use crate::display::model_names::{tests::fixture, tests::Fake, ModelNames};

// v2 names the harness on every request, as every session request must: a
// request without one is read as a Broca session (v1's requests named none,
// and an app that copied them lost the limits of every other harness).
const SET: &str = "phone-attachments-v2";
/// The runner's own image bounds on the session's head: stricter than the
/// catalog's on bytes, looser on pixels.
const RUNNER_IMAGE_BYTES: u64 = 5_242_880;
const RUNNER_IMAGE_DIMENSION: u64 = 8_000;

/// `n` finished prompt uploads of `mime`, each with bytes of its own.
async fn uploads(p: &Prompt, n: usize, mime: &str, stem: &str) -> Vec<(String, String)> {
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let name = format!("{stem}-{i}");
        let id = p
            .upload_named(&name, mime, format!("{name}, its own bytes\n").as_bytes())
            .await;
        ids.push((id, name));
    }
    ids
}

fn parts(files: &[(String, String)], mime: &str) -> Vec<Value> {
    files
        .iter()
        .map(|(id, name)| file_part(id, mime, name))
        .collect()
}

#[tokio::test]
async fn the_attachment_samples_are_what_the_reader_serves() {
    let p = Prompt::new();
    *p.runner.input.lock().unwrap() = Some(json!({"attachments": "yes",
        "max_image_bytes": RUNNER_IMAGE_BYTES, "max_image_dimension": RUNNER_IMAGE_DIMENSION}));
    *p.resolver.2.lock().unwrap() = Some(Arc::new(HeadSource(
        p.runner.clone(),
        tokio::sync::watch::channel(0).0,
    )));
    p.resolver.3.store(true, Ordering::SeqCst);
    let catalog = Arc::new(Fake::default());
    let mut page = fixture("fusiform-attachment-bounds.json");
    let gateway = fixture("fusiform-gateway-without-bounds.json");
    for (key, facts) in gateway["models"].as_object().unwrap() {
        page["models"][key] = facts.clone();
    }
    catalog.page(Ok(page));
    catalog.targets(Ok(fixture("routing-resolved.json")));
    let app = Arc::new(
        AppReader::with_sender(p.resolver.clone(), p.sender.clone())
            .with_uploads(Some(p.uploads.clone())),
    );
    app.set_model_names(Arc::new(ModelNames::new(catalog)));
    let mut r = Recorder {
        served: Served::new(SET, app).on_route(phone_route()),
        root: p.resolver.0.to_string_lossy().into_owned(),
        corr: 0,
    };
    let opus = json!({"provider": "anthropic", "model": "claude-opus-5-5", "variant": "high"});
    let gateway = json!({"provider": "anyapi", "model": "anthropic/claude-opus-4-6"});

    // A model whose catalog row states its image count and bounds: the count
    // is the catalog's, each bound the stricter of the catalog's and the
    // runner's, and other files are held to 4.
    *p.runner.model.lock().unwrap() = Some(opus);
    let details = r
        .keep(
            "01-details",
            "session.details",
            json!({"session": "s", "harness": "broca", "attachmentLimits": true}),
        )
        .await;
    assert_eq!(
        details["attachments"],
        json!({"maxImages": 600, "maxFiles": 4,
            "imageBytes": RUNNER_IMAGE_BYTES, "imageLongEdgePx": 3000})
    );

    // Twelve photos, more than the 10 Occipital used to allow.
    let photos = uploads(&p, 12, "image/jpeg", "photo").await;
    let sent = r
        .keep(
            "02-send-twelve-photos",
            "session.enqueue_user_message",
            json!({"session": "s", "harness": "broca", "text": "All twelve from the walk",
                "clientMessageId": "6f1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c01",
                "parts": parts(&photos, "image/jpeg")}),
        )
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(
        p.runner.last_send()["content"].as_array().unwrap().len(),
        12
    );

    // Five PDFs: other files are held to 4 whatever the model takes.
    let papers = uploads(&p, 5, "application/pdf", "paper").await;
    let refused = r
        .keep(
            "refusal-too_many_files-files",
            "session.enqueue_user_message",
            json!({"session": "s", "harness": "broca", "text": "Read these",
                "clientMessageId": "6f1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c02",
                "parts": parts(&papers, "application/pdf")}),
        )
        .await;
    assert_eq!(refused["code"], TOO_MANY_FILES, "{refused}");
    assert_eq!(
        refused["detail"],
        json!({"maxImages": 600, "maxFiles": 4, "images": 0, "files": 5})
    );

    // A gateway model the catalog states no bounds for: the fallbacks, with the
    // runner's own image bounds.
    *p.runner.model.lock().unwrap() = Some(gateway);
    let details = r
        .keep(
            "03-details-without-catalog-limits",
            "session.details",
            json!({"session": "s", "harness": "broca", "attachmentLimits": true}),
        )
        .await;
    assert_eq!(
        details["attachments"],
        json!({"maxImages": 4, "maxFiles": 4,
            "imageBytes": RUNNER_IMAGE_BYTES, "imageLongEdgePx": RUNNER_IMAGE_DIMENSION})
    );
    let more = uploads(&p, 5, "image/jpeg", "more").await;
    let sends_before = p.runner.methods().len();
    let refused = r
        .keep(
            "refusal-too_many_files-images",
            "session.enqueue_user_message",
            json!({"session": "s", "harness": "broca", "text": "And these five",
                "clientMessageId": "6f1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c03",
                "parts": parts(&more, "image/jpeg")}),
        )
        .await;
    assert_eq!(refused["code"], TOO_MANY_FILES, "{refused}");
    assert_eq!(
        refused["detail"],
        json!({"maxImages": 4, "maxFiles": 4, "images": 5, "files": 0})
    );
    // Refused before anything was copied to the runner or sent.
    assert!(p.runner.methods()[sends_before..]
        .iter()
        .all(|method| !method.starts_with("session.upload") && method != "session.send"));

    r.served.check();
}
