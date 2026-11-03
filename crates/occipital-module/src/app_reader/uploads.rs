//! Files an app attaches to an agent message. The app sends the file in pieces; the
//! module keeps the committed prefix in a private blob. At finish, the file goes one
//! of two ways:
//! - into the model's prompt: a photo or screenshot for a model that can see it. The
//!   blob waits for a send (`session.enqueue_user_message`) to name the upload, and
//!   that send copies it to the session's runner (see `send`).
//! - into the session's project under `.cortexkit/uploads/<YYYY-MM-DD>/`: anything
//!   the model can't take inline, such as a video, or a photo for a model without
//!   vision. The app puts the file's absolute path in the message text.
//!
//! Six management operations, admitted exactly as the app reads are (a
//! daemon-attested `reserved:callosum` route, checked before any parameter is read):
//!
//! - `upload.begin {session, name, mime, size, sha256, prompt?}` answers
//!   `{upload_id, chunk_bytes, expires_at}` (`expires_at` in milliseconds since the
//!   Unix epoch). The project root comes from the session's Entorhinal binding, never
//!   from the app, and the upload is bound to the opening device's fingerprint. With
//!   `prompt: true` the reply adds `destination`, `prompt` or `folder`, chosen from
//!   the session's runner and model; without it the file always goes to the folder.
//! - `upload.chunk {upload_id, offset}` carries the piece as the payload of a framed
//!   request and answers `{committed}`. Only the contiguous prefix is kept: a piece
//!   overlapping it must match it byte for byte, and a piece starting past it is
//!   refused without being stored.
//! - `upload.status {upload_id}` answers `{committed}`.
//! - `upload.finish {upload_id}` answers `{path, size}` for the folder and
//!   `{attachment_id, size}` for the prompt, and the same again on every retry for 30
//!   days. A prompt upload no send names within a day of its finish expires.
//! - `upload.place {upload_id}` puts a prompt upload in the project after all and
//!   answers `{path, size}`: the app's fallback when a send naming it was refused
//!   `media_unsupported`.
//! - `upload.abort {upload_id}` answers `{aborted}` and discards an unfinished
//!   upload. A finished upload's file and recorded answer are left alone
//!   (`aborted: false`): the app has already been given the path.
//!
//! Every operation but `begin` is refused `upload_not_yours` unless it comes from the
//! device that opened the upload.

pub(crate) mod destination;
pub(crate) mod store;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use occipital_app_reader::source::ReadError;
use serde::Deserialize;
use serde_json::{json, Value};
use subc_protocol::manifest::{ManagementOperation, ManagementOperationKind, OperationPayload};

use super::{Caller, PromptFit, SourceResolver};
use sha2::{Digest, Sha256};
pub(crate) use store::RunnerUpload;
use store::{Destination, Row, Rows, State};

pub const BEGIN: &str = "upload.begin";
pub const CHUNK: &str = "upload.chunk";
pub const STATUS: &str = "upload.status";
pub const FINISH: &str = "upload.finish";
pub const ABORT: &str = "upload.abort";
pub const PLACE: &str = "upload.place";
pub const METHODS: [&str; 6] = [BEGIN, CHUNK, STATUS, FINISH, ABORT, PLACE];

/// The largest file that goes into the prompt. The send that names it copies it to
/// the runner inside the app request's 45-second deadline, so a larger file goes to
/// the project folder instead.
pub const PROMPT_MAX_BYTES: u64 = 32 << 20;

/// The piece size the phone is told to send.
pub const CHUNK_BYTES: u64 = 1 << 20;
/// The largest payload `upload.chunk` declares; Callosum refuses a larger one before
/// it reaches the module.
pub const MAX_CHUNK_PAYLOAD: usize = 8 << 20;
/// The largest file an upload may declare.
pub const MAX_FILE_BYTES: u64 = 2 << 30;
/// Free space each volume must keep after an upload: the module's data volume, which
/// holds the blob, and the project's, which receives the file.
pub const DISK_FLOOR_BYTES: u64 = 20 << 30;
/// How long an unfinished upload lives.
pub const OPEN_LIFE_MS: i64 = 24 * 60 * 60 * 1000;
/// How long a finished upload's answer is kept, and how long its file is left before
/// cleanup may remove it (only if it is still exactly the file written).
pub const RESULT_LIFE_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Rows each cleanup step handles per pass, so a pass never holds the store long.
pub const CLEANUP_BATCH: usize = 64;
const CLEANUP_EVERY: Duration = Duration::from_secs(10 * 60);
/// The folder inside the module's data directory holding unfinished uploads' blobs.
/// The backup catalog declares it excluded.
pub const BLOB_DIR: &str = "upload-blobs";

/// The refusal codes the phone's retry policy reads. The texts beside them are for
/// people only.
pub mod codes {
    pub const UPLOAD_TOO_LARGE: &str = "upload_too_large";
    pub const DISK_FLOOR: &str = "disk_floor";
    pub const UPLOAD_UNKNOWN: &str = "upload_unknown";
    pub const UPLOAD_EXPIRED: &str = "upload_expired";
    pub const OFFSET_GAP: &str = "offset_gap";
    pub const CHUNK_MISMATCH: &str = "chunk_mismatch";
    pub const CHECKSUM_MISMATCH: &str = "checksum_mismatch";
    pub const DESTINATION_UNSAFE: &str = "destination_unsafe";
    pub const SESSION_ON_OTHER_MACHINE: &str = "session_on_other_machine";
    pub const UPLOAD_NOT_YOURS: &str = "upload_not_yours";
    /// A send named an upload whose bytes aren't all held and checked yet.
    pub const UPLOAD_NOT_FINISHED: &str = "upload_not_finished";
    /// A send named an upload an earlier send already carried to the model.
    pub const UPLOAD_SENT: &str = "upload_sent";
    /// A send named an upload that went to the project folder: its path goes in
    /// the message text instead.
    pub const UPLOAD_NOT_FOR_PROMPT: &str = "upload_not_for_prompt";
}

pub fn operations() -> Vec<ManagementOperation> {
    METHODS
        .into_iter()
        .map(|name| ManagementOperation {
            name: name.into(),
            kind: if name == STATUS {
                ManagementOperationKind::Query
            } else {
                ManagementOperationKind::Mutate
            },
            description: None,
        })
        .collect()
}

/// Only `upload.chunk` takes a raw payload.
pub fn operation_payload() -> BTreeMap<String, OperationPayload> {
    [(CHUNK.to_string(), OperationPayload::new(MAX_CHUNK_PAYLOAD))]
        .into_iter()
        .collect()
}

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
pub type FreeSpace = Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>;
pub type Ids = Arc<dyn Fn() -> String + Send + Sync>;

/// A fresh upload id: 128 random bits from the thread's cryptographic generator.
fn random_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn refusal(code: &'static str, message: impl Into<String>) -> ReadError {
    ReadError::new(code, message)
}

fn invalid(message: impl Into<String>) -> ReadError {
    ReadError::new("invalid_request", message)
}

fn storage(error: impl std::fmt::Display) -> ReadError {
    ReadError::new("provider_unavailable", format!("upload storage: {error}"))
}

#[derive(Deserialize)]
struct BeginParams {
    session: String,
    /// The session's harness when it isn't a Broca session: a head's `opencode` or
    /// `pi`, found whatever runner hosts it. Omitted for a Broca session, as before.
    #[serde(default)]
    harness: Option<String>,
    name: String,
    mime: String,
    size: u64,
    sha256: String,
    /// The app can send the file into the model's prompt. Without it the file always
    /// goes to the project folder, so an app that predates prompt uploads, which
    /// expects a path from finish, still gets one.
    #[serde(default)]
    prompt: bool,
}

/// A ready prompt upload, as a send needs it: the bytes to copy to the runner and
/// what to declare for them.
#[derive(Debug, Clone)]
pub(crate) struct PromptFile {
    pub upload_id: String,
    pub blob: PathBuf,
    pub mime: String,
    pub size: u64,
    pub sha256: String,
    pub runner_upload: Option<RunnerUpload>,
}

#[derive(Deserialize)]
struct IdParams {
    upload_id: String,
}

#[derive(Deserialize)]
struct ChunkParams {
    upload_id: String,
    offset: u64,
}

fn parse<T: serde::de::DeserializeOwned>(method: &str, params: Value) -> Result<T, ReadError> {
    serde_json::from_value(params).map_err(|error| invalid(format!("{method}: {error}")))
}

/// What one cleanup pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub expired: usize,
    pub forgotten: usize,
    pub files_removed: usize,
    pub files_kept: usize,
    pub orphan_blobs: usize,
}

pub struct Uploads {
    rows: Rows,
    blobs: PathBuf,
    /// On Unix, the user id owning the upload-blob folder, which this module created
    /// in its own data directory; project folders it writes into must have the same
    /// owner.
    owner: u32,
    clock: Clock,
    free: FreeSpace,
    ids: Ids,
    /// One lock per upload, so pieces of the same upload apply in turn while other
    /// uploads proceed.
    locks: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    /// Test-only crash point between the blob's sync and the row's update.
    #[cfg(test)]
    pub(crate) crash_after_blob_sync: std::sync::atomic::AtomicBool,
}

impl Uploads {
    /// Open the upload rows in `store` and the blob folder in `data_root`.
    pub fn open(
        store: Arc<cortexkit_store::SqliteStore>,
        data_root: &Path,
    ) -> Result<Arc<Self>, crate::Error> {
        Self::with(
            store,
            data_root,
            Arc::new(crate::now_ms),
            Arc::new(|path: &Path| occipital_files::available_bytes(path)),
            Arc::new(random_id),
        )
    }

    pub fn with(
        store: Arc<cortexkit_store::SqliteStore>,
        data_root: &Path,
        clock: Clock,
        free: FreeSpace,
        ids: Ids,
    ) -> Result<Arc<Self>, crate::Error> {
        let rows = Rows::open(store)?;
        let blobs = data_root.join(BLOB_DIR);
        match std::fs::symlink_metadata(&blobs) {
            Ok(meta) if destination::is_link(&meta) || !meta.is_dir() => {
                return Err(format!("{} is not a folder", blobs.display()).into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                destination::create_private_dir(&blobs)?;
            }
            Err(error) => return Err(error.into()),
        }
        #[cfg(unix)]
        let owner = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(&blobs)?.uid()
        };
        #[cfg(not(unix))]
        let owner = 0;
        Ok(Arc::new(Self {
            rows,
            blobs,
            owner,
            clock,
            free,
            ids,
            locks: Mutex::default(),
            #[cfg(test)]
            crash_after_blob_sync: Default::default(),
        }))
    }

    fn blob(&self, upload_id: &str) -> PathBuf {
        self.blobs.join(format!("{upload_id}.part"))
    }

    fn lock(&self, upload_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(upload_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(upload_id.into(), Arc::downgrade(&lock));
        lock
    }

    /// Serve one admitted upload operation. `payload` is the framed request's raw
    /// bytes, which only `upload.chunk` accepts.
    pub async fn serve(
        self: &Arc<Self>,
        caller: &Caller,
        method: &str,
        params: Value,
        payload: Option<Vec<u8>>,
        resolver: Arc<dyn SourceResolver>,
    ) -> Result<Value, ReadError> {
        if payload.is_some() && method != CHUNK {
            return Err(invalid(format!("{method} takes no payload")));
        }
        // Uploads belong to a paired device: the fingerprint in the daemon-attested
        // route identity is 32 lowercase hex characters.
        let device = match caller {
            Caller::Device { fingerprint, .. } if is_hex(fingerprint, 32) => {
                Some(fingerprint.clone())
            }
            _ => None,
        };
        let this = self.clone();
        if method == BEGIN {
            let Some(device) = device else {
                return Err(ReadError::new(
                    "carrier_op_not_permitted",
                    "uploads are opened only by a paired device",
                ));
            };
            let request: BeginParams = parse(method, params)?;
            if request.session.is_empty() || request.name.is_empty() {
                return Err(invalid("upload.begin needs a session and a name"));
            }
            if !is_hex(&request.sha256, 64) {
                return Err(invalid("sha256 must be 64 lowercase hex characters"));
            }
            if request.size > MAX_FILE_BYTES {
                return Err(refusal(
                    codes::UPLOAD_TOO_LARGE,
                    format!("a file may be at most {MAX_FILE_BYTES} bytes"),
                )
                .with_detail(json!({"max_bytes": MAX_FILE_BYTES})));
            }
            let project_root = resolver
                .project_root(&request.session, request.harness.as_deref())
                .await?;
            // The prompt only when the session's runner takes uploads and its model
            // isn't known to refuse this type. A failed check means the folder: the
            // file still reaches the session, as a path.
            let fit = if request.prompt && request.size <= PROMPT_MAX_BYTES {
                resolver
                    .prompt_fit(&request.session, request.harness.as_deref(), &request.mime)
                    .await
                    .unwrap_or(None)
            } else {
                None
            };
            return blocking(move || this.begin(device, request, project_root, fit)).await;
        }
        let device = device.unwrap_or_default();
        match method {
            CHUNK => {
                let request: ChunkParams = parse(method, params)?;
                let payload = payload.unwrap_or_default();
                if payload.len() > MAX_CHUNK_PAYLOAD {
                    return Err(invalid(format!(
                        "a piece may be at most {MAX_CHUNK_PAYLOAD} bytes"
                    )));
                }
                blocking(move || this.chunk(&device, &request.upload_id, request.offset, &payload))
                    .await
            }
            STATUS | FINISH | ABORT | PLACE => {
                let request: IdParams = parse(method, params)?;
                let method = method.to_owned();
                blocking(move || match method.as_str() {
                    STATUS => this.status(&device, &request.upload_id),
                    FINISH => this.finish(&device, &request.upload_id),
                    PLACE => this.place(&device, &request.upload_id),
                    _ => this.abort(&device, &request.upload_id),
                })
                .await
            }
            _ => Err(invalid(format!("unknown upload operation {method}"))),
        }
    }

    /// Refuse unless both volumes keep [`DISK_FLOOR_BYTES`] free after `incoming` more
    /// bytes land on the project's volume and `blob_incoming` on the data volume.
    fn disk_floor(
        &self,
        project_root: &Path,
        incoming: u64,
        blob_incoming: u64,
    ) -> Result<(), ReadError> {
        for (volume, path, adding) in [
            ("data", self.blobs.as_path(), blob_incoming),
            ("project", project_root, incoming),
        ] {
            let available = (self.free)(path)
                .map_err(|error| storage(format!("free space on the {volume} volume: {error}")))?;
            let needed = adding.saturating_add(DISK_FLOOR_BYTES);
            if available < needed {
                return Err(refusal(
                    codes::DISK_FLOOR,
                    format!(
                        "the {volume} volume would keep less than {DISK_FLOOR_BYTES} bytes free"
                    ),
                )
                .with_detail(json!({"volume": volume, "available": available, "needed": needed})));
            }
        }
        Ok(())
    }

    fn begin(
        &self,
        device: String,
        request: BeginParams,
        project_root: PathBuf,
        fit: Option<PromptFit>,
    ) -> Result<Value, ReadError> {
        self.disk_floor(&project_root, request.size, request.size)?;
        let destination = if fit.is_some() {
            Destination::Prompt
        } else {
            Destination::Folder
        };
        let now = (self.clock)();
        let upload_id = (self.ids)();
        let row = Row {
            upload_id: upload_id.clone(),
            device,
            session: request.session,
            project_root,
            name: request.name,
            mime: request.mime,
            size: request.size,
            sha256: request.sha256,
            committed: 0,
            state: State::Open,
            created_at: now,
            expires_at: now + OPEN_LIFE_MS,
            placed: None,
            destination,
            runner_upload: None,
        };
        let prompt = request.prompt;
        // The row first: cleanup removes a blob without a row, so a blob never
        // exists for long without one.
        self.rows.insert(&row).map_err(storage)?;
        if let Err(error) = destination::create_private_file(&self.blob(&upload_id)) {
            let _ = self.rows.delete(&upload_id);
            return Err(storage(error));
        }
        let mut reply = json!({
            "upload_id": upload_id,
            "chunk_bytes": CHUNK_BYTES,
            "expires_at": row.expires_at,
        });
        if prompt {
            reply["destination"] = json!(destination.as_str());
        }
        // What the phone needs to prepare the file: whether a message may carry
        // it with no text, and the largest image the model takes, when the runner
        // reports it.
        if let Some(fit) = fit {
            reply["media_only"] = json!(fit.media_only);
            if let Some(bytes) = fit.max_image_bytes {
                reply["max_image_bytes"] = json!(bytes);
            }
            if let Some(side) = fit.max_image_dimension {
                reply["max_image_dimension"] = json!(side);
            }
        }
        Ok(reply)
    }

    /// The upload's row, refused `upload_unknown` when there is none (or its record
    /// has outlived its life) and `upload_not_yours` when another device opened it.
    fn lookup(&self, device: &str, upload_id: &str) -> Result<Row, ReadError> {
        let unknown = || refusal(codes::UPLOAD_UNKNOWN, "no such upload");
        // Checked before the id is used in a file name.
        if !is_hex(upload_id, 32) {
            return Err(unknown());
        }
        let row = self
            .rows
            .get(upload_id)
            .map_err(storage)?
            .ok_or_else(unknown)?;
        let now = (self.clock)();
        let gone = match (&row.state, &row.placed) {
            (State::Finished, Some(placed)) => placed.finished_at + RESULT_LIFE_MS <= now,
            (State::Expired, _) => row.created_at + RESULT_LIFE_MS <= now,
            _ => false,
        };
        if gone {
            return Err(unknown());
        }
        if row.device != device {
            return Err(refusal(
                codes::UPLOAD_NOT_YOURS,
                "the upload was opened by another device",
            ));
        }
        Ok(row)
    }

    fn refuse_expired(&self, row: &Row) -> Result<(), ReadError> {
        if row.state == State::Expired
            || (row.state == State::Open && (self.clock)() >= row.expires_at)
        {
            return Err(refusal(
                codes::UPLOAD_EXPIRED,
                "the upload was not finished within its 24 hours",
            ));
        }
        Ok(())
    }

    /// Open the blob and bring the row and the blob to the same durable prefix: a
    /// blob shorter than the row (a store restored without its blobs) lowers the
    /// row, and bytes past the row's prefix (written before a crash, never
    /// acknowledged) are cut off.
    fn reconcile(&self, row: &Row) -> Result<(File, u64), ReadError> {
        let path = self.blob(&row.upload_id);
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true);
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                destination::create_private_file(&path).map_err(storage)?
            }
            Err(error) => return Err(storage(error)),
        };
        let len = file.metadata().map_err(storage)?.len();
        let mut committed = row.committed;
        if len < committed {
            committed = len;
            self.rows
                .set_committed(&row.upload_id, committed)
                .map_err(storage)?;
        } else if len > committed {
            file.set_len(committed).map_err(storage)?;
            file.sync_all().map_err(storage)?;
        }
        Ok((file, committed))
    }

    fn chunk(
        &self,
        device: &str,
        upload_id: &str,
        offset: u64,
        payload: &[u8],
    ) -> Result<Value, ReadError> {
        let lock = self.lock(upload_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let row = self.lookup(device, upload_id)?;
        if row.state == State::Finished {
            // Every piece was taken before the finish, and the blob is gone, so a late
            // repeat is answered without being stored or compared.
            return Ok(json!({"committed": row.size}));
        }
        self.refuse_expired(&row)?;
        let (mut file, committed) = self.reconcile(&row)?;
        if offset > committed {
            return Err(refusal(
                codes::OFFSET_GAP,
                format!("the piece starts at {offset}, past the {committed} bytes held"),
            )
            .with_detail(json!({"committed": committed})));
        }
        let end = offset + payload.len() as u64;
        if end > row.size {
            return Err(refusal(
                codes::UPLOAD_TOO_LARGE,
                format!(
                    "the piece ends at {end}, past the declared {} bytes",
                    row.size
                ),
            )
            .with_detail(json!({"size": row.size})));
        }
        // The part already held must be the same bytes: a repeat is accepted, a
        // different piece at the same place is not.
        let overlap = (end.min(committed) - offset) as usize;
        if overlap > 0 {
            let mut held = vec![0; overlap];
            file.seek(SeekFrom::Start(offset)).map_err(storage)?;
            file.read_exact(&mut held).map_err(storage)?;
            if held != payload[..overlap] {
                return Err(refusal(
                    codes::CHUNK_MISMATCH,
                    format!("the piece at {offset} differs from the bytes already held"),
                )
                .with_detail(json!({"committed": committed})));
            }
        }
        if end > committed {
            file.seek(SeekFrom::Start(committed)).map_err(storage)?;
            file.write_all(&payload[overlap..]).map_err(storage)?;
            // Durable before the prefix moves, so the row never claims bytes a crash
            // could lose.
            file.sync_data().map_err(storage)?;
            #[cfg(test)]
            if self
                .crash_after_blob_sync
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(storage("simulated crash after the blob sync"));
            }
            self.rows.set_committed(upload_id, end).map_err(storage)?;
        }
        Ok(json!({"committed": end.max(committed)}))
    }

    fn status(&self, device: &str, upload_id: &str) -> Result<Value, ReadError> {
        let lock = self.lock(upload_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let row = self.lookup(device, upload_id)?;
        if row.state == State::Finished {
            return Ok(json!({"committed": row.size}));
        }
        self.refuse_expired(&row)?;
        let (_, committed) = self.reconcile(&row)?;
        Ok(json!({"committed": committed}))
    }

    fn finish(&self, device: &str, upload_id: &str) -> Result<Value, ReadError> {
        let lock = self.lock(upload_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let row = self.lookup(device, upload_id)?;
        // A retry after a lost reply gets the recorded answer unchanged.
        match (row.state, &row.placed) {
            (State::Finished, Some(placed)) => return Ok(placed_answer(placed, row.size)),
            (State::Ready | State::Sent, _) => return Ok(prompt_answer(&row)),
            _ => {}
        }
        self.refuse_expired(&row)?;
        let (blob, committed) = self.reconcile(&row)?;
        held_whole(&row, committed)?;
        if row.destination == Destination::Prompt {
            // The bytes stay in the blob until a send copies them to the runner, so
            // they are checked here, as placing a file checks them while copying.
            if sha256_of(&blob).map_err(storage)? != row.sha256 {
                return Err(refusal(
                    codes::CHECKSUM_MISMATCH,
                    "the held bytes don't match the declared sha256",
                ));
            }
            let now = (self.clock)();
            self.rows
                .ready(upload_id, now, now + OPEN_LIFE_MS)
                .map_err(storage)?;
            return Ok(prompt_answer(&row));
        }
        self.place_held(&row, blob)
    }

    /// Put a prompt upload in the project folder after all, as a folder upload's
    /// finish does: the app's fallback when a send naming it was refused, for
    /// example because the model can't take its type. A folder upload's finished
    /// file answers the same; an unfinished upload is finished into the folder.
    fn place(&self, device: &str, upload_id: &str) -> Result<Value, ReadError> {
        let lock = self.lock(upload_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let row = self.lookup(device, upload_id)?;
        match (row.state, &row.placed) {
            (State::Finished, Some(placed)) => return Ok(placed_answer(placed, row.size)),
            (State::Sent, _) => {
                return Err(refusal(
                    codes::UPLOAD_SENT,
                    "a send already carried this file to the model",
                ))
            }
            _ => {}
        }
        self.refuse_expired(&row)?;
        let (blob, committed) = self.reconcile(&row)?;
        held_whole(&row, committed)?;
        self.place_held(&row, blob)
    }

    /// Copy an upload's whole blob into the session's project, record the file and
    /// drop the blob.
    fn place_held(&self, row: &Row, mut blob: File) -> Result<Value, ReadError> {
        let upload_id = row.upload_id.as_str();
        // The blob already sits on the data volume; the copy adds the file to the
        // project's.
        self.disk_floor(&row.project_root, row.size, 0)?;
        blob.seek(SeekFrom::Start(0)).map_err(storage)?;
        let now = (self.clock)();
        let name = destination::clean_name(&row.name);
        let date = destination::utc_date(now);
        let placed = destination::place(
            &mut blob,
            &destination::Request {
                project_root: &row.project_root,
                date: &date,
                name: &name,
                upload_id,
                size: row.size,
                sha256: &row.sha256,
                owner: self.owner,
            },
        )
        .map_err(|error| match error {
            destination::PlaceError::Unsafe(reason) => refusal(codes::DESTINATION_UNSAFE, reason),
            destination::PlaceError::Checksum(reason) => refusal(codes::CHECKSUM_MISMATCH, reason),
            destination::PlaceError::Io(error) => storage(error),
        })?;
        self.rows
            .finish(
                upload_id,
                &store::Placed {
                    path: placed.path.clone(),
                    inode: placed.inode,
                    mtime_ns: placed.mtime_ns,
                    finished_at: now,
                },
            )
            .map_err(storage)?;
        drop(blob);
        // The answer is recorded, so the blob is no longer needed. A failed removal
        // leaves it to cleanup's orphan sweep.
        let _ = std::fs::remove_file(self.blob(upload_id));
        Ok(json!({"path": placed.path.to_string_lossy(), "size": row.size}))
    }

    /// The ready prompt upload `attachment_id`, for a send from `device` to
    /// `session`. An upload of another session reads as unknown, as one of another
    /// device reads as not yours.
    pub(crate) fn prompt_file(
        &self,
        device: &str,
        session: &str,
        attachment_id: &str,
    ) -> Result<PromptFile, ReadError> {
        let row = self.lookup(device, attachment_id)?;
        if row.session != session {
            return Err(refusal(codes::UPLOAD_UNKNOWN, "no such upload"));
        }
        if row.destination == Destination::Folder {
            return Err(refusal(
                codes::UPLOAD_NOT_FOR_PROMPT,
                "this upload went to the project folder; name its path in the message text",
            ));
        }
        match row.state {
            State::Ready if (self.clock)() < row.expires_at => Ok(PromptFile {
                blob: self.blob(&row.upload_id),
                upload_id: row.upload_id,
                mime: row.mime,
                size: row.size,
                sha256: row.sha256,
                runner_upload: row.runner_upload,
            }),
            State::Ready | State::Expired => Err(refusal(
                codes::UPLOAD_EXPIRED,
                "no send named the upload within its 24 hours",
            )),
            State::Sent => Err(refusal(
                codes::UPLOAD_SENT,
                "a send already carried this file to the model",
            )),
            State::Open | State::Finished => Err(refusal(
                codes::UPLOAD_NOT_FINISHED,
                "finish the upload before a send names it",
            )),
        }
    }

    /// The SHA-256 of each file `attachment_ids` names, in order, for a send from
    /// `device` to `session`: a send's duplicate check matches files by their bytes.
    /// A sent upload still answers, so a repeat of an accepted message is recognized.
    pub(crate) fn digests(
        &self,
        device: &str,
        session: &str,
        attachment_ids: &[String],
    ) -> Result<Vec<String>, ReadError> {
        attachment_ids
            .iter()
            .map(|id| {
                let row = self.lookup(device, id)?;
                if row.session != session {
                    return Err(refusal(codes::UPLOAD_UNKNOWN, "no such upload"));
                }
                Ok(row.sha256)
            })
            .collect()
    }

    /// The runner upload an earlier send from `device` copied `file`'s bytes to in
    /// `runner` (`<runner module>/<session>`): a message re-uploaded under a new
    /// upload id is sent with the copy its first attempt made, so the runner sees
    /// the same message.
    pub(crate) fn runner_copy(
        &self,
        device: &str,
        session: &str,
        file: &PromptFile,
        runner: &str,
    ) -> Result<Option<String>, crate::Error> {
        self.rows
            .runner_copy(device, session, &file.sha256, &file.mime, runner)
    }

    /// Remember, or forget with `None`, the runner upload a ready upload was copied
    /// to, so a retried send reuses it.
    pub(crate) fn record_runner_upload(
        &self,
        attachment_id: &str,
        runner: Option<&RunnerUpload>,
    ) -> Result<(), crate::Error> {
        let lock = self.lock(attachment_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        self.rows.set_runner_upload(attachment_id, runner)
    }

    /// A send carrying the upload was accepted: the runner holds the bytes now, so
    /// the blob goes. A failed removal leaves it to cleanup's orphan sweep.
    pub(crate) fn mark_sent(&self, attachment_id: &str) -> Result<(), crate::Error> {
        let lock = self.lock(attachment_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        self.rows.sent(attachment_id)?;
        let _ = std::fs::remove_file(self.blob(attachment_id));
        Ok(())
    }

    fn abort(&self, device: &str, upload_id: &str) -> Result<Value, ReadError> {
        let lock = self.lock(upload_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let row = self.lookup(device, upload_id)?;
        if row.state == State::Finished {
            return Ok(json!({"aborted": false}));
        }
        // The blob before the row: a crash between them leaves a row whose blob is
        // missing, which reads as an empty prefix, never a blob nothing refers to.
        match std::fs::remove_file(self.blob(upload_id)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage(error)),
        }
        self.rows.delete(upload_id).map_err(storage)?;
        Ok(json!({"aborted": true}))
    }

    /// One bounded cleanup pass:
    /// - unfinished uploads past their 24 hours lose their blob and become expired
    ///   records, so a late finish is told `upload_expired`;
    /// - expired records and finished answers older than 30 days are forgotten;
    /// - a finished file older than 30 days is removed only if it is still exactly
    ///   the file written (same identity, size and modification time). Nothing else
    ///   in `uploads/` is ever touched;
    /// - blobs no unfinished upload refers to, older than 24 hours, are removed.
    pub fn cleanup_pass(&self) -> Result<Swept, crate::Error> {
        let now = (self.clock)();
        let mut swept = Swept::default();
        let due = self.rows.due_open(now, CLEANUP_BATCH)?;
        for row in due
            .into_iter()
            .chain(self.rows.due_ready(now, CLEANUP_BATCH)?)
        {
            let lock = self.lock(&row.upload_id);
            let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
            match std::fs::remove_file(self.blob(&row.upload_id)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            self.rows.expire(&row.upload_id)?;
            swept.expired += 1;
        }
        let cutoff = now - RESULT_LIFE_MS;
        for row in self.rows.due_expired(cutoff, CLEANUP_BATCH)? {
            self.rows.delete(&row.upload_id)?;
            swept.forgotten += 1;
        }
        for row in self.rows.due_finished(cutoff, CLEANUP_BATCH)? {
            let lock = self.lock(&row.upload_id);
            let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(placed) = &row.placed {
                if unchanged(placed, row.size) {
                    match std::fs::remove_file(&placed.path) {
                        Ok(()) => swept.files_removed += 1,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                } else {
                    swept.files_kept += 1;
                }
            }
            self.rows.delete(&row.upload_id)?;
            swept.forgotten += 1;
        }
        swept.orphan_blobs = self.sweep_orphans(now)?;
        Ok(swept)
    }

    fn sweep_orphans(&self, now: i64) -> Result<usize, crate::Error> {
        let mut removed = 0;
        for entry in std::fs::read_dir(&self.blobs)?.take(4 * CLEANUP_BATCH) {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(upload_id) = name.strip_suffix(".part").filter(|id| is_hex(id, 32)) else {
                continue;
            };
            let lock = self.lock(upload_id);
            let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
            if self
                .rows
                .get(upload_id)?
                .is_some_and(|row| matches!(row.state, State::Open | State::Ready))
            {
                continue;
            }
            let meta = std::fs::symlink_metadata(entry.path())?;
            let age_ms = destination::mtime_ns(&meta).map_or(i64::MAX, |at| now - at / 1_000_000);
            if age_ms >= OPEN_LIFE_MS {
                std::fs::remove_file(entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Clean up now and then every ten minutes, off the request path, until the
    /// returned task is aborted.
    pub fn start(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let Some(uploads) = this.upgrade() else {
                    return;
                };
                let outcome = tokio::task::spawn_blocking(move || uploads.cleanup_pass()).await;
                match outcome {
                    Ok(Ok(swept)) if swept != Swept::default() => {
                        tracing::info!(target: "uploads", ?swept, "upload cleanup pass");
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(target: "uploads", reason = "upload_cleanup_failed", %error, "upload cleanup pass failed");
                    }
                    Err(error) => {
                        tracing::warn!(target: "uploads", reason = "upload_cleanup_failed", %error, "upload cleanup pass failed");
                    }
                }
                tokio::time::sleep(CLEANUP_EVERY).await;
            }
        })
    }
}

fn placed_answer(placed: &store::Placed, size: u64) -> Value {
    json!({"path": placed.path.to_string_lossy(), "size": size})
}

/// A prompt upload's finish: the id a send names it by, which is the upload's own.
fn prompt_answer(row: &Row) -> Value {
    json!({"attachment_id": row.upload_id, "size": row.size})
}

/// Refuse unless every declared byte is held.
fn held_whole(row: &Row, committed: u64) -> Result<(), ReadError> {
    if committed != row.size {
        return Err(refusal(
            codes::CHECKSUM_MISMATCH,
            format!("{committed} of the declared {} bytes are held", row.size),
        )
        .with_detail(json!({"committed": committed, "size": row.size})));
    }
    Ok(())
}

/// The lowercase hex SHA-256 of a file's bytes, read from its start.
fn sha256_of(mut file: &File) -> io::Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(format!("{:x}", hasher.finalize()));
        }
        hasher.update(&buffer[..read]);
    }
}

/// Whether the finished file is still exactly the one written: not replaced by a
/// link, the same file identity, size and modification time.
fn unchanged(placed: &store::Placed, size: u64) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(&placed.path) else {
        return false;
    };
    if destination::is_link(&meta) || !meta.is_file() || meta.len() != size {
        return false;
    }
    if destination::mtime_ns(&meta).ok() != Some(placed.mtime_ns) {
        return false;
    }
    File::open(&placed.path)
        .and_then(|file| destination::file_identity(&file))
        .is_ok_and(|identity| identity == placed.inode)
}

async fn blocking(
    work: impl FnOnce() -> Result<Value, ReadError> + Send + 'static,
) -> Result<Value, ReadError> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| Err(storage(format!("upload worker failed: {error}"))))
}
