//! Upload operations served through the reader as the module's handler serves
//! them, plus the pure name rules, which run on every platform.

use super::*;
use crate::app_reader::{
    broca::{BrocaRoute, BrocaRoutes, BrocaSources},
    resolve::{Entorhinal, Resolved, Resolver},
    AppReader, RouteBinding,
};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use subc_protocol::{BindIdentity, Frame, FrameType, Principal, RouteTarget};
use tokio::sync::mpsc;

mod names;
mod ops;
mod samples;

/// 2026-01-01T00:00:00Z.
pub(super) const START: i64 = 1_767_225_600_000;
pub(super) const PHONE: &str = "0123456789abcdef0123456789abcdef";
pub(super) const OTHER_PHONE: &str = "fedcba9876543210fedcba9876543210";
pub(super) const SESSION: &str = "phone-target";
pub(super) const HELD: &str = "held-session";
const GIB: u64 = 1 << 30;

/// Entorhinal's session lookup for two Broca sessions: one bound to the test's
/// project, one held on another machine. Anything else is not found.
struct Lookup {
    root: PathBuf,
}

#[async_trait]
impl Entorhinal for Lookup {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        let session = params["session"].as_str().unwrap_or_default();
        if method != "agent.session_for_runner" || (session != SESSION && session != HELD) {
            return Ok(json!({"code": "not_found", "message":"not registered"}));
        }
        Ok(json!({"result": {
            "agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref",
            "incarnation":"entorhinal-test", "ended_at_ms":null,
            "runner": "broca", "session": session, "harness": "broca",
            "project_root": self.root,
            "heldOn": (session == HELD).then_some("other-computer"),
        }}))
    }
}

/// Uploads never open a transcript route.
struct NoRoutes;

#[async_trait]
impl BrocaRoutes for NoRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        panic!("uploads must not open a transcript route")
    }
}

pub(super) struct Fixture {
    pub dir: tempfile::TempDir,
    pub store: Arc<cortexkit_store::SqliteStore>,
    pub project: PathBuf,
    pub now: Arc<AtomicI64>,
    /// Free bytes reported for the data volume and the project's volume.
    pub free_data: Arc<AtomicU64>,
    pub free_project: Arc<AtomicU64>,
    pub next_id: Arc<AtomicU64>,
    pub uploads: Arc<Uploads>,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::shared(&dir.path().join("store.db"));
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let now = Arc::new(AtomicI64::new(START));
        let free_data = Arc::new(AtomicU64::new(500 * GIB));
        let free_project = Arc::new(AtomicU64::new(500 * GIB));
        let next_id = Arc::new(AtomicU64::new(1));
        Self {
            uploads: Self::uploads_over(
                &store,
                dir.path(),
                &project,
                now.clone(),
                free_data.clone(),
                free_project.clone(),
                next_id.clone(),
            ),
            dir,
            store,
            project,
            now,
            free_data,
            free_project,
            next_id,
        }
    }

    fn uploads_over(
        store: &Arc<cortexkit_store::SqliteStore>,
        data: &Path,
        project: &Path,
        now: Arc<AtomicI64>,
        free_data: Arc<AtomicU64>,
        free_project: Arc<AtomicU64>,
        next_id: Arc<AtomicU64>,
    ) -> Arc<Uploads> {
        let project = project.to_path_buf();
        Uploads::with(
            store.clone(),
            data,
            Arc::new(move || now.load(Ordering::SeqCst)),
            Arc::new(move |path: &Path| {
                Ok(if path.starts_with(&project) {
                    free_project.load(Ordering::SeqCst)
                } else {
                    free_data.load(Ordering::SeqCst)
                })
            }),
            // Ids are deterministic so the published samples are reproducible.
            Arc::new(move || {
                format!(
                    "{:032x}",
                    0xa11ce_u128 << 64 | u128::from(next_id.fetch_add(1, Ordering::SeqCst))
                )
            }),
        )
        .unwrap()
    }

    /// The module starting again over the same store and data directory.
    pub fn restart(&mut self) {
        self.uploads = Self::uploads_over(
            &self.store,
            self.dir.path(),
            &self.project,
            self.now.clone(),
            self.free_data.clone(),
            self.free_project.clone(),
            self.next_id.clone(),
        );
    }

    pub fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn blob(&self, upload_id: &str) -> PathBuf {
        self.dir
            .path()
            .join(BLOB_DIR)
            .join(format!("{upload_id}.part"))
    }

    pub fn date_dir(&self) -> PathBuf {
        self.project
            .join(".cortexkit")
            .join("uploads")
            .join(destination::utc_date(self.now.load(Ordering::SeqCst)))
    }

    pub fn app(&self) -> Arc<AppReader> {
        let resolver = Arc::new(BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(Lookup {
                root: self.project.clone(),
            }))),
            Arc::new(NoRoutes),
        ));
        Arc::new(AppReader::new(resolver).with_uploads(Some(self.uploads.clone())))
    }

    pub fn wire(&self) -> Wire {
        Wire::new(self.app())
    }
}

pub(super) fn binding(principal: &str, harness: &str) -> RouteBinding {
    (
        Some(Principal::Reserved {
            module_id: principal.into(),
        }),
        RouteTarget::ManagementSurface {
            module_id: crate::MODULE_ID.into(),
        },
        None,
        Some(BindIdentity::new(harness, "caller")),
        None,
    )
}

pub(super) fn device(fingerprint: &str) -> RouteBinding {
    binding("callosum", &format!("fed-phone:{fingerprint}"))
}

/// What a call was answered with.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Reply {
    Result(Value),
    Refusal(Value),
}

impl Reply {
    pub fn ok(self) -> Value {
        match self {
            Self::Result(value) => value,
            Self::Refusal(body) => panic!("expected a result, got refusal {body}"),
        }
    }

    pub fn code(&self) -> String {
        match self {
            Self::Refusal(body) => body["code"].as_str().unwrap().to_owned(),
            Self::Result(value) => panic!("expected a refusal, got {value}"),
        }
    }

    pub fn body(&self) -> &Value {
        match self {
            Self::Refusal(body) | Self::Result(body) => body,
        }
    }
}

/// The reader serving uploads as the handler serves them, and four routes:
/// channel 7 from the phone that opens uploads, 8 from a second phone, 9 from a
/// Callosum route with no device identity, and 10 from a principal other than
/// Callosum carrying a phone identity.
pub(super) struct Wire {
    connection: crate::app_reader::tests::Connection,
    routes: std::collections::HashMap<u16, RouteBinding>,
    tx: mpsc::Sender<Frame>,
    output: mpsc::Receiver<Frame>,
    corr: u64,
}

pub(super) const PHONE_ROUTE: u16 = 7;
pub(super) const OTHER_ROUTE: u16 = 8;
pub(super) const OPERATOR_ROUTE: u16 = 9;
pub(super) const FOREIGN_ROUTE: u16 = 10;

impl Wire {
    fn new(app: Arc<AppReader>) -> Self {
        let (tx, output) = mpsc::channel(64);
        Self {
            connection: crate::app_reader::tests::Connection::new(app),
            routes: std::collections::HashMap::new(),
            tx,
            output,
            corr: 100,
        }
    }

    async fn next(&mut self) -> Frame {
        tokio::time::timeout(std::time::Duration::from_secs(20), self.output.recv())
            .await
            .expect("the module answers")
            .expect("egress stays open")
    }

    pub async fn bind_all(&mut self) {
        for (channel, binding) in [
            (PHONE_ROUTE, device(PHONE)),
            (OTHER_ROUTE, device(OTHER_PHONE)),
            (OPERATOR_ROUTE, binding("callosum", "operator-console")),
            (
                FOREIGN_ROUTE,
                binding("broca", &format!("fed-phone:{PHONE}")),
            ),
        ] {
            self.routes.insert(channel, binding);
        }
    }

    /// Send `body` (framed when `binary`) on `channel` and return the reply.
    pub async fn raw(&mut self, channel: u16, body: Vec<u8>, binary: bool) -> Reply {
        self.corr += 1;
        let corr = self.corr;
        let binding = self.routes.get(&channel).cloned();
        self.connection
            .dispatch_body((channel, 1, corr), binding, body, binary, &self.tx);
        let reply = self.next().await;
        assert_eq!(reply.header.corr, corr);
        let body: Value = serde_json::from_slice(&reply.body).unwrap();
        match reply.header.ty {
            FrameType::Response => Reply::Result(body["result"].clone()),
            FrameType::Error => Reply::Refusal(body),
            other => panic!("unexpected {other:?}"),
        }
    }

    pub async fn call(&mut self, channel: u16, method: &str, params: Value) -> Reply {
        let body = json!({"method": method, "params": params})
            .to_string()
            .into_bytes();
        self.raw(channel, body, false).await
    }

    /// A framed `upload.chunk`: the exact bytes sent are returned with the reply.
    pub async fn chunk_on(
        &mut self,
        channel: u16,
        upload_id: &str,
        offset: u64,
        payload: &[u8],
    ) -> (Vec<u8>, Reply) {
        let body = subc_protocol::framed_body::encode(
            &json!({"method": CHUNK, "params": {"upload_id": upload_id, "offset": offset}}),
            payload,
        )
        .unwrap();
        let reply = self.raw(channel, body.clone(), true).await;
        (body, reply)
    }

    pub async fn chunk(&mut self, upload_id: &str, offset: u64, payload: &[u8]) -> Reply {
        self.chunk_on(PHONE_ROUTE, upload_id, offset, payload)
            .await
            .1
    }

    pub async fn begin(&mut self, name: &str, content: &[u8]) -> String {
        self.call(PHONE_ROUTE, BEGIN, begin_params(name, content))
            .await
            .ok()["upload_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    pub async fn id_call(&mut self, method: &str, upload_id: &str) -> Reply {
        self.call(PHONE_ROUTE, method, json!({"upload_id": upload_id}))
            .await
    }

    /// Begin, send every piece, and finish: the finish's reply.
    pub async fn upload(&mut self, name: &str, content: &[u8]) -> Value {
        let id = self.begin(name, content).await;
        for (index, piece) in content.chunks(1024).enumerate() {
            self.chunk(&id, (index * 1024) as u64, piece).await.ok();
        }
        self.id_call(FINISH, &id).await.ok()
    }
}

pub(super) fn sha(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

pub(super) fn begin_params(name: &str, content: &[u8]) -> Value {
    json!({"session": SESSION, "name": name, "mime": "application/octet-stream",
        "size": content.len(), "sha256": sha(content)})
}

/// Deterministic content of `len` bytes that differs at every offset modulo 251.
pub(super) fn content(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}
