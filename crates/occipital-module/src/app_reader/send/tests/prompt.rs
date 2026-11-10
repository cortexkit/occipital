//! Sends naming files that go into the prompt: each file is copied to the
//! session's runner through the runner contract's uploads (`session.upload_*`)
//! and named in the send's `content`, in the message's order.

use super::*;
use crate::app_reader::{uploads, Caller, SourceResolver};
use std::{path::PathBuf, sync::atomic::AtomicU64};

mod attachments_samples;
mod samples;

const PHONE: &str = "0123456789abcdef0123456789abcdef";
const OTHER_PHONE: &str = "fedcba9876543210fedcba9876543210";

/// One upload the runner holds: what begin declared, and what arrived.
struct Held {
    mime: String,
    bytes: u64,
    sha256: String,
    data: Vec<u8>,
    committed: bool,
}

/// A head on a runner declaring `attachments`, keeping what each upload received.
struct UploadRunner {
    attachments: AtomicBool,
    /// Whether it also declares `media_only_send`, storing a message of files alone.
    media_only: AtomicBool,
    input: Mutex<Option<Value>>,
    /// The head's model, `{provider, model, variant?}`, when it names one.
    model: Mutex<Option<Value>>,
    /// Each upload by its runner id.
    uploads: Mutex<HashMap<String, Held>>,
    next: AtomicU64,
    calls: Mutex<Vec<(String, Value)>>,
    scripted: Mutex<VecDeque<Result<Value, RunnerError>>>,
}

impl UploadRunner {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            attachments: AtomicBool::new(true),
            media_only: AtomicBool::new(false),
            input: Mutex::default(),
            model: Mutex::default(),
            uploads: Mutex::default(),
            next: AtomicU64::new(1),
            calls: Mutex::default(),
            scripted: Mutex::default(),
        })
    }

    fn methods(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(method, _)| method.clone())
            .collect()
    }

    fn last_send(&self) -> Value {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(method, _)| method == "session.send")
            .unwrap()
            .1
            .clone()
    }

    fn received(&self, upload_id: &str) -> Vec<u8> {
        self.uploads.lock().unwrap()[upload_id].data.clone()
    }
}

struct UploadTarget(Arc<UploadRunner>, SessionRef);

#[async_trait]
impl SendTarget for UploadTarget {
    fn reference(&self) -> &SessionRef {
        &self.1
    }

    async fn call(&self, method: &'static str, params: Value) -> Result<Value, RunnerError> {
        let runner = &self.0;
        runner
            .calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        if method == "session.send" {
            if let Some(scripted) = runner.scripted.lock().unwrap().pop_front() {
                return scripted;
            }
        }
        let mut uploads = runner.uploads.lock().unwrap();
        match method {
            "session.head" => {
                let mut head = json!({
                    "incarnation": "inc-a", "first_ordinal": 1, "history_version": 1,
                    "updated_at": 1, "last_run_state": {"run_id": "r1", "state": "completed"},
                    "sender": {"modes": ["steer", "idle_start"], "interrupt": true},
                });
                if let Some(input) = runner.input.lock().unwrap().clone() {
                    head["input"] = input;
                }
                if let Some(model) = runner.model.lock().unwrap().clone() {
                    head["model"] = model;
                }
                Ok(head)
            }
            "role.describe" => {
                let mut groups = vec!["head_sender".to_owned()];
                if runner.attachments.load(Ordering::SeqCst) {
                    groups.push("attachments".into());
                }
                if runner.media_only.load(Ordering::SeqCst) {
                    groups.push("media_only_send".into());
                }
                Ok(
                    serde_json::to_value(RoleDescribe::default().with_capabilities(groups))
                        .unwrap(),
                )
            }
            "session.upload_begin" => {
                let id = format!("ru-{}", runner.next.fetch_add(1, Ordering::SeqCst));
                uploads.insert(
                    id.clone(),
                    Held {
                        mime: params["mime"].as_str().unwrap().into(),
                        bytes: params["bytes"].as_u64().unwrap(),
                        sha256: params["sha256"].as_str().unwrap().into(),
                        data: Vec::new(),
                        committed: false,
                    },
                );
                Ok(json!({"upload_id": id}))
            }
            "session.upload_append" => {
                let upload = uploads
                    .get_mut(params["upload_id"].as_str().unwrap())
                    .unwrap();
                assert_eq!(params["offset"].as_u64().unwrap(), upload.data.len() as u64);
                let piece = decode(params["data"].as_str().unwrap());
                assert!(piece.len() <= CHUNK_MAX);
                upload.data.extend(piece);
                Ok(json!({"received": upload.data.len()}))
            }
            "session.upload_commit" => {
                let upload = uploads
                    .get_mut(params["upload_id"].as_str().unwrap())
                    .unwrap();
                assert_eq!(upload.data.len() as u64, upload.bytes);
                assert_eq!(format!("{:x}", Sha256::digest(&upload.data)), upload.sha256);
                upload.committed = true;
                Ok(json!({"state": "committed", "sha256": upload.sha256,
                    "mime": upload.mime, "bytes": upload.bytes}))
            }
            "session.upload_status" => {
                Ok(match uploads.get(params["upload_id"].as_str().unwrap()) {
                    Some(upload) if upload.committed => {
                        json!({"state": "committed", "received": upload.bytes})
                    }
                    Some(upload) => json!({"state": "open", "received": upload.data.len()}),
                    None => json!({"state": "gone", "received": 0}),
                })
            }
            "session.send" => Ok(json!({"state": "active", "run_id": "r2"})),
            other => panic!("the sender called {other}"),
        }
    }
}

struct UploadTargets(Arc<UploadRunner>);

#[async_trait]
impl SendTargets for UploadTargets {
    async fn open(&self, session: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        Ok(Arc::new(UploadTarget(
            self.0.clone(),
            SessionRef {
                runner: "broca".into(),
                session: session.into(),
            },
        )))
    }
}

/// Where uploads are placed, and a model that takes images and nothing else,
/// as the fit says, or every file when the fourth member is set. The third is
/// the session's head for `session.details`, when a test reads them.
struct Project(
    PathBuf,
    Mutex<crate::app_reader::PromptFit>,
    Mutex<Option<Arc<HeadSource>>>,
    AtomicBool,
);

/// The session's head as `session.details` reads it: the same model and image
/// bounds the runner's `session.head` gives the send path. Nothing else is read.
struct HeadSource(Arc<UploadRunner>, tokio::sync::watch::Sender<u64>);

#[async_trait]
impl occipital_app_reader::source::TranscriptSource for HeadSource {
    fn harness(&self) -> &'static str {
        "broca"
    }
    async fn head(&self) -> Result<occipital_app_reader::source::Head, ReadError> {
        let input = self.0.input.lock().unwrap().clone().unwrap_or_default();
        Ok(occipital_app_reader::source::Head {
            history_version: Some(1),
            incarnation: Some("inc-a".into()),
            model: self.0.model.lock().unwrap().clone(),
            input: occipital_app_reader::source::InputCapability {
                max_image_bytes: input["max_image_bytes"].as_u64(),
                max_image_dimension: input["max_image_dimension"].as_u64(),
                ..Default::default()
            },
            ..Default::default()
        })
    }
    async fn newest(&self, _: u32) -> Result<occipital_app_reader::source::HistoryPage, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "only the head is read here",
        ))
    }
    async fn older(
        &self,
        _: &occipital_app_reader::source::Anchor,
        _: u32,
    ) -> Result<occipital_app_reader::source::HistoryPage, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "only the head is read here",
        ))
    }
    async fn newer(
        &self,
        _: &str,
        _: u32,
    ) -> Result<occipital_app_reader::source::HistoryPage, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "only the head is read here",
        ))
    }
    async fn exact(
        &self,
        _: &occipital_app_reader::source::Anchor,
    ) -> Result<Option<occipital_app_reader::source::Row>, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "only the head is read here",
        ))
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.1.subscribe()
    }
    fn activity(&self) -> Option<(&'static str, i64)> {
        None
    }
}

#[async_trait]
impl SourceResolver for Project {
    async fn resolve(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<Arc<dyn occipital_app_reader::source::TranscriptSource>>, ReadError> {
        Ok(self
            .2
            .lock()
            .unwrap()
            .clone()
            .map(|head| head as Arc<dyn occipital_app_reader::source::TranscriptSource>))
    }

    async fn project_root(&self, _: &str, _: Option<&str>) -> Result<PathBuf, ReadError> {
        Ok(self.0.clone())
    }

    async fn prompt_fit(
        &self,
        _: &str,
        _: Option<&str>,
        mime: &str,
    ) -> Result<Option<crate::app_reader::PromptFit>, ReadError> {
        Ok(
            (mime.starts_with("image/") || self.3.load(Ordering::SeqCst))
                .then(|| self.1.lock().unwrap().clone()),
        )
    }
}

struct Prompt {
    _dir: tempfile::TempDir,
    runner: Arc<UploadRunner>,
    uploads: Arc<uploads::Uploads>,
    resolver: Arc<Project>,
    sender: Arc<Sender>,
    /// The clock both the uploads and the sender read.
    now: Arc<AtomicI64>,
}

impl Prompt {
    fn new() -> Self {
        Self::with_fit(crate::app_reader::PromptFit::default())
    }

    /// A session whose prompt takes images as `fit` says, on a runner declaring
    /// `media_only_send` when `fit` does.
    fn with_fit(fit: crate::app_reader::PromptFit) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::shared(&dir.path().join("store.db"));
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let ids = Arc::new(AtomicU64::new(1));
        let now = Arc::new(AtomicI64::new(START));
        let receipts = Receipts::open(store.clone()).unwrap();
        let clock = now.clone();
        let uploads = uploads::Uploads::with(
            store,
            dir.path(),
            Arc::new(move || clock.load(Ordering::SeqCst)),
            Arc::new(|_: &std::path::Path| Ok(u64::MAX)),
            Arc::new(move || format!("{:032x}", ids.fetch_add(1, Ordering::SeqCst))),
        )
        .unwrap();
        let runner = UploadRunner::new();
        runner.media_only.store(fit.media_only, Ordering::SeqCst);
        let clock = now.clone();
        let sender = Sender::with_clock(
            Arc::new(UploadTargets(runner.clone())),
            Some(receipts),
            Arc::new(move || clock.load(Ordering::SeqCst)),
        )
        .with_uploads(Some(uploads.clone()));
        Self {
            _dir: dir,
            runner,
            uploads,
            resolver: Arc::new(Project(
                project,
                Mutex::new(fit),
                Mutex::default(),
                AtomicBool::new(false),
            )),
            sender: Arc::new(sender),
            now,
        }
    }

    /// One upload operation from the phone.
    async fn serve(&self, method: &str, params: Value, payload: Option<Vec<u8>>) -> Value {
        let caller = Caller::Device {
            class: "phone".into(),
            fingerprint: PHONE.into(),
        };
        self.uploads
            .serve(&caller, method, params, payload, self.resolver.clone())
            .await
            .unwrap()
    }

    /// A finished prompt upload of `data` from the phone to session `s`: its id.
    async fn upload(&self, mime: &str, data: &[u8]) -> String {
        self.upload_named("shot", mime, data).await
    }

    /// [`Self::upload`] of a file named `name`.
    async fn upload_named(&self, name: &str, mime: &str, data: &[u8]) -> String {
        let begun = self
            .serve(uploads::BEGIN, begin_params(name, mime, data), None)
            .await;
        assert_eq!(begun["destination"], "prompt");
        let id = begun["upload_id"].as_str().unwrap().to_owned();
        self.chunk(&id, data).await;
        self.serve(uploads::FINISH, json!({"upload_id": id}), None)
            .await;
        id
    }

    /// The whole of `data` as one piece of upload `id`.
    async fn chunk(&self, id: &str, data: &[u8]) {
        self.serve(
            uploads::CHUNK,
            json!({"upload_id": id, "offset": 0}),
            Some(data.to_vec()),
        )
        .await;
    }
}

/// `upload.begin` for session `s`, asking for the prompt.
fn begin_params(name: &str, mime: &str, data: &[u8]) -> Value {
    json!({"session": "s", "name": name, "mime": mime, "size": data.len(),
        "sha256": format!("{:x}", Sha256::digest(data)), "prompt": true})
}

fn with_files(id: &str, text: &str, files: &[&str]) -> Value {
    let parts: Vec<Value> = files
        .iter()
        .map(|file| json!({"type": "file", "attachment_id": file, "mime": "image/png"}))
        .collect();
    json!({"session": "s", "text": text, "clientMessageId": id, "parts": parts})
}

fn decode(text: &str) -> Vec<u8> {
    crate::base64::decode(text).expect("the sender's pieces are canonical base64")
}

#[tokio::test]
async fn each_file_reaches_the_runner_in_the_messages_order() {
    let p = Prompt::new();
    let small = b"tiny png".to_vec();
    let large: Vec<u8> = (0..CHUNK_MAX + 3).map(|i| (i % 251) as u8).collect();
    let first = p.upload("image/png", &large).await;
    let second = p.upload("image/jpeg", &small).await;

    let receipt = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "look", &[&first, &second]))
        .await
        .unwrap();
    assert_eq!(receipt["ok"], true, "{receipt}");
    assert_eq!(
        p.runner.methods(),
        [
            "session.head",
            "role.describe",
            "session.upload_begin",
            "session.upload_append",
            "session.upload_append",
            "session.upload_commit",
            "session.upload_begin",
            "session.upload_append",
            "session.upload_commit",
            "session.send",
        ]
    );
    let send = p.runner.last_send();
    assert_eq!(
        send["content"],
        json!([{"type": "upload", "upload_id": "ru-1"}, {"type": "upload", "upload_id": "ru-2"}])
    );
    assert_eq!(send["prompt"], "look");
    assert_eq!(p.runner.received("ru-1"), large);
    assert_eq!(p.runner.received("ru-2"), small);
    assert_eq!(p.runner.uploads.lock().unwrap()["ru-2"].mime, "image/jpeg");
    // The runner holds the bytes now: no second send can name them.
    for id in [&first, &second] {
        assert_eq!(
            p.uploads.prompt_file(PHONE, "s", id).unwrap_err().code,
            uploads::codes::UPLOAD_SENT
        );
    }
}

#[tokio::test]
async fn a_message_without_files_keeps_its_send_bytes() {
    let p = Prompt::new();
    p.sender
        .send_from(Some(PHONE), with_files("m1", "hello", &[]))
        .await
        .unwrap();
    assert!(p.runner.last_send().get("content").is_none());
    assert!(!p
        .runner
        .methods()
        .iter()
        .any(|m| m.starts_with("session.upload")));
}

#[tokio::test]
async fn a_resent_message_reuses_the_runners_copy() {
    let p = Prompt::new();
    let id = p.upload("image/png", b"abc").await;
    p.runner
        .scripted
        .lock()
        .unwrap()
        .push_back(Err(RunnerError::Unreachable("route lost".into())));
    let lost = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "see", &[&id]))
        .await
        .unwrap();
    assert_eq!(lost["code"], codes::NOT_SENT);
    assert!(
        p.uploads.prompt_file(PHONE, "s", &id).is_ok(),
        "not sent yet"
    );

    p.runner.calls.lock().unwrap().clear();
    let receipt = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "see", &[&id]))
        .await
        .unwrap();
    assert_eq!(receipt["ok"], true);
    assert_eq!(
        p.runner.methods(),
        [
            "session.head",
            "role.describe",
            "session.upload_status",
            "session.send"
        ]
    );
    assert_eq!(
        p.runner.last_send()["content"],
        json!([{"type": "upload", "upload_id": "ru-1"}])
    );
}

/// The app may upload a message's file again before retrying it, as core's
/// records show: the new upload id carries the same bytes, so the message is the
/// same one.
#[tokio::test]
async fn a_retry_that_uploaded_the_file_again_is_the_same_message() {
    let p = Prompt::new();
    let first = p.upload("image/png", b"abc").await;
    p.runner
        .scripted
        .lock()
        .unwrap()
        .push_back(Err(RunnerError::Unreachable("route lost".into())));
    p.sender
        .send_from(Some(PHONE), with_files("m1", "see", &[&first]))
        .await
        .unwrap();

    // The reply was lost, so nothing is recorded here, but the runner may hold the
    // message: the retry names the copy the first attempt made.
    let again = p.upload("image/png", b"abc").await;
    p.runner.calls.lock().unwrap().clear();
    let receipt = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "see", &[&again]))
        .await
        .unwrap();
    assert_eq!(receipt["duplicate"], false, "{receipt}");
    assert_eq!(
        p.runner.methods(),
        [
            "session.head",
            "role.describe",
            "session.upload_status",
            "session.send"
        ]
    );
    assert_eq!(
        p.runner.last_send()["content"],
        json!([{"type": "upload", "upload_id": "ru-1"}])
    );

    // Accepted now: a third copy is answered from the receipt, untouched by the runner.
    let third = p.upload("image/png", b"abc").await;
    p.runner.calls.lock().unwrap().clear();
    let repeat = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "see", &[&third]))
        .await
        .unwrap();
    assert_eq!(repeat["duplicate"], true, "{repeat}");
    assert!(p.runner.methods().is_empty());

    // Different bytes under the same message id are a conflict.
    let other = p.upload("image/png", b"xyz").await;
    let refusal = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "see", &[&other]))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, codes::CONFLICT);
}

#[tokio::test]
async fn a_copy_the_runner_lost_is_uploaded_again() {
    let p = Prompt::new();
    let id = p.upload("image/png", b"abc").await;
    let lost = uploads::RunnerUpload {
        runner: "broca/s".into(),
        upload_id: "ru-gone".into(),
    };
    p.uploads.record_runner_upload(&id, Some(&lost)).unwrap();
    p.sender
        .send_from(Some(PHONE), with_files("m1", "see", &[&id]))
        .await
        .unwrap();
    assert_eq!(
        p.runner.methods()[2..4],
        ["session.upload_status", "session.upload_begin"]
    );
    assert_eq!(
        p.runner.last_send()["content"],
        json!([{"type": "upload", "upload_id": "ru-1"}])
    );
}

#[tokio::test]
async fn a_runner_or_model_that_cant_take_the_file_is_refused_before_any_write() {
    let p = Prompt::new();
    let id = p.upload("image/png", b"abc").await;

    p.runner.attachments.store(false, Ordering::SeqCst);
    let refusal = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "look", &[&id]))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, MEDIA_UNSUPPORTED);

    p.runner.attachments.store(true, Ordering::SeqCst);
    *p.runner.input.lock().unwrap() = Some(json!({"attachments": "yes", "modalities": ["text"]}));
    let refusal = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "look", &[&id]))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, MEDIA_UNSUPPORTED);
    assert_eq!(
        refusal.detail,
        Some(json!({"attachment_id": id, "mime": "image/png"}))
    );
    assert!(!p
        .runner
        .methods()
        .iter()
        .any(|m| m.starts_with("session.upload") || m == "session.send"));
    assert!(
        p.uploads.prompt_file(PHONE, "s", &id).is_ok(),
        "still placeable"
    );
}

#[tokio::test]
async fn the_runners_media_refusal_reaches_the_app_and_keeps_the_file() {
    let p = Prompt::new();
    let id = p.upload("image/png", b"abc").await;
    p.runner
        .scripted
        .lock()
        .unwrap()
        .push_back(Err(RunnerError::Refused {
            code: "media_unsupported".into(),
            message: "the model takes no images".into(),
            detail: Some(json!({"mime": "image/png"})),
        }));
    let refusal = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "look", &[&id]))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, MEDIA_UNSUPPORTED);
    assert_eq!(refusal.detail, Some(json!({"mime": "image/png"})));
    assert!(p.uploads.prompt_file(PHONE, "s", &id).is_ok());
}

/// Only a runner declaring `media_only_send` stores a message of files alone.
/// Elsewhere such a message is refused before anything is copied or sent, and
/// Occipital makes up no text for it.
#[tokio::test]
async fn a_message_of_files_alone_goes_only_to_a_runner_that_stores_one() {
    let p = Prompt::new();
    let id = p.upload("image/png", b"abc").await;
    let refusal = p
        .sender
        .send_from(Some(PHONE), with_files("m1", " ", &[&id]))
        .await
        .unwrap_err();
    assert_eq!(
        (refusal.code, refusal.message.as_str()),
        ("invalid_request", "a message with files needs text too")
    );
    assert!(!p
        .runner
        .methods()
        .iter()
        .any(|m| m.starts_with("session.upload") || m == "session.send"));

    p.runner.media_only.store(true, Ordering::SeqCst);
    let receipt = p
        .sender
        .send_from(Some(PHONE), with_files("m2", "", &[&id]))
        .await
        .unwrap();
    assert_eq!(receipt["ok"], true, "{receipt}");
    let send = p.runner.last_send();
    assert_eq!(send["prompt"], "");
    assert_eq!(
        send["content"],
        json!([{"type": "upload", "upload_id": "ru-1"}])
    );
}

/// The runner's refusal of an image above the model's limit reaches the app by
/// its own code, with the limit it names, and the file stays sendable or
/// placeable.
#[tokio::test]
async fn the_runners_size_refusal_reaches_the_app_and_keeps_the_file() {
    let p = Prompt::new();
    let id = p.upload("image/png", b"abc").await;
    p.runner
        .scripted
        .lock()
        .unwrap()
        .push_back(Err(RunnerError::Refused {
            code: "media_too_large".into(),
            message: "the image is above the model's limit".into(),
            detail: Some(json!({"max_image_bytes": 2})),
        }));
    let refusal = p
        .sender
        .send_from(Some(PHONE), with_files("m1", "look", &[&id]))
        .await
        .unwrap_err();
    assert_eq!(
        (refusal.code, refusal.detail),
        (MEDIA_TOO_LARGE, Some(json!({"max_image_bytes": 2})))
    );
    assert!(p.uploads.prompt_file(PHONE, "s", &id).is_ok());
}

#[tokio::test]
async fn only_the_uploading_device_can_send_its_file() {
    let p = Prompt::new();
    let id = p.upload("image/png", b"abc").await;
    for device in [None, Some(OTHER_PHONE)] {
        let refusal = p
            .sender
            .send_from(device, with_files("m1", "look", &[&id]))
            .await
            .unwrap_err();
        assert_eq!(refusal.code, uploads::codes::UPLOAD_NOT_YOURS, "{device:?}");
    }
    assert!(!p.runner.methods().iter().any(|m| m == "session.send"));
}
