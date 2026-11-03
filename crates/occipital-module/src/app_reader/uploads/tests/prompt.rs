//! Uploads bound for the model's prompt: the app opts in at begin, the module
//! picks the destination, finish names the upload instead of placing it, a send
//! takes it as a [`PromptFile`], and `upload.place` puts it in the project after
//! all.

use super::*;

fn prompt_begin(name: &str, mime: &str, content: &[u8]) -> Value {
    json!({"session": SESSION, "name": name, "mime": mime, "size": content.len(),
        "sha256": sha(content), "prompt": true})
}

/// Begin with the prompt opt-in and send every piece: the begin reply.
async fn begin_whole(wire: &mut Wire, mime: &str, data: &[u8]) -> Value {
    let begun = wire
        .call(PHONE_ROUTE, BEGIN, prompt_begin("shot.png", mime, data))
        .await
        .ok();
    let id = begun["upload_id"].as_str().unwrap().to_owned();
    for (index, piece) in data.chunks(1024).enumerate() {
        wire.chunk(&id, (index * 1024) as u64, piece).await.ok();
    }
    begun
}

/// A finished prompt upload: its id.
async fn ready(wire: &mut Wire, data: &[u8]) -> String {
    let begun = begin_whole(wire, "image/png", data).await;
    let id = begun["upload_id"].as_str().unwrap().to_owned();
    wire.id_call(FINISH, &id).await.ok();
    id
}

#[tokio::test]
async fn a_prompt_upload_is_named_not_placed_until_placed() {
    let f = Fixture::new();
    let mut wire = f.prompt_wire(true);
    wire.bind_all().await;
    let data = content(3000);
    let begun = begin_whole(&mut wire, "image/png", &data).await;
    assert_eq!(begun["destination"], "prompt");
    let id = begun["upload_id"].as_str().unwrap().to_owned();

    let finished = wire.id_call(FINISH, &id).await.ok();
    assert_eq!(finished, json!({"attachment_id": id, "size": 3000}));
    assert!(f.blob(&id).exists(), "the bytes wait for a send");
    assert!(!f.date_dir().exists(), "nothing lands in the project");
    assert_eq!(wire.id_call(FINISH, &id).await.ok(), finished);
    let file = f.uploads.prompt_file(PHONE, SESSION, &id).unwrap();
    assert_eq!(
        (file.mime.as_str(), file.size, file.sha256.as_str()),
        ("image/png", 3000, sha(&data).as_str())
    );
    assert_eq!(std::fs::read(&file.blob).unwrap(), data);

    // A refused send's fallback: the same upload placed in the project.
    let placed = wire.id_call(PLACE, &id).await.ok();
    let path = PathBuf::from(placed["path"].as_str().unwrap());
    assert!(path.starts_with(f.date_dir()), "{}", path.display());
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert!(!f.blob(&id).exists());
    assert_eq!(wire.id_call(PLACE, &id).await.ok(), placed);
    assert_eq!(wire.id_call(FINISH, &id).await.ok(), placed);
    assert_eq!(
        f.uploads.prompt_file(PHONE, SESSION, &id).unwrap_err().code,
        codes::UPLOAD_NOT_FOR_PROMPT
    );
}

#[tokio::test]
async fn without_the_opt_in_or_a_taker_the_file_goes_to_the_folder() {
    let f = Fixture::new();
    let data = content(100);

    // An app that predates prompt uploads gets today's reply and a path.
    let mut wire = f.prompt_wire(true);
    wire.bind_all().await;
    let begun = wire
        .call(PHONE_ROUTE, BEGIN, begin_params("a.png", &data))
        .await
        .ok();
    assert!(begun.get("destination").is_none(), "{begun}");
    let id = begun["upload_id"].as_str().unwrap().to_owned();
    wire.chunk(&id, 0, &data).await.ok();
    assert!(wire.id_call(FINISH, &id).await.ok()["path"].is_string());

    // The session's runner or model doesn't take the file.
    let mut refusing = f.prompt_wire(false);
    refusing.bind_all().await;
    let begun = begin_whole(&mut refusing, "image/png", &data).await;
    assert_eq!(begun["destination"], "folder");
    let id = begun["upload_id"].as_str().unwrap().to_owned();
    assert!(refusing.id_call(FINISH, &id).await.ok()["path"].is_string());

    // Too large to copy within a send.
    let large = json!({"session": SESSION, "name": "big.png", "mime": "image/png",
        "size": PROMPT_MAX_BYTES + 1, "sha256": sha(b""), "prompt": true});
    let begun = wire.call(PHONE_ROUTE, BEGIN, large).await.ok();
    assert_eq!(begun["destination"], "folder");
}

#[tokio::test]
async fn a_prompt_upload_whose_bytes_differ_is_refused_at_finish() {
    let f = Fixture::new();
    let mut wire = f.prompt_wire(true);
    wire.bind_all().await;
    let data = content(500);
    let mut params = prompt_begin("shot.png", "image/png", &data);
    params["sha256"] = json!(sha(b"something else"));
    let id = wire.call(PHONE_ROUTE, BEGIN, params).await.ok()["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    wire.chunk(&id, 0, &data).await.ok();
    assert_eq!(
        wire.id_call(FINISH, &id).await.code(),
        codes::CHECKSUM_MISMATCH
    );
    assert_eq!(
        f.uploads.prompt_file(PHONE, SESSION, &id).unwrap_err().code,
        codes::UPLOAD_NOT_FINISHED
    );
}

#[tokio::test]
async fn a_send_takes_only_a_ready_upload_of_its_own_device_and_session() {
    let f = Fixture::new();
    let mut wire = f.prompt_wire(true);
    wire.bind_all().await;
    let data = content(64);
    let open = begin_whole(&mut wire, "image/png", &data).await["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        f.uploads
            .prompt_file(PHONE, SESSION, &open)
            .unwrap_err()
            .code,
        codes::UPLOAD_NOT_FINISHED
    );
    let id = ready(&mut wire, &data).await;
    for (device, session, code) in [
        (OTHER_PHONE, SESSION, codes::UPLOAD_NOT_YOURS),
        (PHONE, "another-session", codes::UPLOAD_UNKNOWN),
    ] {
        assert_eq!(
            f.uploads
                .prompt_file(device, session, &id)
                .unwrap_err()
                .code,
            code
        );
    }

    // Once a send carried it, the runner holds the bytes, and no second send or
    // placement can take it.
    let runner = RunnerUpload {
        runner: "broca/phone-target".into(),
        upload_id: "up-1".into(),
    };
    f.uploads.record_runner_upload(&id, Some(&runner)).unwrap();
    assert_eq!(
        f.uploads
            .prompt_file(PHONE, SESSION, &id)
            .unwrap()
            .runner_upload,
        Some(runner)
    );
    f.uploads.mark_sent(&id).unwrap();
    assert!(!f.blob(&id).exists());
    assert_eq!(
        f.uploads.prompt_file(PHONE, SESSION, &id).unwrap_err().code,
        codes::UPLOAD_SENT
    );
    assert_eq!(wire.id_call(PLACE, &id).await.code(), codes::UPLOAD_SENT);
    assert_eq!(
        wire.id_call(FINISH, &id).await.ok(),
        json!({"attachment_id": id, "size": 64})
    );
}

#[tokio::test]
async fn an_unsent_prompt_upload_expires_with_its_blob() {
    let f = Fixture::new();
    let mut wire = f.prompt_wire(true);
    wire.bind_all().await;
    let id = ready(&mut wire, &content(64)).await;
    // A send has a day from the finish, however long the pieces took.
    f.advance(OPEN_LIFE_MS - 1);
    assert_eq!(f.uploads.cleanup_pass().unwrap(), Swept::default());
    assert!(f.uploads.prompt_file(PHONE, SESSION, &id).is_ok());
    f.advance(1);
    assert_eq!(
        f.uploads.prompt_file(PHONE, SESSION, &id).unwrap_err().code,
        codes::UPLOAD_EXPIRED
    );
    assert_eq!(f.uploads.cleanup_pass().unwrap().expired, 1);
    assert!(!f.blob(&id).exists());
    assert_eq!(wire.id_call(PLACE, &id).await.code(), codes::UPLOAD_EXPIRED);
}

/// A store written before prompt uploads existed keeps its uploads, each read as
/// a folder upload.
#[test]
fn rows_written_before_prompt_uploads_read_as_folder_uploads() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::tests::shared(&dir.path().join("store.db"));
    store
        .migrate(store::NAMESPACE, &store::MIGRATIONS[..1])
        .unwrap();
    store
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO harness_app_upload(upload_id,device,session,project_root,name,mime,\
                 size,sha256,committed,state,created_at,expires_at) \
                 VALUES('u1','d','s','/p','a.png','image/png',3,'x',3,'open',1,2)",
                [],
            )
        })
        .unwrap();
    let rows = store::Rows::open(store).unwrap();
    let row = rows.get("u1").unwrap().unwrap();
    assert_eq!(row.destination, store::Destination::Folder);
    assert_eq!(row.runner_upload, None);
}
