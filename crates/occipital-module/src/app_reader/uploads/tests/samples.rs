//! The tagged samples the phone pins for the upload operations, recorded
//! from the real handlers through the module's frame loop.
//!
//! `the_published_samples_are_what_the_handlers_answer` records the set again and
//! compares it byte for byte with `tests/fixtures/phone-upload-v1/`.
//! `THALAMUS_PRINT_PHONE_UPLOAD_SAMPLES` is rejected so an inherited flag cannot
//! skip that comparison.

use super::*;

const SET: &str = "tests/fixtures/phone-upload-v1";
/// The tests' temporary project root is replaced with this in every sample, so the
/// samples don't depend on where the test ran.
const PROJECT: &str = "/path/to/project";
const FILE: &[u8] = b"phone upload sample\n";

fn set_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SET)
}

/// Replace the project root with [`PROJECT`] and use `/` in every path, so a sample
/// recorded on any platform has the same bytes.
fn normalize(value: &mut Value, root: &str) {
    match value {
        Value::String(text) => {
            if let Some(rest) = text.strip_prefix(root) {
                *text = format!("{PROJECT}{}", rest.replace('\\', "/"));
            } else if text.contains(root) {
                *text = text.replace(root, PROJECT).replace('\\', "/");
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| normalize(item, root)),
        Value::Object(map) => map.values_mut().for_each(|item| normalize(item, root)),
        _ => {}
    }
}

struct Recorder {
    files: Vec<(String, Vec<u8>)>,
}

impl Recorder {
    fn add(&mut self, file: &str, tag: &str, op: &str, request: Value, reply: Reply, root: &Path) {
        let (key, mut body) = match reply {
            Reply::Result(value) => ("reply", value),
            Reply::Refusal(body) => ("refusal", body),
        };
        let root = root.to_string_lossy();
        normalize(&mut body, &root);
        let mut sample = json!({"tag": tag, "op": op, "request": request});
        sample[key] = body;
        let mut bytes = serde_json::to_vec_pretty(&sample).unwrap();
        bytes.push(b'\n');
        self.files.push((file.into(), bytes));
    }
}

fn chunk_request(upload_id: &str, offset: u64, payload: &[u8]) -> Value {
    json!({"upload_id": upload_id, "offset": offset,
        "payload_hex": payload.iter().map(|b| format!("{b:02x}")).collect::<String>()})
}

async fn record() -> Vec<(String, Vec<u8>)> {
    let mut out = Recorder { files: Vec::new() };
    let f = Fixture::new();
    let root = f.project.clone();
    let mut wire = f.wire();
    wire.bind_all().await;

    let params = begin_params("clip.mov", FILE);
    let reply = wire.call(PHONE_ROUTE, BEGIN, params.clone()).await;
    let id = reply.clone().ok()["upload_id"].as_str().unwrap().to_owned();
    out.add("01-begin.json", "begin", BEGIN, params, reply, &root);

    let (framed, reply) = wire.chunk_on(PHONE_ROUTE, &id, 0, FILE).await;
    let mut request = chunk_request(&id, 0, FILE);
    request["framed_request"] = json!("02-chunk.request.bin");
    out.add("02-chunk.json", "chunk", CHUNK, request, reply, &root);
    out.files.push(("02-chunk.request.bin".into(), framed));

    let reply = wire.id_call(STATUS, &id).await;
    out.add(
        "03-status.json",
        "status",
        STATUS,
        json!({"upload_id": id}),
        reply,
        &root,
    );
    let reply = wire.id_call(FINISH, &id).await;
    out.add(
        "04-finish.json",
        "finish",
        FINISH,
        json!({"upload_id": id}),
        reply,
        &root,
    );

    let aborted = wire.begin("draft.zip", FILE).await;
    let reply = wire.id_call(ABORT, &aborted).await;
    out.add(
        "05-abort.json",
        "abort",
        ABORT,
        json!({"upload_id": aborted}),
        reply,
        &root,
    );

    let mut params = begin_params("huge.mov", b"");
    params["size"] = json!(MAX_FILE_BYTES + 1);
    let reply = wire.call(PHONE_ROUTE, BEGIN, params.clone()).await;
    out.add(
        "refusal-upload_too_large.json",
        codes::UPLOAD_TOO_LARGE,
        BEGIN,
        params,
        reply,
        &root,
    );

    f.free_project
        .store(DISK_FLOOR_BYTES + 10, Ordering::SeqCst);
    let params = begin_params("clip.mov", FILE);
    let reply = wire.call(PHONE_ROUTE, BEGIN, params.clone()).await;
    out.add(
        "refusal-disk_floor.json",
        codes::DISK_FLOOR,
        BEGIN,
        params,
        reply,
        &root,
    );
    f.free_project.store(500 << 30, Ordering::SeqCst);

    let unknown = "00000000000000000000000000000000";
    let reply = wire.id_call(STATUS, unknown).await;
    out.add(
        "refusal-upload_unknown.json",
        codes::UPLOAD_UNKNOWN,
        STATUS,
        json!({"upload_id": unknown}),
        reply,
        &root,
    );

    let gap = wire.begin("clip.mov", FILE).await;
    let reply = wire.chunk(&gap, 10, &FILE[10..]).await;
    out.add(
        "refusal-offset_gap.json",
        codes::OFFSET_GAP,
        CHUNK,
        chunk_request(&gap, 10, &FILE[10..]),
        reply,
        &root,
    );

    let mismatch = wire.begin("clip.mov", FILE).await;
    wire.chunk(&mismatch, 0, FILE).await.ok();
    let other = b"PHONE";
    let reply = wire.chunk(&mismatch, 0, other).await;
    out.add(
        "refusal-chunk_mismatch.json",
        codes::CHUNK_MISMATCH,
        CHUNK,
        chunk_request(&mismatch, 0, other),
        reply,
        &root,
    );

    let mut params = begin_params("clip.mov", FILE);
    params["sha256"] = json!(sha(b"a different file"));
    let bad = wire.call(PHONE_ROUTE, BEGIN, params).await.ok()["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    wire.chunk(&bad, 0, FILE).await.ok();
    let reply = wire.id_call(FINISH, &bad).await;
    out.add(
        "refusal-checksum_mismatch.json",
        codes::CHECKSUM_MISMATCH,
        FINISH,
        json!({"upload_id": bad}),
        reply,
        &root,
    );

    let mut params = begin_params("clip.mov", FILE);
    params["session"] = json!(HELD);
    let reply = wire.call(PHONE_ROUTE, BEGIN, params.clone()).await;
    out.add(
        "refusal-session_on_other_machine.json",
        codes::SESSION_ON_OTHER_MACHINE,
        BEGIN,
        params,
        reply,
        &root,
    );

    let reply = wire
        .call(OTHER_ROUTE, STATUS, json!({"upload_id": mismatch}))
        .await;
    out.add(
        "refusal-upload_not_yours.json",
        codes::UPLOAD_NOT_YOURS,
        STATUS,
        json!({"upload_id": mismatch}),
        reply,
        &root,
    );

    let late = wire.begin("clip.mov", FILE).await;
    wire.chunk(&late, 0, FILE).await.ok();
    f.advance(OPEN_LIFE_MS);
    let reply = wire.id_call(FINISH, &late).await;
    out.add(
        "refusal-upload_expired.json",
        codes::UPLOAD_EXPIRED,
        FINISH,
        json!({"upload_id": late}),
        reply,
        &root,
    );

    // A project whose `.cortexkit/uploads` is a file, not a folder.
    let g = Fixture::new();
    std::fs::create_dir(g.project.join(".cortexkit")).unwrap();
    std::fs::write(g.project.join(".cortexkit").join("uploads"), b"").unwrap();
    let mut wire = g.wire();
    wire.bind_all().await;
    let id = wire.begin("clip.mov", FILE).await;
    wire.chunk(&id, 0, FILE).await.ok();
    let reply = wire.id_call(FINISH, &id).await;
    out.add(
        "refusal-destination_unsafe.json",
        codes::DESTINATION_UNSAFE,
        FINISH,
        json!({"upload_id": id}),
        reply,
        &g.project,
    );
    out.files
}

#[tokio::test]
async fn the_published_samples_are_what_the_handlers_answer() {
    assert!(
        std::env::var_os("THALAMUS_PRINT_PHONE_UPLOAD_SAMPLES").is_none(),
        "unset THALAMUS_PRINT_PHONE_UPLOAD_SAMPLES: correctness tests must compare the published samples"
    );
    let recorded = record().await;
    // Every refusal code the phone's retry policy reads has a sample.
    for code in [
        codes::UPLOAD_TOO_LARGE,
        codes::DISK_FLOOR,
        codes::UPLOAD_UNKNOWN,
        codes::UPLOAD_EXPIRED,
        codes::OFFSET_GAP,
        codes::CHUNK_MISMATCH,
        codes::CHECKSUM_MISMATCH,
        codes::DESTINATION_UNSAFE,
        codes::SESSION_ON_OTHER_MACHINE,
        codes::UPLOAD_NOT_YOURS,
    ] {
        let file = format!("refusal-{code}.json");
        let (_, bytes) = recorded.iter().find(|(name, _)| *name == file).unwrap();
        let sample: Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(sample["refusal"]["code"], code, "{file}");
    }
    for (name, bytes) in &recorded {
        let published = std::fs::read(set_dir().join(name)).unwrap_or_default();
        assert!(
            published == *bytes,
            "{name} differs from what the handlers answer now:\n{}",
            String::from_utf8_lossy(bytes)
        );
    }
    // The framed request is exactly what the codec makes of the sampled head and payload.
    let framed = std::fs::read(set_dir().join("02-chunk.request.bin")).unwrap();
    let (head, payload) = subc_protocol::framed_body::decode(&framed).unwrap();
    assert_eq!(head["method"], CHUNK);
    assert_eq!(payload, FILE);
}

#[test]
fn the_upload_samples_are_complete_and_their_checksums_verify() {
    let root = set_dir();
    let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
    let mut listed = Vec::new();
    for line in sums.lines() {
        let (digest, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(sha(&bytes), digest, "{name}");
        if name.ends_with(".json") {
            let sample: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(
                sample["tag"].is_string() && sample["op"].is_string(),
                "{name}"
            );
            assert!(
                sample["reply"].is_object() ^ sample["refusal"].is_object(),
                "{name}"
            );
        }
        // No sample names the machine it was recorded on.
        let text = String::from_utf8_lossy(&bytes);
        for word in ["/Users/", "/home/", "/tmp/", "Mac"] {
            assert!(!text.contains(word), "{name} contains {word}");
        }
        listed.push(name.to_string());
    }
    let mut present: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "SHA256SUMS" && name != "README.md")
        .collect();
    present.sort();
    listed.sort();
    assert_eq!(listed, present, "every sample file is listed in SHA256SUMS");
    assert_eq!(listed.len(), 16);
}
