//! The sample set the apps pin for sending a video from the phone's library,
//! `phone-uploads-video-v1`: each request served through the reader's frame
//! loop on a paired phone's route, with the reply bytes the phone receives. Its
//! README says what each file shows.
//!
//! Both cases end in the project folder: a video to a model that takes images
//! only, and a video too large for any prompt. A video that goes into the
//! prompt waits on a runner whose model takes video. The pieces of the file
//! are sent outside the set, since `upload.chunk` is sampled in
//! `phone-upload-v1`; the file itself is in the set, so an app can send the
//! same bytes.

use super::samples::{phone_route, Recorder};
use super::*;
use crate::app_reader::{broca::tests::served_samples::Served, AppReader};

const SET: &str = "phone-uploads-video-v1";
/// A real one-second QuickTime movie, 32 by 32 pixels, as the phone's library
/// hands it over.
const CLIP: &[u8] = include_bytes!("clip.mov");
/// A size past the 32 MiB a prompt takes, as a longer recording declares it.
const LARGE: u64 = 48 << 20;

#[tokio::test]
async fn the_video_upload_samples_are_what_the_reader_serves() {
    let p = Prompt::new();
    let app = Arc::new(
        AppReader::with_sender(p.resolver.clone(), p.sender.clone())
            .with_uploads(Some(p.uploads.clone())),
    );
    let mut r = Recorder {
        served: Served::new(SET, app).on_route(phone_route()),
        root: p.resolver.0.to_string_lossy().into_owned(),
        corr: 0,
    };

    // A model that takes images only: the video goes to the project, and its
    // path is what the phone puts in the message.
    let begun = r
        .keep(
            "01-begin-video",
            uploads::BEGIN,
            begin_params("IMG_0042.MOV", "video/quicktime", CLIP),
        )
        .await;
    assert_eq!(begun["destination"], "folder", "{begun}");
    let clip = begun["upload_id"].as_str().unwrap().to_owned();
    p.chunk(&clip, CLIP).await;
    let status = r
        .keep("02-status", uploads::STATUS, json!({"upload_id": clip}))
        .await;
    assert_eq!(status["committed"], CLIP.len());
    let finished = r
        .keep("03-finish", uploads::FINISH, json!({"upload_id": clip}))
        .await;
    assert_eq!(finished["size"], CLIP.len());
    let path = finished["path"].as_str().unwrap();
    assert!(
        path.starts_with(&format!("{}/.cortexkit/uploads/", super::samples::PROJECT)),
        "{path}"
    );
    let on_disk = path.replace(super::samples::PROJECT, &r.root);
    assert_eq!(std::fs::read(&on_disk).unwrap(), CLIP);

    // A model that takes every file, and a video past the prompt's size: it
    // goes to the project all the same.
    p.resolver.3.store(true, Ordering::SeqCst);
    let large = r
        .keep(
            "04-begin-video-over-prompt-size",
            uploads::BEGIN,
            json!({"session": "s", "name": "IMG_0043.MOV", "mime": "video/quicktime",
                "size": LARGE, "sha256": "ab".repeat(32), "prompt": true}),
        )
        .await;
    assert_eq!(large["destination"], "folder", "{large}");
    const { assert!(LARGE > uploads::PROMPT_MAX_BYTES) };

    r.served.keep_file("IMG_0042.MOV", CLIP.to_vec());
    r.served.check();
}
