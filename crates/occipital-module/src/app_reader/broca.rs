//! Owner-free Broca reads and one lingering feed shared by transcript/display watches.

use crate::app_reader::Guard as _;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};

use async_trait::async_trait;
use cortexkit_role_llm_runner::{
    archive_prunes_reasoning::PrunedReasoning,
    describe::{check_describe, RoleDescribe},
    display::DisplayPage,
    history::{HistoryForm, HistoryRequest},
    read::{ReadRequest, SenderGrant},
};
use futures_util::future::{BoxFuture, FutureExt, Shared};
use occipital_app_reader::{
    broca_entries,
    display_events::{self, DisplayInput},
    source::{
        Anchor, Attachments, Head, HistoryPage, InputCapability, Positioned, ReadError, Row,
        SendGrant, TranscriptSource,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use subc_client_rs::consumer::{
    CallError, CallOptions, ConsumerOptions, SubcConsumer, SubscribeOptions, Subscription,
};
use subc_client_rs::ConsumerIdentity;
use subc_protocol::RouteTarget;
use tokio::sync::{broadcast, watch};

use crate::display::call_pins::CallPinSlot;

use super::{
    display_hub::DisplayHub,
    resolve::{
        served_harness, DaemonEntorhinal, Resolved, Resolver, SessionRef, BROCA_RUNNER_MODULE_ID,
    },
    send::{RunnerError, SendTarget, SendTargets},
    SourceResolver, SourceWatch,
};

#[async_trait]
pub trait SourceConnector: Send + Sync {
    async fn connect(&self) -> Result<Arc<BrocaSources>, ReadError>;
}

type ConnectAttempt = Shared<BoxFuture<'static, Result<Arc<BrocaSources>, ReadError>>>;

#[derive(Default)]
struct LazyState {
    next: u64,
    attempt: Option<(u64, ConnectAttempt)>,
    ready: Option<Arc<BrocaSources>>,
}

/// Open daemon client connections only when an app read or watch needs them.
/// Concurrent requests share an attempt; later requests retry after failure
/// without a module restart.
pub struct LazyBrocaSources {
    connector: Arc<dyn SourceConnector>,
    state: Mutex<LazyState>,
}

impl LazyBrocaSources {
    pub fn new(connector: Arc<dyn SourceConnector>) -> Self {
        Self {
            connector,
            state: Mutex::new(LazyState::default()),
        }
    }

    /// Reach the runners and Entorhinal through the daemon at `connection`, as
    /// `principal`. Pages report their calls' pins to `call_pins` once it is set.
    pub fn live(
        connection: std::path::PathBuf,
        principal: ConsumerIdentity,
        call_pins: CallPinSlot,
    ) -> Self {
        Self::new(Arc::new(LiveConnector {
            connection,
            principal,
            call_pins,
        }))
    }

    /// Refuse every read and send: a module started without a daemon connection,
    /// as in tests that serve only the tool provider.
    pub fn unavailable() -> Self {
        Self::new(Arc::new(Unavailable))
    }

    pub(crate) async fn connected(&self) -> Result<Arc<BrocaSources>, ReadError> {
        let (generation, attempt) = {
            let mut state = self.state.guard();
            if let Some(ready) = &state.ready {
                return Ok(ready.clone());
            }
            match &state.attempt {
                Some(attempt) => attempt.clone(),
                None => {
                    state.next = state.next.wrapping_add(1);
                    let connector = self.connector.clone();
                    let attempt = async move { connector.connect().await }.boxed().shared();
                    state.attempt = Some((state.next, attempt.clone()));
                    (state.next, attempt)
                }
            }
        };
        let result = attempt.await.map_err(|error| {
            ReadError::new(
                "provider_unavailable",
                format!("Broca reader connection unavailable: {}", error.message),
            )
        });
        let mut state = self.state.guard();
        if state
            .attempt
            .as_ref()
            .is_some_and(|(current, _)| *current == generation)
        {
            if let Ok(ready) = &result {
                state.ready = Some(ready.clone());
            }
            state.attempt = None;
        }
        result
    }
}

struct LiveConnector {
    connection: std::path::PathBuf,
    principal: ConsumerIdentity,
    call_pins: CallPinSlot,
}

#[async_trait]
impl SourceConnector for LiveConnector {
    async fn connect(&self) -> Result<Arc<BrocaSources>, ReadError> {
        BrocaSources::live(
            &self.connection,
            self.principal.clone(),
            subc_protocol::BindIdentity::new(crate::MODULE_ID, "app-reader"),
        )
        .await
        .map(|sources| Arc::new(sources.with_call_pins(self.call_pins.clone())))
    }
}

struct Unavailable;

#[async_trait]
impl SourceConnector for Unavailable {
    async fn connect(&self) -> Result<Arc<BrocaSources>, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "this module was started without a daemon connection for reads",
        ))
    }
}

#[async_trait]
impl SourceResolver for LazyBrocaSources {
    async fn validate_session(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<(), ReadError> {
        self.connected()
            .await?
            .validate_session(session, harness)
            .await
    }

    async fn resolve(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        SourceResolver::resolve(self.connected().await?.as_ref(), session, harness).await
    }

    async fn project_root(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<std::path::PathBuf, ReadError> {
        self.connected().await?.project_root(session, harness).await
    }

    async fn prompt_accepts(
        &self,
        session: &str,
        harness: Option<&str>,
        mime: &str,
    ) -> Result<bool, ReadError> {
        self.connected()
            .await?
            .prompt_accepts(session, harness, mime)
            .await
    }

    async fn watch(
        &self,
        session: &str,
        harness: Option<&str>,
        hub: Arc<DisplayHub>,
    ) -> Result<SourceWatch, ReadError> {
        self.connected().await?.watch(session, harness, hub).await
    }

    async fn reference(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<SessionRef, ReadError> {
        SourceResolver::reference(self.connected().await?.as_ref(), session, harness).await
    }
}

#[async_trait]
impl SendTargets for LazyBrocaSources {
    async fn validate(&self, session: &str, harness: Option<&str>) -> Result<(), ReadError> {
        self.connected()
            .await?
            .validate_session(session, harness)
            .await
    }

    async fn open(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<Arc<dyn SendTarget>, ReadError> {
        self.connected().await?.send_target(session, harness).await
    }
}

pub const SUBSCRIPTION_LINGER: Duration = Duration::from_secs(60);
const SOURCE_PAGE_LIMIT: u32 = 100;
/// A runner numbers a session's entries from 1, and every entry stays readable.
const FIRST_ORDINAL: u64 = 1;

/// Only observation operations can cross this adapter's route boundary. In
/// particular, attaching a reader never opens a session or acquires its owner.
#[derive(Debug, Clone, Copy)]
pub enum ReadOp {
    Head,
    Read,
    History,
    Status,
    /// Learn the runner's capability groups before interpreting optional metadata
    /// or asking for display history.
    Describe,
}

impl ReadOp {
    pub(super) fn method(self) -> &'static str {
        match self {
            Self::Head => "session.head",
            Self::Read => "session.read",
            Self::History => "session.history",
            Self::Status => "run.status",
            Self::Describe => "role.describe",
        }
    }
}

#[async_trait]
pub trait EventStream: Send {
    async fn next(&mut self) -> Result<Option<Value>, ReadError>;
}

#[async_trait]
pub trait BrocaRoute: Send + Sync {
    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError>;
    /// Dropping the returned stream cancels only the subscription, never the run.
    /// With `live_ids`, a message its host names only when storing it streams
    /// under a `live_id` until then; ask for it only from a runner declaring it.
    async fn subscribe(&self, live_ids: bool) -> Result<Box<dyn EventStream>, ReadError>;
}

#[async_trait]
pub trait BrocaRoutes: Send + Sync {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError>;

    /// The session's send path, for the apps' send. Kept apart from [`BrocaRoute`],
    /// whose calls are observation only.
    fn sender(&self, _resolved: &Resolved) -> Option<Arc<dyn SendTarget>> {
        None
    }
}

struct DaemonRoutes {
    calls: Arc<SubcConsumer>,
    streams: Arc<SubcConsumer>,
    principal: ConsumerIdentity,
}

#[async_trait]
impl BrocaRoutes for DaemonRoutes {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(Arc::new(DaemonRoute {
            calls: self.calls.clone(),
            streams: self.streams.clone(),
            principal: self.principal.clone(),
            resolved: resolved.clone(),
        }))
    }

    fn sender(&self, resolved: &Resolved) -> Option<Arc<dyn SendTarget>> {
        Some(Arc::new(DaemonSender {
            calls: self.calls.clone(),
            principal: self.principal.clone(),
            resolved: resolved.clone(),
        }))
    }
}

/// Calls into one runner session as this module, `reserved:occipital`. The runner
/// checks that attested route principal, never a request field, against the
/// senders the session's plan allows.
struct DaemonSender {
    calls: Arc<SubcConsumer>,
    principal: ConsumerIdentity,
    resolved: Resolved,
}

#[async_trait]
impl SendTarget for DaemonSender {
    fn reference(&self) -> &SessionRef {
        &self.resolved.reference
    }

    async fn call(&self, method: &'static str, params: Value) -> Result<Value, RunnerError> {
        let bytes = self
            .calls
            .call(
                target(&self.resolved),
                self.resolved.identity(),
                json!({"method": method, "params": params})
                    .to_string()
                    .into_bytes(),
                CallOptions {
                    consumer_identity: Some(self.principal.clone()),
                    role_versions: versions(),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| match error {
                CallError::Module(body) => RunnerError::Refused {
                    code: body.code,
                    message: body.message,
                    detail: body.detail,
                },
                other => {
                    RunnerError::Unreachable(format!("{}: {other}", self.resolved.reference.runner))
                }
            })?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|error| RunnerError::Refused {
                code: "invalid_provider_reply".into(),
                message: error.to_string(),
                detail: None,
            })?;
        runner_result(value)
    }
}

/// A runner reply is either a result or a refusal envelope `{code, message, detail?}`.
pub(crate) fn runner_result(value: Value) -> Result<Value, RunnerError> {
    if let Some(code) = value.get("code").and_then(Value::as_str) {
        return Err(RunnerError::Refused {
            code: code.into(),
            message: value["message"].as_str().unwrap_or_default().into(),
            detail: value
                .get("detail")
                .filter(|detail| !detail.is_null())
                .cloned(),
        });
    }
    Ok(value.get("result").cloned().unwrap_or(value))
}

struct DaemonRoute {
    calls: Arc<SubcConsumer>,
    streams: Arc<SubcConsumer>,
    principal: ConsumerIdentity,
    resolved: Resolved,
}

fn target(resolved: &Resolved) -> RouteTarget {
    RouteTarget::ManagementSurface {
        module_id: resolved.reference.runner.clone(),
    }
}

fn versions() -> Option<BTreeMap<String, String>> {
    Some(BTreeMap::from([("llm-runner".into(), "v1".into())]))
}

fn body(op: ReadOp, params: Value) -> Vec<u8> {
    json!({"method":op.method(), "params":params})
        .to_string()
        .into_bytes()
}

/// A failed call to `runner`, as the app sees it; the message names the runner.
fn call_error(runner: &str, error: CallError) -> ReadError {
    let code = match error.code() {
        Some("unknown_mid" | "unknown_ordinal" | "cursor_expired") => "cursor_expired",
        Some("session_unknown" | "unknown_session") => "session_unknown",
        _ => "provider_unavailable",
    };
    ReadError::new(code, format!("{runner}: {error}"))
}

/// A runner's reply: its result, or its refusal as the app sees it.
fn result(value: Value) -> Result<Value, ReadError> {
    if let Some(code) = value.get("code").and_then(Value::as_str) {
        return Err(ReadError::new(
            match code {
                "unknown_mid" | "unknown_ordinal" | "cursor_expired" => "cursor_expired",
                "session_unknown" | "unknown_session" => "session_unknown",
                _ => "provider_unavailable",
            },
            format!("{code}: {}", value["message"]),
        ));
    }
    Ok(value.get("result").cloned().unwrap_or(value))
}

#[async_trait]
impl BrocaRoute for DaemonRoute {
    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        let bytes = self
            .calls
            .call(
                target(&self.resolved),
                self.resolved.identity(),
                body(op, params),
                CallOptions {
                    consumer_identity: Some(self.principal.clone()),
                    role_versions: versions(),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| {
                if matches!(op, ReadOp::History) {
                    tracing::debug!(code = error.code().unwrap_or("provider_unavailable"), %error,
                        "Broca display history call failed");
                }
                call_error(&self.resolved.reference.runner, error)
            })?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
        if matches!(op, ReadOp::History) {
            if let Some(code) = value.get("code").and_then(Value::as_str) {
                tracing::debug!(code, "Broca display history refused");
            }
        }
        result(value).map_err(|mut error| {
            error.message = format!("{} {}", self.resolved.reference.runner, error.message);
            error
        })
    }

    async fn subscribe(&self, live_ids: bool) -> Result<Box<dyn EventStream>, ReadError> {
        let subscription = self
            .streams
            .subscribe(
                target(&self.resolved),
                self.resolved.identity(),
                json!({"method":"session.subscribe", "params":subscribe_params(live_ids)})
                    .to_string()
                    .into_bytes(),
                SubscribeOptions {
                    consumer_identity: Some(self.principal.clone()),
                    role_versions: versions(),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| call_error(&self.resolved.reference.runner, error))?;
        Ok(Box::new(DaemonEvents(
            subscription,
            self.resolved.reference.runner.clone(),
        )))
    }
}

/// `session.subscribe`'s params: from the live edge, and `live_ids` only when asked.
fn subscribe_params(live_ids: bool) -> Value {
    let mut params = json!({"from": "live"});
    if live_ids {
        params["live_ids"] = json!(true);
    }
    params
}

/// A runner's live event stream, and the runner's module id for its errors.
struct DaemonEvents(Subscription, String);

#[async_trait]
impl EventStream for DaemonEvents {
    async fn next(&mut self) -> Result<Option<Value>, ReadError> {
        match self.0.events().recv().await {
            Some(bytes) => decode_subscription_frame(&bytes).map(Some),
            None => {
                let end = self.0.ended().await;
                Err(ReadError::new(
                    "provider_unavailable",
                    format!("{} subscription ended: {end:?}", self.1),
                ))
            }
        }
    }
}

fn decode_subscription_frame(bytes: &[u8]) -> Result<Value, ReadError> {
    serde_json::from_slice(bytes).map_err(|error| invalid(error.to_string()))
}

/// The serving layer keeps this registry for its lifetime. Repeated resolutions
/// share a source (and hence a feed) only while the bind identity is unchanged.
pub struct BrocaSources {
    resolver: Arc<Resolver>,
    routes: Arc<dyn BrocaRoutes>,
    sources: tokio::sync::Mutex<HashMap<SessionRef, SourceSlot>>,
    describes: Mutex<HashMap<SessionRef, DescribeSlot>>,
    call_pins: CallPinSlot,
}

type SourceSlot = Arc<tokio::sync::Mutex<Weak<BrocaSource>>>;

/// A session's successfully checked `role.describe` answer, cached and shared by
/// every source built for the session until its route is lost or rebound. Two
/// reads that miss the cache at once may each ask; either answer is kept. A plain read builds a fresh
/// source each time, so the cache lives here rather than in the source.
type DescribeSlot = Arc<Mutex<Option<Arc<RoleDescribe>>>>;

impl BrocaSources {
    pub fn new(resolver: Arc<Resolver>, routes: Arc<dyn BrocaRoutes>) -> Self {
        Self {
            resolver,
            routes,
            sources: tokio::sync::Mutex::new(HashMap::new()),
            describes: Mutex::new(HashMap::new()),
            call_pins: CallPinSlot::default(),
        }
    }

    /// Report each built page's call pins to `call_pins` once it is set.
    pub fn with_call_pins(mut self, call_pins: CallPinSlot) -> Self {
        self.call_pins = call_pins;
        self
    }

    pub async fn live(
        connection: &Path,
        principal: ConsumerIdentity,
        lookup_identity: subc_protocol::BindIdentity,
    ) -> Result<Self, ReadError> {
        let consumer = Arc::new(
            SubcConsumer::connect(connection, ConsumerOptions::default())
                .await
                .map_err(|error| ReadError::new("provider_unavailable", error.to_string()))?,
        );
        let resolver = Arc::new(Resolver::new(Arc::new(DaemonEntorhinal {
            consumer,
            identity: lookup_identity,
            principal: principal.clone(),
        })));
        Self::connect(connection, principal, resolver).await
    }
    pub async fn connect(
        connection: &Path,
        principal: ConsumerIdentity,
        resolver: Arc<Resolver>,
    ) -> Result<Self, ReadError> {
        let connect = || SubcConsumer::connect(connection, ConsumerOptions::default());
        let calls = Arc::new(
            connect()
                .await
                .map_err(|error| ReadError::new("provider_unavailable", error.to_string()))?,
        );
        // A held request consumes a flow-control permit; keep ordinary reads on a separate connection.
        let streams = Arc::new(
            connect()
                .await
                .map_err(|error| ReadError::new("provider_unavailable", error.to_string()))?,
        );
        Ok(Self::new(
            resolver,
            Arc::new(DaemonRoutes {
                calls,
                streams,
                principal,
            }),
        ))
    }

    /// Resolve the app's session the way a read does, then open its send path.
    /// The send path into the session the app named, routed as reads are: to the
    /// runner that hosts it, and any other runner's session refused `session_unknown`
    /// naming that runner.
    pub async fn send_target(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<Arc<dyn SendTarget>, ReadError> {
        let reference = self.reference(session, harness).await?;
        let resolved = self.resolver.resolve(&reference).await?;
        self.routes
            .sender(&resolved)
            .ok_or_else(|| ReadError::new("provider_unavailable", "no send path is connected"))
    }

    pub async fn source(&self, reference: &SessionRef) -> Result<Arc<BrocaSource>, ReadError> {
        let slot = {
            let mut sources = self.sources.lock().await;
            sources.retain(|_, slot| {
                Arc::strong_count(slot) > 1
                    || slot
                        .try_lock()
                        .map_or(true, |source| source.strong_count() != 0)
            });
            sources
                .entry(reference.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(Weak::new())))
                .clone()
        };
        // Only resolutions for this session wait for its network round trip.
        let mut slot = super::observe::step("source_slot", slot.lock()).await;
        if let Some(source) = slot.upgrade() {
            match source.refresh_binding().await {
                Ok(()) => return Ok(source),
                Err(error) if error.code == "session_binding_changed" => source.retire().await,
                Err(error) => return Err(error),
            }
        }
        let resolved = self.resolver.resolve(reference).await?;
        let route = self.routes.open(&resolved).await?;
        let sendable = self.routes.sender(&resolved).is_some();
        let describe = self
            .describes
            .guard()
            .entry(reference.clone())
            .or_default()
            .clone();
        let source = BrocaSource::with_send_path(resolved, route, sendable, describe);
        *source.resolver.guard() = Some(self.resolver.clone());
        *source.call_pins.guard() = self.call_pins.clone();
        *slot = Arc::downgrade(&source);
        Ok(source)
    }
}

impl BrocaSources {
    /// The session the phone named, under the runner that serves it. A session named
    /// without a harness, or as Broca's, is a Broca session. Any other harness is routed
    /// by the runner Entorhinal records for the session: Thalamus's OpenCode and Pi
    /// runners are read like Broca, and any other runner, or none, is refused
    /// `session_unknown` naming it (null for none), so the phone reads the runner again.
    async fn reference(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<SessionRef, ReadError> {
        let Some(harness) = harness.filter(|harness| *harness != "broca") else {
            return Ok(SessionRef {
                runner: BROCA_RUNNER_MODULE_ID.into(),
                session: session.into(),
            });
        };
        match self.resolver.runner_by_harness(harness, session).await? {
            Some(runner)
                if runner != BROCA_RUNNER_MODULE_ID && served_harness(&runner).is_some() =>
            {
                Ok(SessionRef {
                    runner,
                    session: session.into(),
                })
            }
            runner => Err(ReadError::new(
                "session_unknown",
                format!(
                    "session {session} ({harness}) is not hosted by a runner this reader serves"
                ),
            )
            .with_detail(json!({"runner": runner}))),
        }
    }
}

#[async_trait]
impl SourceResolver for BrocaSources {
    async fn validate_session(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<(), ReadError> {
        let reference = self.reference(session, harness).await?;
        self.resolver.resolve(&reference).await?;
        Ok(())
    }

    async fn project_root(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<std::path::PathBuf, ReadError> {
        if let Some(harness) = harness.filter(|harness| *harness != "broca") {
            return self
                .resolver
                .project_root_by_harness(harness, session)
                .await;
        }
        Ok(self
            .resolver
            .resolve(&SessionRef {
                runner: BROCA_RUNNER_MODULE_ID.into(),
                session: session.into(),
            })
            .await?
            .project_root)
    }

    async fn resolve(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        let reference = self.reference(session, harness).await?;
        Ok(Some(self.source(&reference).await?))
    }

    async fn prompt_accepts(
        &self,
        session: &str,
        harness: Option<&str>,
        mime: &str,
    ) -> Result<bool, ReadError> {
        let reference = self.reference(session, harness).await?;
        let source = self.source(&reference).await?;
        // A runner without the group would ignore the send's `content` and drop
        // the file silently, so its description must name the group.
        if !source
            .describe()
            .await
            .is_some_and(|describe| describe.declares("attachments"))
        {
            return Ok(false);
        }
        Ok(source.head().await?.input.accepts(mime) != Some(false))
    }

    async fn watch(
        &self,
        session: &str,
        harness: Option<&str>,
        hub: Arc<DisplayHub>,
    ) -> Result<SourceWatch, ReadError> {
        let reference = self.reference(session, harness).await?;
        let source = self.source(&reference).await?;
        let lease = source.acquire_inner(Some((session, &hub))).await?;
        let live = Some(source.feed_state());
        Ok(SourceWatch {
            source: Some(source),
            lease: Some(Box::new(lease)),
            live,
        })
    }

    async fn reference(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<SessionRef, ReadError> {
        BrocaSources::reference(self, session, harness).await
    }
}

struct Feed {
    users: watch::Sender<usize>,
    task: tokio::task::JoinHandle<()>,
}

pub struct BrocaSource {
    resolved: Mutex<Resolved>,
    resolver: Mutex<Option<Arc<Resolver>>>,
    binding_refresh: tokio::sync::Mutex<()>,
    retired: AtomicBool,
    route: Arc<dyn BrocaRoute>,
    changes: watch::Sender<u64>,
    display: broadcast::Sender<DisplayInput>,
    display_targets: Mutex<Vec<(String, Weak<DisplayHub>)>>,
    live: watch::Sender<bool>,
    activity: Mutex<Option<(&'static str, i64)>>,
    /// The rich UI stamp of the run the last head read found still running, in the
    /// phone's shape. The feed subscribes from the live tail, so a run already going
    /// when it attaches never replays its `run_started`; this seeds its stamp.
    running_rich_ui: Mutex<Option<Value>>,
    mids: Mutex<HashSet<String>>,
    feed: tokio::sync::Mutex<Option<Feed>>,
    feed_live: AtomicBool,
    /// Whether this module has a send path to this session. Without one, the apps'
    /// send refuses it, so replies say the app may not send whatever the runner grants.
    sendable: bool,
    describe: DescribeSlot,
    /// Where the pins of the calls this source's pages carry are kept.
    call_pins: Mutex<CallPinSlot>,
}

impl BrocaSource {
    /// The phone's rows for stored entries, by the session's message shape.
    fn rows(
        &self,
        entries: &[Value],
        full: bool,
        pruned_reasoning: &[PrunedReasoning],
    ) -> Result<Vec<Row>, ReadError> {
        let profile = match self.harness() {
            "opencode2" => Some(broca_entries::DisplayProfile::OpenCode2),
            "pi" => Some(broca_entries::DisplayProfile::Pi),
            _ => None,
        };
        let rows = match profile {
            Some(profile) => broca_entries::display_rows(entries, full, profile, pruned_reasoning),
            None => broca_entries::rows_with_pruned_reasoning(entries, full, pruned_reasoning),
        }?;
        let call_pins = self.call_pins.guard().clone();
        if let Some(call_pins) = call_pins.get() {
            let calls: Vec<(String, String)> = rows
                .iter()
                .flat_map(|row| occipital_app_reader::tool_times::call_identities(&row.display))
                .collect();
            if !calls.is_empty() {
                let reference = self.resolved.guard().reference.clone();
                call_pins.seen(&reference.runner, &reference.session, calls);
            }
        }
        Ok(rows)
    }

    /// Construct a session-bound source; route implementations must use this resolved
    /// identity rather than accepting a second session in request bodies.
    pub fn new(resolved: Resolved, route: Arc<dyn BrocaRoute>) -> Arc<Self> {
        Self::with_send_path(resolved, route, true, DescribeSlot::default())
    }

    /// [`BrocaSource::new`], stating whether this module can send to the session, and
    /// where its runner's `role.describe` answer is kept.
    fn with_send_path(
        resolved: Resolved,
        route: Arc<dyn BrocaRoute>,
        sendable: bool,
        describe: DescribeSlot,
    ) -> Arc<Self> {
        Arc::new(Self {
            resolved: Mutex::new(resolved),
            resolver: Mutex::new(None),
            binding_refresh: tokio::sync::Mutex::new(()),
            retired: AtomicBool::new(false),
            route,
            changes: watch::channel(0).0,
            display: broadcast::channel(256).0,
            display_targets: Mutex::new(Vec::new()),
            live: watch::channel(false).0,
            activity: Mutex::new(None),
            running_rich_ui: Mutex::new(None),
            mids: Mutex::new(HashSet::new()),
            feed: tokio::sync::Mutex::new(None),
            feed_live: AtomicBool::new(false),
            sendable,
            describe,
            call_pins: Mutex::new(CallPinSlot::default()),
        })
    }

    pub fn feed_state(&self) -> watch::Receiver<bool> {
        self.live.subscribe()
    }

    pub fn network(&self) -> super::resolve::Network {
        self.resolved.guard().network
    }

    fn attach_display(&self, session: &str, hub: &Arc<DisplayHub>) {
        let mut targets = self.display_targets.guard();
        let weak = Arc::downgrade(hub);
        if !targets
            .iter()
            .any(|(name, target)| name == session && target.ptr_eq(&weak))
        {
            targets.push((session.into(), weak));
            if self.activity_feed_live() {
                hub.attach(session);
            }
        }
    }

    fn display_input(&self, input: DisplayInput) {
        let _ = self.display.send(input.clone());
        let mut targets = self.display_targets.guard();
        targets.retain(|(session, target)| {
            if let Some(hub) = target.upgrade() {
                hub.ingest(session, input.clone());
                true
            } else {
                false
            }
        });
    }

    pub async fn acquire(self: &Arc<Self>) -> Result<SubscriptionLease, ReadError> {
        self.acquire_inner(None).await
    }

    async fn acquire_inner(
        self: &Arc<Self>,
        display: Option<(&str, &Arc<DisplayHub>)>,
    ) -> Result<SubscriptionLease, ReadError> {
        let mut feed = super::observe::step("feed_slot", self.feed.lock()).await;
        if self.retired.load(Ordering::Acquire) {
            return Err(ReadError::new(
                "session_binding_changed",
                format!(
                    "Broca session {} has been retired",
                    self.resolved.guard().reference.session
                ),
            ));
        }
        let events = self.display.subscribe();
        if let Some(current) = feed
            .as_ref()
            .filter(|current| !current.task.is_finished() && self.feed_live.load(Ordering::Acquire))
        {
            if let Some((session, hub)) = display {
                self.attach_display(session, hub);
            }
            current.users.send_modify(|users| *users += 1);
            return Ok(SubscriptionLease {
                source: self.clone(),
                users: current.users.clone(),
                events,
            });
        }
        if let Some(old) = feed.take() {
            old.task.abort();
        }
        // A runner that names some messages only when storing them streams them
        // under a `live_id` to a subscriber that asks; one that isn't asked holds
        // their words until then.
        let live_ids = self.describe().await.is_some_and(|describe| {
            describe.declares(cortexkit_role_llm_runner::capabilities::LIVE_IDS)
        });
        let stream =
            super::observe::remote("broca", "session.subscribe", self.route.subscribe(live_ids))
                .await?;
        // Attach before reading so control events committed during the cold read
        // stay queued, rather than disappearing between the snapshot and attach.
        self.newest(SOURCE_PAGE_LIMIT).await?;
        if let Some((session, hub)) = display {
            self.attach_display(session, hub);
        }
        let (users, receiver) = watch::channel(1);
        self.feed_live.store(true, Ordering::Release);
        self.live.send_replace(true);
        self.bump();
        self.display_input(DisplayInput::Reset);
        // Events queued since the subscribe come after this, so a run that started or
        // ended after the cold read still sets or clears the stamp in order.
        let running = self.running_rich_ui.guard().clone();
        if let Some(rich_ui) = running {
            self.display_input(DisplayInput::RunStart {
                rich_ui: Some(rich_ui),
            });
        }
        let task = tokio::spawn(pump(self.clone(), stream, receiver));
        *feed = Some(Feed {
            users: users.clone(),
            task,
        });
        Ok(SubscriptionLease {
            source: self.clone(),
            users,
            events,
        })
    }

    fn bump(&self) {
        self.changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    async fn refresh_binding(&self) -> Result<(), ReadError> {
        // Serialize lookup and publication together; an older lookup completing late
        // must not overwrite a newer network policy.
        let _refresh = super::observe::step("binding_refresh", self.binding_refresh.lock()).await;
        let reference = self.resolved.guard().reference.clone();
        if self.retired.load(Ordering::Acquire) {
            return Err(ReadError::new(
                "session_binding_changed",
                format!("Broca session {} changed bind identity", reference.session),
            ));
        }
        let resolver = self.resolver.guard().clone();
        if let Some(resolver) = resolver {
            let current = resolver.resolve(&reference).await?;
            let mut old = self.resolved.guard();
            if old.project_root != current.project_root || old.harness != current.harness {
                drop(old);
                self.retired.store(true, Ordering::Release);
                self.lost();
                return Err(ReadError::new(
                    "session_binding_changed",
                    format!("Broca session {} changed bind identity", reference.session),
                ));
            }
            // A watch can outlive a network-plan change. Refresh its recorded policy
            // and invalidate cache tokens even when Broca's transcript did not change.
            if old.network != current.network || old.network_history != current.network_history {
                self.bump();
            }
            *old = current;
        }
        Ok(())
    }

    async fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        let feed = self.feed.lock().await.take();
        if let Some(feed) = feed {
            feed.task.abort();
            let _ = feed.task.await;
        }
        self.lost();
    }

    fn lost(&self) {
        // The route this answer came from is gone; its replacement may describe
        // itself differently.
        *self.describe.guard() = None;
        self.feed_live.store(false, Ordering::Release);
        self.live.send_replace(false);
        *self.activity.guard() = None;
        self.display_input(DisplayInput::Reset);
        for (session, target) in self.display_targets.guard().drain(..) {
            if let Some(hub) = target.upgrade() {
                hub.remove(&session);
            }
        }
        self.bump();
    }

    async fn remote_read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        super::observe::remote("broca", op.method(), self.route.read(op, params)).await
    }

    async fn read_head(&self) -> Result<Head, ReadError> {
        let value = self.remote_read(ReadOp::Head, json!({})).await?;
        let meta = head_meta(&value)?;
        // Only a written session has a durable transcript update time.
        let at = meta.updated_at.and_then(|at| i64::try_from(at).ok());
        *self.activity.guard() = at.and_then(|at| {
            meta.last_run_state
                .as_ref()
                .and_then(|run| activity_for(Some(&run.state)))
                .map(|activity| (activity, at))
        });
        *self.running_rich_ui.guard() = meta
            .last_run_rich_ui
            .as_ref()
            .filter(|_| {
                meta.last_run_state
                    .as_ref()
                    .is_some_and(|run| activity_for(Some(&run.state)) == Some("busy"))
            })
            .and_then(|stamp| serde_json::to_value(stamp).ok())
            .as_ref()
            .and_then(occipital_app_reader::broca_entries::rich_ui);
        // The contract's head is a durable snapshot, including the last run and
        // transcript update time. Include rendering policy as well: changing it
        // changes served rows even when the transcript itself is untouched.
        let activity_token = {
            let resolved = self.resolved.guard();
            let policy: Vec<_> = resolved
                .network_history
                .iter()
                .map(|change| json!([change.since_ms, change.network]))
                .collect();
            let facts = json!([meta, resolved.reference.session, resolved.network, policy]);
            Some(format!(
                "broca-activity-{:x}",
                Sha256::digest(facts.to_string().as_bytes())
            ))
        };
        // For send grants, only a head without `sender` needs the capability
        // declaration. Display history consults the same cached declaration.
        let describe = if self.sendable && meta.sender.is_none() {
            self.describe().await
        } else {
            None
        };
        let sender = send_grant(&meta, describe.as_deref(), !self.sendable);
        Ok(Head {
            // Every entry stays readable from ordinal 1. The head's own
            // `first_ordinal` is where the newest compaction marker starts what the
            // model sees, and doesn't bound what the apps may read.
            first_ordinal: meta.last_ordinal.map(|_| FIRST_ORDINAL),
            last_ordinal: meta.last_ordinal,
            history_version: Some(meta.history_version),
            incarnation: Some(meta.incarnation.clone()),
            activity_token,
            input: InputCapability {
                attachments: match value["input"]["attachments"].as_str() {
                    Some("yes") => Attachments::Yes,
                    Some("no") => Attachments::No,
                    _ => Attachments::Unknown,
                },
                mime_types: strings(&value["input"]["mime_types"]),
                modalities: strings(&value["input"]["modalities"]),
            },
            model: meta.model.as_ref().map(json_of).transpose()?,
            // Counts past 2^53 - 1 can't reach the phone exactly, so such a total is
            // left out as unknown rather than shown rounded.
            usage: meta
                .usage
                .as_ref()
                .filter(|usage| usage.consistent())
                .map(json_of)
                .transpose()?,
            sender,
            last_run: meta.last_run_state,
            last_run_error: meta.last_run_error,
        })
    }

    /// The runner's checked `role.describe`, cached. A failed or malformed answer
    /// isn't kept, so the next read asks again; until then it reads as `None`.
    async fn describe(&self) -> Option<Arc<RoleDescribe>> {
        if let Some(describe) = self.describe.guard().clone() {
            return Some(describe);
        }
        let value = self.remote_read(ReadOp::Describe, json!({})).await.ok()?;
        let describe = Arc::new(check_describe(&value).ok()?);
        *self.describe.guard() = Some(describe.clone());
        Some(describe)
    }

    async fn echo_client_message_ids(&self, entries: &mut [Value]) {
        // Where each runner's stored message names a person's message: Broca's at
        // `role`, OpenCode 2's at `type`, and a Pi journal entry's inside its
        // wrapped `message`.
        let is_user = |entry: &Value| {
            let message = &entry["message"];
            message["role"] == "user"
                || message["type"] == "user"
                || message["message"]["role"] == "user"
        };
        if !entries.iter().any(is_user)
            // Broca refuses the entire display range if even one write time is missing.
            || !entries.iter().all(|entry| entry["written_at"].as_u64().is_some())
        {
            return;
        }
        if !self
            .describe()
            .await
            .is_some_and(|describe| describe.declares("history_display"))
        {
            return;
        }
        let Some(from) = entries.first().and_then(|entry| entry["ordinal"].as_u64()) else {
            tracing::debug!("Broca display history range has no first ordinal");
            return;
        };
        let request = HistoryRequest::from_ordinal(from)
            .with_limit(entries.len() as u64)
            .with_form(HistoryForm::Display);
        let Ok(request) = json_of(&request) else {
            return;
        };
        // Use the same route and call deadline as session.read, with no retry:
        // reconciliation metadata must never make the raw transcript unavailable.
        let page = match self
            .remote_read(ReadOp::History, request)
            .await
            .and_then(|value| {
                serde_json::from_value::<DisplayPage>(value)
                    .map_err(|error| invalid(error.to_string()))
            }) {
            Ok(page) => page,
            Err(error) => {
                tracing::debug!(code = error.code, message = %error.message,
                    "Broca display history unavailable; keeping raw page");
                return;
            }
        };
        for history_entry in page.messages {
            if history_entry.row.role != "user" {
                continue;
            }
            let entry = entries.iter_mut().find(|entry| {
                entry["ordinal"].as_u64() == Some(history_entry.ordinal)
                    && entry["mid"].as_str() == Some(history_entry.mid.as_str())
            });
            let Some(entry) = entry else {
                tracing::debug!(ordinal = ?history_entry.ordinal, mid = %history_entry.mid,
                    "Broca display history row does not match raw page");
                continue;
            };
            if is_user(entry) {
                if let Some(id) = history_entry.row.client_message_id {
                    // Reuse the raw projection's clientMessageId member without
                    // replacing any content, ordering or timestamps from session.read.
                    entry["client_message_id"] = json!(id);
                }
            }
        }
    }

    async fn history_page(&self, request: HistoryRequest) -> Result<RawPage, ReadError> {
        let value = self
            .remote_read(
                ReadOp::History,
                json_of(&request.with_max_bytes(2_097_152))?,
            )
            .await?;
        serde_json::from_value(value).map_err(|error| invalid(error.to_string()))
    }

    /// Project entries read from the history `head` names, which the caller has
    /// checked page by page, once a second head read shows that history unchanged.
    async fn project_window(
        &self,
        head: Head,
        entries: Vec<Value>,
        pruned_reasoning: Vec<PrunedReasoning>,
        older: bool,
        newer: bool,
    ) -> Result<Snapshot, ReadError> {
        let after = self.read_head().await?;
        if after.history_version != head.history_version || after.incarnation != head.incarnation {
            return Err(unstable());
        }
        let mut rows = super::observe::synchronous("projection", || {
            self.rows(&entries, false, &pruned_reasoning)
        })?;
        // The apps draw every system row as a prompt card that opens on tap, so
        // one with nothing in it, such as the entry Pi writes when a session
        // starts, is left out of pages. The rows kept keep their ordinals, and
        // whether the page reached the start comes from the runner's page.
        rows.retain(|row| !is_empty_system_row(&row.display));
        let mut seen = HashSet::new();
        let mut last = None;
        let mut numbered = false;
        for row in &rows {
            if !seen.insert(row.mid.clone())
                || (row.ordinal.is_none() && numbered)
                || row
                    .ordinal
                    .is_some_and(|ordinal| last.is_some_and(|previous| ordinal <= previous))
            {
                return Err(invalid(
                    "Broca session.read returned overlapping or unordered identities",
                ));
            }
            if let Some(ordinal) = row.ordinal {
                last = Some(ordinal);
                numbered = true;
            }
        }
        self.mids.guard().extend(seen);
        let resolved = self.resolved.guard();
        for row in &mut rows {
            row.display["network"] = json!(resolved.network_at(row.written_at));
        }
        Ok(Snapshot {
            rows,
            head,
            entries,
            pruned_reasoning,
            older,
            newer,
        })
    }

    async fn numbered_window(
        &self,
        head: Head,
        from: u64,
        end: u64,
    ) -> Result<Snapshot, ReadError> {
        let mut next = from;
        let mut entries = Vec::new();
        let mut pruned_reasoning = Vec::new();
        let older = from > FIRST_ORDINAL;
        // A runner declaring `rich_ui_stamp` adds each stamped assistant entry's stamp
        // to the same read when asked, and one declaring `read_display_facts` adds
        // each entry's client message id and its calls' times. Any other runner
        // refuses either field, so each is asked for only where declared.
        let describe = self.describe().await;
        let stamped = describe
            .as_ref()
            .is_some_and(|describe| describe.declares("rich_ui_stamp"));
        let display_facts = describe
            .as_ref()
            .is_some_and(|describe| describe.declares("read_display_facts"));
        while next < end {
            let request = ReadRequest::range(next)
                .with_limit((end - next).min(u64::from(SOURCE_PAGE_LIMIT)))
                .with_max_bytes(2_097_152)
                .with_include_rich_ui(stamped)
                .with_include_display_facts(display_facts);
            let value = self.remote_read(ReadOp::Read, json_of(&request)?).await?;
            let mut page: RawPage =
                serde_json::from_value(value).map_err(|error| invalid(error.to_string()))?;
            if !page.agrees_with(&head) {
                return Err(unstable());
            }
            // The requested ordinal window, not total transcript length, bounds the work.
            if page.next_from_ordinal.is_some_and(|cursor| cursor <= next) {
                return Err(invalid("Broca session.read repeated its cursor"));
            }
            page.messages.retain(|entry| {
                entry["ordinal"]
                    .as_u64()
                    .is_some_and(|ordinal| ordinal >= next && ordinal >= from && ordinal < end)
            });
            // With display facts the read already carries the client ids, so no
            // display-history read is needed.
            if !display_facts {
                self.echo_client_message_ids(&mut page.messages).await;
            }
            entries.extend(page.messages);
            pruned_reasoning.extend(page.pruned_reasoning);
            let Some(cursor) = page.next_from_ordinal else {
                break;
            };
            next = cursor;
        }
        let newer = head.last_ordinal.is_some_and(|last| end <= last);
        self.project_window(head, entries, pruned_reasoning, older, newer)
            .await
    }

    async fn window_head(&self) -> Result<Head, ReadError> {
        self.refresh_binding().await?;
        self.read_head().await
    }

    /// A window with no entries, for a read that reaches before the first entry or
    /// into an empty transcript.
    fn empty_window(head: Head) -> Snapshot {
        Snapshot {
            rows: Vec::new(),
            head,
            entries: Vec::new(),
            pruned_reasoning: Vec::new(),
            older: false,
            newer: false,
        }
    }

    async fn anchor_entry(&self, head: &Head, anchor: &Anchor) -> Result<Option<Value>, ReadError> {
        let mid = match anchor {
            Anchor::Mid(mid) => mid.clone(),
            Anchor::Cursor(cursor) => {
                let cursor: Cursor = serde_json::from_str(cursor).map_err(|_| expired())?;
                if Some(cursor.history_version) != head.history_version
                    || Some(&cursor.incarnation) != head.incarnation.as_ref()
                {
                    return Err(expired());
                }
                cursor.mid
            }
            _ => return Ok(None),
        };
        let page = self
            .history_page(HistoryRequest::from_mid(&mid).with_limit(1))
            .await?;
        if !page.agrees_with(head) {
            return Err(unstable());
        }
        let older = page.older.is_some();
        let newer = page.newer.is_some();
        let snapshot = self
            .project_window(
                head.clone(),
                page.messages,
                page.pruned_reasoning,
                older,
                newer,
            )
            .await?;
        snapshot
            .entries
            .into_iter()
            .next()
            .filter(|entry| entry["mid"] == mid)
            .map(Some)
            .ok_or_else(expired)
    }

    fn observe(&self, envelope: Value) -> bool {
        if self.retired.load(Ordering::Acquire) {
            return false;
        }
        // A frame that carries no live event, such as the OpenCode runner's send status
        // (`runner_event: "send_accepted"`) or a run's held-call status (`unit: {type:
        // status}`), is skipped and keeps the feed; a control one still wakes the
        // transcript. A frame whose event is malformed still ends the feed.
        if envelope.is_object() && !carries_live_event(&envelope) {
            if envelope["kind"] == "control" {
                self.bump();
            }
            return true;
        }
        let Some(mut normalized) = normalize(envelope) else {
            return false;
        };
        let event = &mut normalized["event"];
        let name = event["type"].as_str().unwrap_or_default().to_string();
        // A message its host names only when storing it streams under the runner's
        // `live_id` until then (`live_ids`), which `display_events::map` keys on.
        let unstored =
            event.get("mid").is_none() && event.get("live_id").is_some_and(Value::is_string);
        if matches!(name.as_str(), "text_delta" | "reasoning_delta") && !unstored {
            // An explicit runner mid can precede its durable message, including on
            // attach replay. Only legacy provider ids need the known-transcript check;
            // never infer their mapping from message order or a run id.
            let id = event
                .get("mid")
                .or_else(|| event.get("message_id"))
                .or_else(|| event.get("messageId"))
                .or_else(|| event.get("messageID"))
                .and_then(Value::as_str);
            let matches_mid = event.get("mid").and_then(Value::as_str).is_some()
                || id.is_some_and(|id| self.mids.guard().contains(id));
            if !matches_mid {
                return true;
            }
            event["mid"] = json!(id);
            if event.get("delta").is_none() {
                event["delta"] = event["text"].clone();
            }
        }
        if normalized["kind"] == "control" {
            if matches!(name.as_str(), "run_started" | "run_finished" | "run_paused") {
                // A control event wakes the reader; only the next durable head
                // can supply a timestamp, never the wall clock of this observer.
                *self.activity.guard() = None;
            }
            self.bump();
        }
        if let Some(input) = display_events::map(&normalized) {
            self.display_input(input);
        }
        true
    }
}

impl Drop for BrocaSource {
    fn drop(&mut self) {
        if let Some(feed) = self.feed.get_mut().take() {
            feed.task.abort();
        }
    }
}

pub struct SubscriptionLease {
    source: Arc<BrocaSource>,
    users: watch::Sender<usize>,
    pub events: broadcast::Receiver<DisplayInput>,
}

impl SubscriptionLease {
    pub fn source(&self) -> &Arc<BrocaSource> {
        &self.source
    }
}

impl Drop for SubscriptionLease {
    fn drop(&mut self) {
        self.users.send_modify(|users| *users -= 1);
    }
}

async fn pump(
    source: Arc<BrocaSource>,
    mut stream: Box<dyn EventStream>,
    mut users: watch::Receiver<usize>,
) {
    let mut deadline = None;
    loop {
        let count = *users.borrow_and_update();
        if count == 0 {
            deadline.get_or_insert_with(|| tokio::time::Instant::now() + SUBSCRIPTION_LINGER);
        } else {
            deadline = None;
        }
        let linger_elapsed = tokio::select! {
            biased;
            changed = users.changed() => { if changed.is_ok() { continue; } else { false } }
            _ = async { match deadline { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => true,
            event = stream.next() => match event {
                Ok(Some(event)) => { if source.observe(event) { continue; } false },
                Ok(None) | Err(_) => false,
            }
        };
        // Serialize linger expiry with acquisition so a reconnect at the deadline
        // cannot receive a lease for a feed that is already being detached.
        let _feed = source.feed.lock().await;
        if linger_elapsed && *users.borrow() != 0 {
            continue;
        }
        source.lost();
        break;
    }
}

#[derive(Deserialize)]
struct RawPage {
    messages: Vec<Value>,
    #[serde(default)]
    pruned_reasoning: Vec<PrunedReasoning>,
    #[serde(default, alias = "nextFromOrdinal")]
    next_from_ordinal: Option<u64>,
    #[serde(default)]
    older: Option<String>,
    #[serde(default)]
    newer: Option<String>,
    history_version: u64,
    incarnation: String,
}

impl RawPage {
    /// Whether the page was read from the history `head` names: the same history
    /// counter and the same log.
    fn agrees_with(&self, head: &Head) -> bool {
        Some(self.history_version) == head.history_version
            && Some(&self.incarnation) == head.incarnation.as_ref()
    }
}

struct Snapshot {
    rows: Vec<Row>,
    head: Head,
    entries: Vec<Value>,
    pruned_reasoning: Vec<PrunedReasoning>,
    older: bool,
    newer: bool,
}

/// A cursor binds the history it was served from: the history counter, which a
/// rewrite raises, and the log's incarnation, which a compaction cut keeps and a
/// recreated log changes. A cursor that doesn't decode, such as one served before
/// runners numbered their entries from 1, reads as expired, and the app reloads.
#[derive(Deserialize, Serialize)]
struct Cursor {
    mid: String,
    after: bool,
    history_version: u64,
    incarnation: String,
}

impl Cursor {
    /// A cursor at `mid` in the history `head` names; `None` for a head without
    /// one, which no runner on this contract serves.
    fn at(mid: &str, after: bool, head: &Head) -> Option<String> {
        let cursor = Cursor {
            mid: mid.into(),
            after,
            history_version: head.history_version?,
            incarnation: head.incarnation.clone()?,
        };
        Some(encode_cursor(&cursor))
    }
}

/// A cursor as the phone holds it. Its plain fields always serialize; were that
/// ever to fail, the empty cursor reads back as expired and the phone reloads,
/// rather than the read panicking.
fn encode_cursor(cursor: &Cursor) -> String {
    serde_json::to_string(cursor).unwrap_or_default()
}

/// Whether a row is a system row with nothing to show: no text, and only text
/// parts that are empty. A system row with text, such as a Broca worker's
/// instructions, has something to show.
fn is_empty_system_row(display: &Value) -> bool {
    display["role"] == "system"
        && display["text"].as_str().unwrap_or_default().is_empty()
        && display["parts"].as_array().is_none_or(|parts| {
            parts.iter().all(|part| {
                part["type"] == "text" && part["text"].as_str().unwrap_or_default().is_empty()
            })
        })
}

impl Snapshot {
    fn page(&self, mut start: usize, end: usize) -> HistoryPage {
        if self.rows[start..end]
            .iter()
            .any(|row| row.ordinal.is_some())
        {
            // A history page must not mix unnumbered rows with numbered rows.
            start += self.rows[start..end]
                .iter()
                .position(|row| row.ordinal.is_some())
                .unwrap_or(0);
        }
        HistoryPage {
            rows: self.rows[start..end].to_vec(),
            older: ((start > 0 || self.older) && start < end).then(|| self.cursor(start, false)),
            newer: ((end < self.rows.len() || self.newer) && end > start)
                .then(|| self.cursor(end - 1, true)),
            history_version: self.head.history_version,
            incarnation: self.head.incarnation.clone(),
        }
    }

    fn cursor(&self, index: usize, after: bool) -> String {
        Cursor::at(&self.rows[index].mid, after, &self.head).unwrap_or_default()
    }
}

#[async_trait]
impl TranscriptSource for BrocaSource {
    fn harness(&self) -> &'static str {
        match self.resolved.guard().harness.as_str() {
            "opencode2" => "opencode2",
            "pi" => "pi",
            _ => "broca",
        }
    }
    async fn head(&self) -> Result<Head, ReadError> {
        self.refresh_binding().await?;
        self.read_head().await
    }
    async fn newest(&self, limit: u32) -> Result<HistoryPage, ReadError> {
        let head = self.window_head().await?;
        let snapshot = if let Some(last) = head.last_ordinal {
            let end = last.saturating_add(1);
            let from = end.saturating_sub(u64::from(limit)).max(FIRST_ORDINAL);
            self.numbered_window(head, from, end).await?
        } else {
            Self::empty_window(head)
        };
        Ok(snapshot.page(0, snapshot.rows.len()))
    }
    async fn older(&self, anchor: &Anchor, limit: u32) -> Result<HistoryPage, ReadError> {
        if *anchor == Anchor::Newest {
            return self.newest(limit).await;
        }
        let head = self.window_head().await?;
        let after = match anchor {
            Anchor::Cursor(cursor) => {
                serde_json::from_str::<Cursor>(cursor)
                    .map_err(|_| expired())?
                    .after
            }
            _ => false,
        };
        let entry = self.anchor_entry(&head, anchor).await?;
        let ordinal = match anchor {
            Anchor::Ordinal(ordinal) => Some(*ordinal),
            _ => entry.as_ref().and_then(|entry| entry["ordinal"].as_u64()),
        };
        // Every entry is numbered, so an anchor without an ordinal names no entry
        // of this history.
        let ordinal = ordinal.ok_or_else(expired)?;
        let snapshot = if !after && ordinal <= FIRST_ORDINAL {
            // Nothing is older than the first entry.
            Self::empty_window(head)
        } else {
            let Some(last) = head.last_ordinal.filter(|last| ordinal <= *last) else {
                return Err(expired());
            };
            let (from, end) = if after {
                let from = ordinal.saturating_add(1);
                (
                    from,
                    from.saturating_add(u64::from(limit))
                        .min(last.saturating_add(1)),
                )
            } else {
                (
                    ordinal.saturating_sub(u64::from(limit)).max(FIRST_ORDINAL),
                    ordinal,
                )
            };
            self.numbered_window(head, from, end).await?
        };
        let start = usize::from(
            after
                && entry
                    .as_ref()
                    // Skip the after cursor's anchor by mid, even if client_message_id was added to the fresh page.
                    .is_some_and(|entry| {
                        snapshot
                            .entries
                            .first()
                            .is_some_and(|first| first["mid"] == entry["mid"])
                    }),
        );
        Ok(snapshot.page(start, snapshot.rows.len()))
    }
    async fn newer(&self, from_mid: &str, limit: u32) -> Result<HistoryPage, ReadError> {
        let head = self.window_head().await?;
        let cursor = Cursor::at(from_mid, true, &head).ok_or_else(expired)?;
        self.older(&Anchor::Cursor(cursor), limit).await
    }
    async fn around(&self, mid: &str, limit: u32) -> Result<Option<Positioned>, ReadError> {
        // The message is found by its id through the runner's history, one row.
        if self.describe().await.is_some_and(|describe| {
            !describe.declares("host_history") && !describe.declares("history_display")
        }) {
            return Err(occipital_app_reader::source::unsupported("aroundMessageId"));
        }
        let head = self.window_head().await?;
        let entry = match self.anchor_entry(&head, &Anchor::Mid(mid.into())).await {
            Ok(entry) => entry,
            // The runner knows no such message, or the one it found isn't this one.
            Err(error) if matches!(error.code, "unknown_mid" | "cursor_expired") => None,
            Err(error) => return Err(error),
        };
        let (Some(ordinal), Some(last)) = (
            entry.and_then(|entry| entry["ordinal"].as_u64()),
            head.last_ordinal,
        ) else {
            return Ok(None);
        };
        // A message outside the numbered entries the head names is not in this
        // version of the transcript.
        if !(FIRST_ORDINAL..=last).contains(&ordinal) {
            return Ok(None);
        }
        let from = ordinal.saturating_sub(u64::from(limit)).max(FIRST_ORDINAL);
        let end = ordinal
            .saturating_add(u64::from(limit))
            .saturating_add(1)
            .min(last.saturating_add(1));
        let snapshot = self.numbered_window(head, from, end).await?;
        Ok(Some(Positioned {
            page: snapshot.page(0, snapshot.rows.len()),
            anchor: Some(ordinal),
            newest: Some(last),
        }))
    }
    async fn after(&self, ordinal: u64, limit: u32) -> Result<Positioned, ReadError> {
        let head = self.window_head().await?;
        let newest = head.last_ordinal;
        let from = ordinal.saturating_add(1).max(FIRST_ORDINAL);
        let Some(last) = newest.filter(|last| from <= *last && limit > 0) else {
            return Ok(Positioned {
                page: HistoryPage {
                    rows: Vec::new(),
                    older: None,
                    newer: None,
                    history_version: head.history_version,
                    incarnation: head.incarnation.clone(),
                },
                anchor: None,
                newest,
            });
        };
        let end = from
            .saturating_add(u64::from(limit))
            .min(last.saturating_add(1));
        let snapshot = self.numbered_window(head, from, end).await?;
        Ok(Positioned {
            page: snapshot.page(0, snapshot.rows.len()),
            anchor: None,
            newest,
        })
    }
    async fn exact(&self, anchor: &Anchor) -> Result<Option<Row>, ReadError> {
        let head = self.window_head().await?;
        let entry = self.anchor_entry(&head, anchor).await?;
        let ordinal = match anchor {
            Anchor::Ordinal(ordinal) => Some(*ordinal),
            _ => entry.as_ref().and_then(|entry| entry["ordinal"].as_u64()),
        };
        let Some(ordinal) = ordinal.filter(|ordinal| *ordinal >= FIRST_ORDINAL) else {
            return Ok(None);
        };
        let end = ordinal
            .saturating_add(u64::from(SOURCE_PAGE_LIMIT))
            .min(head.last_ordinal.unwrap_or(ordinal).saturating_add(1));
        let snapshot = self.numbered_window(head, ordinal, end).await?;
        if snapshot.entries.is_empty() {
            return Ok(None);
        }
        // Start result pairing at the selected message, as the full-message read
        // does, so an older unanswered call cannot consume an id-less result.
        let mut rows = self.rows(&snapshot.entries, true, &snapshot.pruned_reasoning)?;
        let mut row = rows.remove(0);
        if let Some(reason) = row
            .display
            .get("fullContentUnavailable")
            .and_then(Value::as_str)
        {
            return Err(ReadError::new("full_content_unavailable", reason));
        }
        let resolved = self.resolved.guard();
        row.display["network"] = json!(resolved.network_at(row.written_at));
        Ok(Some(row))
    }
    async fn numbered_claim_anchor(&self, mid: &str) -> bool {
        let request = HistoryRequest::from_mid(mid)
            .with_limit(1)
            .with_form(HistoryForm::Raw);
        let Ok(request) = json_of(&request) else {
            return false;
        };
        let Ok(value) = self.remote_read(ReadOp::History, request).await else {
            return false;
        };
        let Ok(page) = serde_json::from_value::<RawPage>(value) else {
            return false;
        };
        page.messages
            .first()
            .is_some_and(|entry| entry["mid"] == mid && entry["ordinal"].as_u64().is_some())
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    fn activity(&self) -> Option<(&'static str, i64)> {
        *self.activity.guard()
    }
    fn activity_feed_live(&self) -> bool {
        self.feed_live.load(Ordering::Acquire)
    }
}

/// The unit names older Broca builds sent in place of a typed live event, with the
/// event type each stands for.
const LEGACY_UNIT_EVENTS: [(&str, &str); 7] = [
    ("TextDelta", "text_delta"),
    ("ReasoningDelta", "reasoning_delta"),
    ("DisplayGap", "display_gap"),
    ("DisplayReset", "display_reset"),
    ("RunStarted", "run_started"),
    ("RunFinished", "run_finished"),
    ("RunPaused", "run_paused"),
];

/// Whether a subscription frame carries a live event: a typed `event`, or, from an older
/// Broca, a unit named for one. A unit of any other type (a run's held-call `status`,
/// say) is a durable record with no live event, so it never reaches the display.
fn carries_live_event(envelope: &Value) -> bool {
    if envelope.get("event").is_some() {
        return true;
    }
    let unit = &envelope["unit"];
    let name = unit
        .get("type")
        .or_else(|| unit.get("kind"))
        .and_then(Value::as_str);
    LEGACY_UNIT_EVENTS
        .iter()
        .any(|(legacy, _)| Some(*legacy) == name)
}

fn normalize(mut envelope: Value) -> Option<Value> {
    envelope.as_object()?;
    if !carries_live_event(&envelope) {
        return None;
    }
    if envelope.get("event").is_none() {
        envelope["event"] = envelope["unit"].clone();
    }
    let event = &mut envelope["event"];
    event.as_object()?;
    let name = event
        .get("type")
        .or_else(|| event.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let typed = LEGACY_UNIT_EVENTS
        .iter()
        .find(|(legacy, _)| *legacy == name)
        .map_or(name, |(_, typed)| typed)
        .to_owned();
    event["type"] = json!(typed);
    Some(envelope)
}

fn strings(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_string))
        .collect()
}

/// Decide what replies say about the app's send to this session. This is the one
/// place that decides it.
///
/// This module refusing sends to the session itself (`module_refuses`) wins: the
/// app may not send, whatever the runner grants. Otherwise the contract's
/// `HeadMeta::sender_grant` reads the head against the runner's `role.describe`.
/// This module reads the head as itself, and it's the sender the app's messages go
/// through, so the head's `sender` is the app's grant. A missing `sender` means no
/// grant only when the runner declares the `head_sender` group. Without the
/// group, an older runner's silence is unknown.
///
/// `describe` is `None` when the runner's `role.describe` couldn't be read. That
/// leaves no group to read, so a missing `sender` stays unknown, never "no grant".
pub(crate) fn send_grant(
    head: &cortexkit_role_llm_runner::read::HeadMeta,
    describe: Option<&RoleDescribe>,
    module_refuses: bool,
) -> SendGrant {
    if module_refuses {
        return SendGrant::MayNotSend;
    }
    let no_groups = RoleDescribe::default();
    match head.sender_grant(describe.unwrap_or(&no_groups)) {
        SenderGrant::Granted(sender) => match serde_json::to_value(sender) {
            Ok(Value::Object(sender)) => SendGrant::Granted(sender),
            _ => SendGrant::Unknown,
        },
        SenderGrant::NoGrant => SendGrant::MayNotSend,
        // `SenderGrant::Unknown`, and any outcome a later contract adds.
        _ => SendGrant::Unknown,
    }
}

/// A runner's `session.head`, decoded and checked. The contract rejects partial
/// snapshots, rather than filling in missing counters or times from a previous
/// read. That includes the log identity, which must fit its 1 to 128-byte bound.
fn head_meta(value: &Value) -> Result<cortexkit_role_llm_runner::read::HeadMeta, ReadError> {
    let meta: cortexkit_role_llm_runner::read::HeadMeta =
        serde_json::from_value(value.clone()).map_err(|error| invalid(error.to_string()))?;
    if !meta.snapshot_consistent() {
        return Err(invalid("Broca session.head has an inconsistent snapshot"));
    }
    Ok(meta)
}

fn activity_for(state: Option<&str>) -> Option<&'static str> {
    match state {
        Some("active" | "running" | "pending" | "queued" | "paused") => Some("busy"),
        Some("idle" | "completed" | "interrupted" | "error" | "cancelled") => Some("idle"),
        _ => None,
    }
}
fn invalid(message: impl Into<String>) -> ReadError {
    ReadError::new("invalid_provider_reply", message)
}
/// A runner-contract value as JSON. These types always serialize; a failure is
/// answered as an error rather than a panic all the same.
fn json_of(value: &impl Serialize) -> Result<Value, ReadError> {
    serde_json::to_value(value).map_err(|error| {
        ReadError::new("internal", format!("cannot encode a runner value: {error}"))
    })
}
fn unstable() -> ReadError {
    ReadError::new(
        "observation_unstable",
        "Broca history changed during session.read",
    )
}
fn expired() -> ReadError {
    ReadError::new("cursor_expired", "Broca transcript anchor no longer exists")
}

#[cfg(test)]
pub(crate) mod tests {
    mod held;
    mod held_status_1029;
    mod live_frames;
    mod media_1030;
    mod provider_fallback_1028;
    mod pruned_reasoning;
    mod rich_ui;
    mod rich_ui_stamp_1028;
    mod session_change_1030;
    mod subscribe_flatten_1028;
    mod tool_outage_1027;

    use super::super::resolve::{
        Entorhinal, Network, NetworkChange, OPENCODE_RUNNER_MODULE_ID, PI_RUNNER_MODULE_ID,
    };
    use super::*;
    use occipital_app_reader::source::TranscriptReader;
    use std::sync::atomic::{AtomicU64, AtomicUsize};
    use tokio::sync::mpsc;

    struct FakeRoute {
        entries: Mutex<Vec<Value>>,
        calls: Mutex<Vec<Value>>,
        senders: Mutex<Vec<mpsc::Sender<Result<Value, ReadError>>>>,
        attaches: AtomicUsize,
        /// Whether each subscribe asked for `live_ids`, in order.
        live_ids: Mutex<Vec<bool>>,
        detaches: Arc<AtomicUsize>,
        history: AtomicU64,
        /// The log identity on every head and page.
        incarnation: Mutex<String>,
        unavailable: AtomicBool,
        stuck: AtomicBool,
        updated_at: AtomicU64,
        run_state: Mutex<Value>,
        /// Members merged into every `session.head` answer, such as `model` or `incarnation`.
        head_extra: Mutex<serde_json::Map<String, Value>>,
        /// The `role.describe` answer, or `None` to refuse the call.
        describe: Mutex<Option<Value>>,
        history_replies: Mutex<HashMap<u64, Result<Value, ReadError>>>,
        anchor_replies: Mutex<HashMap<String, Result<Value, ReadError>>>,
        read_entries: AtomicUsize,
    }

    /// A `role.describe` answer that passes the contract's check, declaring `groups`.
    fn describe_with(groups: &[&str]) -> Value {
        use cortexkit_role_llm_runner::{describe::Major, ops, PROVIDES, REQUIRED_OPS};
        let mut served: Vec<String> = REQUIRED_OPS.iter().map(|op| op.to_string()).collect();
        served.push(ops::SESSION_HEAD.into());
        if groups.contains(&"history_display") {
            served.push(ops::SESSION_HISTORY.into());
        }
        if groups.contains(&"live_events") {
            served.push(ops::SESSION_SUBSCRIBE.into());
        }
        // Both groups add members to `session.read`, so a runner declaring either
        // must serve it.
        if (groups.contains(&"rich_ui_stamp") || groups.contains(&"read_display_facts"))
            && !served.iter().any(|op| op == ops::SESSION_READ)
        {
            served.push(ops::SESSION_READ.into());
        }
        serde_json::to_value(
            RoleDescribe::new(vec![Major::new(PROVIDES, served, "alpha")], "fake-broca")
                .with_capabilities(groups.iter().map(|group| group.to_string()).collect())
                .with_session_capabilities_from("baseline"),
        )
        .unwrap()
    }

    /// A runner reply recorded under runner contract 0.7, as a runner on the current
    /// contract serves the same session. Entries are now numbered from 1, so every
    /// ordinal is one higher; the log is named by `incarnation`, which takes the
    /// recorded `lineage_id` when the reply names none, and a reply recorded
    /// before runners kept a history counter is at counter 0, since its history
    /// was never rewritten; and a description that
    /// lists `send.status` also lists the operation's current name, which a runner
    /// declaring `send_status` must serve. Recorded samples keep their bytes and
    /// checksums, and tests read them through this.
    pub(crate) fn current_contract(mut value: Value) -> Value {
        fn walk(value: &mut Value) {
            match value {
                Value::Object(object) => {
                    for key in [
                        "ordinal",
                        "first_ordinal",
                        "last_ordinal",
                        "next_from_ordinal",
                    ] {
                        if let Some(ordinal) = object.get(key).and_then(Value::as_u64) {
                            object.insert(key.into(), json!(ordinal + 1));
                        }
                    }
                    if let Some(lineage) = object.remove("lineage_id") {
                        object.entry("incarnation").or_insert(lineage);
                        object.entry("history_version").or_insert(json!(0));
                    }
                    object.values_mut().for_each(walk);
                }
                Value::Array(values) => {
                    let named = |name: &str| values.iter().any(|value| value == name);
                    if named("send.status") && !named("session.send_status") {
                        values.push(json!("session.send_status"));
                    }
                    values.iter_mut().for_each(walk);
                }
                _ => {}
            }
        }
        walk(&mut value);
        value
    }

    fn raw_history(page: &Value, params: &Value) -> Result<Value, ReadError> {
        let entries = page["messages"].as_array().expect("raw fixture messages");
        let limit = params["limit"].as_u64().unwrap_or(100) as usize;
        let locate = |mid: &str| {
            entries
                .iter()
                .position(|entry| entry["mid"] == mid)
                .ok_or_else(expired)
        };
        let (start, end) = if let Some(mid) = params["from_mid"].as_str() {
            let start = locate(mid)?;
            (start, (start + limit).min(entries.len()))
        } else if let Some(ordinal) = params["from_ordinal"].as_u64() {
            let start = entries
                .iter()
                .position(|entry| entry["ordinal"] == ordinal)
                .ok_or_else(expired)?;
            (start, (start + limit).min(entries.len()))
        } else {
            let end = if let Some(mid) = params["before_mid"].as_str() {
                locate(mid)?
            } else if let Some(ordinal) = params["before_ordinal"].as_u64() {
                entries
                    .iter()
                    .position(|entry| entry["ordinal"] == ordinal)
                    .ok_or_else(expired)?
            } else {
                entries.len()
            };
            (end.saturating_sub(limit), end)
        };
        Ok(
            json!({"messages":entries[start..end], "older":(start > 0).then_some("older"),
            "newer":(end < entries.len()).then_some("newer"),
            "history_version":page["history_version"], "incarnation":page["incarnation"]}),
        )
    }

    fn entry(mid: &str, ordinal: Option<u64>, at: i64) -> Value {
        json!({"mid":mid, "ordinal":ordinal, "created_at_ms":at, "message":{"role":"assistant", "content":[{"type":"text", "text":mid}]}})
    }

    fn resolved() -> Resolved {
        Resolved {
            reference: SessionRef {
                runner: "broca".into(),
                session: "alfonso:s1".into(),
            },
            harness: "broca".into(),
            project_root: "/workspace/project".into(),
            network: Network::No,
            network_history: vec![
                NetworkChange {
                    network: Network::Yes,
                    since_ms: 700,
                },
                NetworkChange {
                    network: Network::No,
                    since_ms: 710,
                },
            ],
        }
    }

    impl FakeRoute {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                entries: Mutex::new(vec![
                    entry("old-a", Some(18), 699),
                    entry("old-b", Some(19), 700),
                    entry("m20", Some(20), 705),
                    entry("m21", Some(21), 710),
                    entry("m22", Some(22), 720),
                ]),
                calls: Mutex::new(Vec::new()),
                senders: Mutex::new(Vec::new()),
                attaches: AtomicUsize::new(0),
                live_ids: Mutex::new(Vec::new()),
                detaches: Arc::new(AtomicUsize::new(0)),
                history: AtomicU64::new(7),
                incarnation: Mutex::new("inc-a".into()),
                unavailable: AtomicBool::new(false),
                stuck: AtomicBool::new(false),
                updated_at: AtomicU64::new(730),
                run_state: Mutex::new(json!({"run_id":"r1", "state":"active"})),
                head_extra: Mutex::new(serde_json::Map::new()),
                describe: Mutex::new(Some(describe_with(&[]))),
                history_replies: Mutex::new(HashMap::new()),
                anchor_replies: Mutex::new(HashMap::new()),
                read_entries: AtomicUsize::new(0),
            })
        }
        async fn emit(&self, value: Value) {
            let sender = self.senders.lock().unwrap().last().unwrap().clone();
            sender.send(Ok(value)).await.unwrap();
        }
    }

    /// A runner decodes these requests with the contract's types, which refuse
    /// unknown members, so every read this module sends must decode under them.
    fn runner_accepts(op: ReadOp, params: &Value) {
        use cortexkit_role_llm_runner::{history::HistoryRequest, read};
        let decoded = match op {
            ReadOp::Head => serde_json::from_value::<read::HeadRequest>(params.clone()).map(drop),
            ReadOp::Read => serde_json::from_value::<read::ReadRequest>(params.clone()).map(drop),
            ReadOp::History => serde_json::from_value::<HistoryRequest>(params.clone()).map(drop),
            ReadOp::Status | ReadOp::Describe => Ok(()),
        };
        if let Err(error) = decoded {
            panic!(
                "{} {params} is refused by the runner contract: {error}",
                op.method()
            );
        }
    }

    #[async_trait]
    impl BrocaRoute for FakeRoute {
        async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
            let request: Value = serde_json::from_slice(&body(op, params.clone())).unwrap();
            runner_accepts(op, &params);
            self.calls.lock().unwrap().push(request.clone());
            if self.unavailable.load(Ordering::Relaxed) {
                return Err(ReadError::new("provider_unavailable", "Broca route lost"));
            }
            match request["method"].as_str().unwrap() {
                "session.head" => {
                    let (first, last) = {
                        let entries = self.entries.lock().unwrap();
                        let mut ordinals =
                            entries.iter().filter_map(|entry| entry["ordinal"].as_u64());
                        (
                            ordinals.next().unwrap_or(1),
                            entries
                                .iter()
                                .rev()
                                .find_map(|entry| entry["ordinal"].as_u64()),
                        )
                    };
                    let mut head = json!({"first_ordinal":first, "last_ordinal":last, "history_version":self.history.load(Ordering::Relaxed), "incarnation":*self.incarnation.lock().unwrap(),
                    "cut":{"mid":"m19", "digest":"0000000000000000000000000000000000000000000000000000000000000000", "written_at":699},
                    "updated_at":self.updated_at.load(Ordering::Relaxed), "last_run_state":*self.run_state.lock().unwrap(),
                    "input":{"attachments":"yes", "modalities":["image", "pdf"]}});
                    // `u64::MAX` omits the required update time for a malformed-head test.
                    if self.updated_at.load(Ordering::Relaxed) == u64::MAX {
                        head.as_object_mut().unwrap().remove("updated_at");
                    }
                    for (key, value) in self.head_extra.lock().unwrap().iter() {
                        head[key] = value.clone();
                    }
                    Ok(head)
                }
                "run.status" => Ok(json!({"state":"active"})),
                "role.describe" => {
                    self.describe.lock().unwrap().clone().ok_or_else(|| {
                        ReadError::new("provider_unavailable", "role.describe failed")
                    })
                }
                "session.read" => {
                    assert_eq!(params["max_bytes"], 2_097_152);
                    assert!(params.get("session").is_none());
                    let entries = self.entries.lock().unwrap();
                    let from = params["from_ordinal"].as_u64().unwrap_or_else(|| {
                        entries
                            .iter()
                            .rev()
                            .find_map(|entry| entry["ordinal"].as_u64())
                            .unwrap_or(0)
                    });
                    let limit = params["limit"].as_u64().unwrap_or(100).min(2) as usize;
                    let selected: Vec<_> = entries
                        .iter()
                        .filter(|entry| {
                            entry["ordinal"]
                                .as_u64()
                                .map_or(from == 0, |ordinal| ordinal >= from)
                        })
                        .collect();
                    let next = selected
                        .get(limit)
                        .and_then(|entry| entry["ordinal"].as_u64());
                    self.read_entries
                        .fetch_add(selected.len().min(limit), Ordering::Relaxed);
                    Ok(
                        json!({"messages":selected.into_iter().take(limit).collect::<Vec<_>>(), "next_from_ordinal":if self.stuck.load(Ordering::Relaxed) { Some(from) } else { next },
                        "history_version":self.history.load(Ordering::Relaxed), "incarnation":*self.incarnation.lock().unwrap()}),
                    )
                }
                "session.history" if params["form"] != "display" => {
                    if let Some(mid) = params["from_mid"]
                        .as_str()
                        .or_else(|| params["before_mid"].as_str())
                    {
                        if let Some(reply) = self.anchor_replies.lock().unwrap().get(mid) {
                            return reply.clone();
                        }
                    }
                    let entries = self.entries.lock().unwrap();
                    let limit = params["limit"].as_u64().unwrap_or(100) as usize;
                    let find_mid = |mid: &str| {
                        entries
                            .iter()
                            .position(|entry| entry["mid"] == mid)
                            .ok_or_else(expired)
                    };
                    let (start, end) = if let Some(mid) = params["from_mid"].as_str() {
                        let start = find_mid(mid)?;
                        (start, (start + limit).min(entries.len()))
                    } else if let Some(ordinal) = params["from_ordinal"].as_u64() {
                        let start = entries
                            .iter()
                            .position(|entry| entry["ordinal"] == ordinal)
                            .ok_or_else(expired)?;
                        (start, (start + limit).min(entries.len()))
                    } else {
                        let end = if let Some(mid) = params["before_mid"].as_str() {
                            find_mid(mid)?
                        } else if let Some(ordinal) = params["before_ordinal"].as_u64() {
                            entries
                                .iter()
                                .position(|entry| entry["ordinal"] == ordinal)
                                .ok_or_else(expired)?
                        } else {
                            entries.len()
                        };
                        (end.saturating_sub(limit), end)
                    };
                    Ok(
                        json!({"messages":entries[start..end], "older":(start > 0).then_some("older"),
                        "newer":(end < entries.len()).then_some("newer"),
                        "history_version":self.history.load(Ordering::Relaxed), "incarnation":*self.incarnation.lock().unwrap()}),
                    )
                }
                "session.history" => {
                    let from = params["from_ordinal"].as_u64().unwrap();
                    let entries = self.entries.lock().unwrap();
                    let count = entries
                        .iter()
                        .filter(|entry| entry["ordinal"].as_u64().is_some_and(|n| n >= from))
                        .take(params["limit"].as_u64().unwrap() as usize)
                        .count();
                    assert_eq!(
                        params,
                        json!({"from_ordinal":from, "limit":count, "form":"display"})
                    );
                    let replies = self.history_replies.lock().unwrap();
                    if let Some(reply) = replies.get(&from) {
                        reply.clone()
                    } else {
                        let mut messages: Vec<_> = replies
                            .values()
                            .filter_map(|reply| reply.as_ref().ok())
                            .filter_map(|page| page["messages"].as_array())
                            .flatten()
                            .filter(|entry| {
                                entry["ordinal"]
                                    .as_u64()
                                    .is_some_and(|ordinal| ordinal >= from)
                            })
                            .cloned()
                            .collect();
                        messages.sort_by_key(|entry| entry["ordinal"].as_u64());
                        messages.truncate(count);
                        Ok(
                            json!({"messages":messages, "history_version":self.history.load(Ordering::Relaxed), "incarnation":*self.incarnation.lock().unwrap()}),
                        )
                    }
                }
                method => Err(ReadError::new(
                    "scope_owner_mismatch",
                    format!("observer cannot call {method}"),
                )),
            }
        }
        async fn subscribe(&self, live_ids: bool) -> Result<Box<dyn EventStream>, ReadError> {
            self.attaches.fetch_add(1, Ordering::Relaxed);
            self.live_ids.lock().unwrap().push(live_ids);
            let (sender, receiver) = mpsc::channel(16);
            self.senders.lock().unwrap().push(sender);
            Ok(Box::new(FakeEvents {
                receiver,
                detaches: self.detaches.clone(),
            }))
        }
    }

    struct FakeEvents {
        receiver: mpsc::Receiver<Result<Value, ReadError>>,
        detaches: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl EventStream for FakeEvents {
        async fn next(&mut self) -> Result<Option<Value>, ReadError> {
            self.receiver.recv().await.transpose()
        }
    }
    impl Drop for FakeEvents {
        fn drop(&mut self) {
            self.detaches.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn newest_older_newer_and_exact_are_exclusive_across_the_boundary() {
        let route = FakeRoute::new();
        let source = BrocaSource::new(resolved(), route);
        let newest = source.newest(2).await.unwrap();
        assert_eq!(
            newest
                .rows
                .iter()
                .map(|row| row.mid.as_str())
                .collect::<Vec<_>>(),
            ["m21", "m22"]
        );
        let older = source
            .older(&Anchor::Cursor(newest.older.unwrap()), 1)
            .await
            .unwrap();
        assert_eq!(
            older
                .rows
                .iter()
                .map(|row| row.mid.as_str())
                .collect::<Vec<_>>(),
            ["m20"]
        );
        // Reaching back to ordinal 1 reaches the session's start.
        let history = source
            .older(&Anchor::Cursor(older.older.unwrap()), 19)
            .await
            .unwrap();
        assert_eq!(
            history
                .rows
                .iter()
                .map(|row| row.mid.as_str())
                .collect::<Vec<_>>(),
            ["old-a", "old-b"]
        );
        assert!(history.older.is_none());
        assert_eq!(history.rows[0].display["network"], "unknown");
        assert_eq!(history.rows[1].display["network"], "yes");
        let delta = source.newer("m20", 1).await.unwrap();
        assert_eq!(delta.rows[0].mid, "m21");
        assert_eq!(delta.rows[0].display["network"], "no");
        let continuation = source
            .older(&Anchor::Cursor(delta.newer.unwrap()), 1)
            .await
            .unwrap();
        assert_eq!(continuation.rows[0].mid, "m22");
        assert!(continuation.newer.is_none());
        assert!(source.newer("m22", 1).await.unwrap().rows.is_empty());
        assert_eq!(
            source
                .exact(&Anchor::Ordinal(20))
                .await
                .unwrap()
                .unwrap()
                .mid,
            "m20"
        );
        assert_eq!(
            source
                .older(&Anchor::Mid("old-a".into()), 1)
                .await
                .unwrap()
                .rows
                .len(),
            0
        );
        assert_eq!(
            source
                .older(&Anchor::Mid("missing".into()), 1)
                .await
                .unwrap_err()
                .code,
            "cursor_expired"
        );
    }

    /// A page around a message reads it and its numbered neighbours in one bounded read;
    /// pages after an ordinal walk forward to the newest message. A message the runner
    /// doesn't know, or one outside the numbered history, is not in this version.
    #[tokio::test]
    async fn pages_open_around_a_message_and_walk_forward() {
        let route = FakeRoute::new();
        *route.describe.lock().unwrap() = Some(describe_with(&["history_display"]));
        let source = BrocaSource::new(resolved(), route.clone());
        let mids = |page: &HistoryPage| {
            page.rows
                .iter()
                .map(|row| row.mid.clone())
                .collect::<Vec<_>>()
        };
        let around = source.around("m21", 1).await.unwrap().unwrap();
        assert_eq!((around.anchor, around.newest), (Some(21), Some(22)));
        assert_eq!(mids(&around.page), ["m20", "m21", "m22"]);
        let alone = source.around("m22", 0).await.unwrap().unwrap();
        assert_eq!(mids(&alone.page), ["m22"]);
        assert!(source.around("never-written", 1).await.unwrap().is_none());
        let oldest = source.around("old-a", 1).await.unwrap().unwrap();
        assert_eq!(mids(&oldest.page), ["old-a", "old-b"]);
        route.anchor_replies.lock().unwrap().insert(
            "gone".into(),
            Err(ReadError::new("unknown_mid", "no such message")),
        );
        assert!(source.around("gone", 1).await.unwrap().is_none());
        assert_eq!(mids(&source.after(20, 1).await.unwrap().page), ["m21"]);
        assert_eq!(
            mids(&source.after(20, 5).await.unwrap().page),
            ["m21", "m22"]
        );
        assert!(source.after(22, 5).await.unwrap().page.rows.is_empty());

        // On the wire: the anchor's ordinal, both directions' next positions, the end.
        let reader = TranscriptReader::default();
        let around = reader
            .read(
                Some(source.as_ref()),
                json!({"session":"s1", "aroundMessageId":"m21", "limit":0}),
            )
            .await
            .unwrap();
        assert_eq!(around["anchorOrdinal"], 21, "{around}");
        assert_eq!(around["messages"][0]["messageId"], "m21");
        assert_eq!(around["nextBeforeOrdinal"], 21, "{around}");
        assert_eq!(around["nextAfterOrdinal"], 21, "{around}");
        assert_eq!(around["version"], "history:7:incarnation:inc-a");
        let newer = reader
            .read(
                Some(source.as_ref()),
                json!({"session":"s1", "afterOrdinal":21, "limit":10}),
            )
            .await
            .unwrap();
        assert_eq!(newer["messages"][0]["messageId"], "m22");
        assert_eq!(newer["reachedNewest"], true, "{newer}");
        assert!(newer.get("nextAfterOrdinal").is_none());
        let missing = reader
            .read(
                Some(source.as_ref()),
                json!({"session":"s1", "aroundMessageId":"never-written", "limit":3}),
            )
            .await
            .unwrap_err();
        assert_eq!(missing.code, "message_not_found");
        assert_eq!(
            missing.detail,
            Some(json!({"version": "history:7:incarnation:inc-a"}))
        );
        for mixed in [
            json!({"session":"s1", "aroundMessageId":"m21", "beforeOrdinal":21}),
            json!({"session":"s1", "aroundMessageId":"m21", "afterOrdinal":20}),
            json!({"session":"s1", "afterOrdinal":20, "messageOrdinal":21}),
        ] {
            let refused = reader
                .read(Some(source.as_ref()), mixed.clone())
                .await
                .unwrap_err();
            assert_eq!(refused.code, "invalid_request", "{mixed}");
        }
    }

    /// A runner whose description declares no history (the fake's default) can't find a
    /// message by its id.
    #[tokio::test]
    async fn a_runner_without_history_refuses_a_page_around_a_message() {
        let source = BrocaSource::new(resolved(), FakeRoute::new());
        let refused = TranscriptReader::default()
            .read(
                Some(source.as_ref()),
                json!({"session":"s1", "aroundMessageId":"m21", "limit":1}),
            )
            .await
            .unwrap_err();
        assert_eq!(refused.code, "unsupported");
        assert_eq!(refused.detail, Some(json!({"field": "aroundMessageId"})));
    }

    #[tokio::test]
    async fn shared_page_body_uses_ordinal_paging_and_history_counter_for_claims() {
        let route = FakeRoute::new();
        let source = BrocaSource::new(resolved(), route.clone());
        let reader = TranscriptReader::default();
        let newest = reader
            .read(Some(source.as_ref()), json!({"session":"s1", "limit":3}))
            .await
            .unwrap();
        assert_eq!(newest["historyVersion"], 7);
        assert_eq!(newest["version"], "history:7:incarnation:inc-a");
        assert_eq!(newest["messages"].as_array().unwrap().len(), 3);
        assert_eq!(newest["nextBeforeOrdinal"], 20);
        assert!(newest.get("nextBeforeCursor").is_none());
        let history = reader
            .read(
                Some(source.as_ref()),
                json!({"session":"s1", "beforeOrdinal":20, "limit":19}),
            )
            .await
            .unwrap();
        assert_eq!(history["messages"].as_array().unwrap().len(), 2);
        assert_eq!(history["reachedStart"], true);
        let mut claim = json!({"session":"s1", "limit":3, "claim":{"heldFloorMessageId":"m20", "sinceMessageId":"m21", "heldVersion":newest["version"]}});
        *route.incarnation.lock().unwrap() = "inc-b".into();
        let expired = reader
            .read(Some(source.as_ref()), claim.clone())
            .await
            .unwrap();
        assert_eq!(expired["claim"], "cursor_expired");
        claim["claim"]["heldVersion"] = json!("history:7:incarnation:inc-b");
        let matched = reader
            .read(Some(source.as_ref()), claim.clone())
            .await
            .unwrap();
        assert_eq!(matched["claim"], "matched");
        assert_eq!(matched["newTurns"][0]["messageId"], "m22");
        route
            .entries
            .lock()
            .unwrap()
            .push(entry("m23", Some(23), 730));
        let matched = reader
            .read(Some(source.as_ref()), claim.clone())
            .await
            .unwrap();
        assert_eq!(matched["newTurns"].as_array().unwrap().len(), 2);
        route.history.store(8, Ordering::Relaxed);
        let expired = reader.read(Some(source.as_ref()), claim).await.unwrap();
        assert_eq!(expired["claim"], "cursor_expired");
        assert_eq!(expired["historyVersion"], 8);
    }

    #[tokio::test]
    async fn unmatched_provider_frames_are_dropped_without_guessing_a_mid() {
        let route = FakeRoute::new();
        let source = BrocaSource::new(resolved(), route.clone());
        let mut lease = source.acquire().await.unwrap();
        assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Reset);
        for id in ["", "provider-message-id", "new-unpersisted-id"] {
            route.emit(json!({"kind":"display", "event":{"type":"text_delta", "message_id":id, "text":"must not appear"}})).await;
        }
        route.emit(json!({"kind":"display", "event":{"type":"text_delta", "message_id":"m21", "text":"safe"}})).await;
        assert_eq!(
            lease.events.recv().await.unwrap(),
            DisplayInput::Delta {
                mid: "m21".into(),
                text: "safe".into(),
                part_type: occipital_app_reader::display::DisplayPartType::Text
            }
        );
        assert!(lease.events.try_recv().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn transcript_and_display_watchers_share_one_feed_through_linger() {
        let route = FakeRoute::new();
        let source = BrocaSource::new(resolved(), route.clone());
        let weak = Arc::downgrade(&source);
        let a = source.acquire().await.unwrap();
        let b = source.acquire().await.unwrap();
        assert_eq!(route.attaches.load(Ordering::Relaxed), 1);
        drop(a);
        drop(b);
        drop(source);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(59)).await;
        let source = weak.upgrade().expect("source stays alive through linger");
        let c = source.acquire().await.unwrap();
        assert_eq!(route.attaches.load(Ordering::Relaxed), 1);
        drop(c);
        tokio::task::yield_now().await;
        let mut live = source.feed_state();
        tokio::time::advance(SUBSCRIPTION_LINGER).await;
        live.changed().await.unwrap();
        assert!(!*live.borrow());
        assert_eq!(route.detaches.load(Ordering::Relaxed), 1);
        let _new = source.acquire().await.unwrap();
        assert_eq!(route.attaches.load(Ordering::Relaxed), 2);
    }

    /// The feed asks for `live_id` streams only from a runner declaring
    /// `live_ids`, and passes a delta keyed on one, and the store that names its
    /// message, to the display.
    #[tokio::test]
    async fn live_ids_are_asked_for_only_when_declared() {
        let plain = FakeRoute::new();
        let source = BrocaSource::new(resolved(), plain.clone());
        let _lease = source.acquire().await.unwrap();
        assert_eq!(*plain.live_ids.lock().unwrap(), [false]);

        let route = FakeRoute::new();
        *route.describe.lock().unwrap() = Some(describe_with(&["live_events", "live_ids"]));
        let source = BrocaSource::new(resolved(), route.clone());
        let mut lease = source.acquire().await.unwrap();
        assert_eq!(*route.live_ids.lock().unwrap(), [true]);
        assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Reset);
        route
            .emit(json!({"kind":"display", "event":{"type":"text_delta", "live_id":"live-1", "delta":"hi"}}))
            .await;
        assert_eq!(
            lease.events.recv().await.unwrap(),
            DisplayInput::Delta {
                mid: "live-1".into(),
                text: "hi".into(),
                part_type: occipital_app_reader::display::DisplayPartType::Text
            }
        );
        route
            .emit(json!({"kind":"control", "event":{"type":"message_started", "mid":"m1", "live_id":"live-1"}}))
            .await;
        assert_eq!(
            lease.events.recv().await.unwrap(),
            DisplayInput::MessageStored {
                mid: "m1".into(),
                live_id: "live-1".into()
            }
        );
    }

    #[test]
    fn subscribe_params_carry_live_ids_only_when_asked() {
        assert_eq!(subscribe_params(false), json!({"from": "live"}));
        assert_eq!(
            subscribe_params(true),
            json!({"from": "live", "live_ids": true})
        );
    }

    #[tokio::test]
    async fn route_loss_resets_display_and_revalidates_durable_activity_tokens() {
        let route = FakeRoute::new();
        let source = BrocaSource::new(resolved(), route.clone());
        let mut lease = source.acquire().await.unwrap();
        lease.events.recv().await.unwrap();
        let reader = TranscriptReader::default();
        let initial = reader
            .read(Some(source.as_ref()), json!({"session":"s1"}))
            .await
            .unwrap();
        assert_eq!(initial["activity"], "busy");
        let token = initial["activityToken"].clone();
        let hit = reader
            .read(
                Some(source.as_ref()),
                json!({"session":"s1", "activityToken":token}),
            )
            .await
            .unwrap();
        assert_eq!(hit["unchanged"], true);
        let mut changes = source.changes();
        let sender = route.senders.lock().unwrap().last().unwrap().clone();
        sender
            .send(Err(ReadError::new("provider_unavailable", "route closed")))
            .await
            .unwrap();
        assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Reset);
        changes.changed().await.unwrap();
        assert!(!source.activity_feed_live());
        assert_eq!(source.activity(), None);
        let after = reader
            .read(
                Some(source.as_ref()),
                json!({"session":"s1", "activityToken":token}),
            )
            .await
            .unwrap();
        assert_eq!(after["unchanged"], true);
        route.unavailable.store(true, Ordering::Relaxed);
        assert_eq!(
            source.newest(10).await.unwrap_err().code,
            "provider_unavailable"
        );
    }

    #[tokio::test]
    async fn a_feed_attached_mid_run_seeds_the_running_runs_stamp() {
        // The feed subscribes from the live tail, so a run already going replays no
        // `run_started`; its stamp comes from the head the attach reads.
        let route = FakeRoute::new();
        route.head_extra.lock().unwrap().insert(
            "last_run_rich_ui".into(),
            json!({"dialect":"1.0", "catalog_digest":"a".repeat(64)}),
        );
        let source = BrocaSource::new(resolved(), route.clone());
        let mut lease = source.acquire().await.unwrap();
        assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Reset);
        // The seed is sent before the feed task starts, so it is already queued.
        assert_eq!(
            lease.events.try_recv().ok(),
            Some(DisplayInput::RunStart {
                rich_ui: Some(json!({"dialect":"1.0", "catalogDigest":"a".repeat(64)}))
            })
        );
    }

    #[tokio::test]
    async fn a_feed_attached_after_a_stamped_run_ended_seeds_no_stamp() {
        let route = FakeRoute::new();
        *route.run_state.lock().unwrap() = json!({"run_id":"r1", "state":"completed"});
        route.head_extra.lock().unwrap().insert(
            "last_run_rich_ui".into(),
            json!({"dialect":"1.0", "catalog_digest":"a".repeat(64)}),
        );
        let source = BrocaSource::new(resolved(), route.clone());
        let mut lease = source.acquire().await.unwrap();
        assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Reset);
        assert!(lease.events.try_recv().is_err());
    }

    #[tokio::test]
    async fn controls_wake_history_update_activity_and_unknown_events_reset_display() {
        let route = FakeRoute::new();
        let source = BrocaSource::new(resolved(), route.clone());
        let mut lease = source.acquire().await.unwrap();
        lease.events.recv().await.unwrap();
        let mut changes = source.changes();
        route
            .emit(json!({"kind":"control", "unit":{"type":"RunFinished", "run_id":"r1"}}))
            .await;
        assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::TurnEnd);
        changes.changed().await.unwrap();
        *route.run_state.lock().unwrap() = json!({"run_id":"r1", "state":"completed"});
        route.updated_at.store(740, Ordering::Relaxed);
        source.head().await.unwrap();
        assert_eq!(source.activity().unwrap().0, "idle");
        route
            .emit(json!({"kind":"display", "event":{"type":"future_display"}}))
            .await;
        assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Reset);
        route
            .emit(json!({"kind":"control", "unit":{"type":"RunStarted", "run_id":"r2"}}))
            .await;
        assert_eq!(
            lease.events.recv().await.unwrap(),
            DisplayInput::RunStart { rich_ui: None }
        );
        *route.run_state.lock().unwrap() = json!({"run_id":"r2", "state":"active"});
        route.updated_at.store(750, Ordering::Relaxed);
        source.head().await.unwrap();
        assert_eq!(source.activity().unwrap().0, "busy");
    }

    #[tokio::test]
    async fn owner_free_reads_never_open_or_acquire_the_session() {
        let route = FakeRoute::new();
        let source = BrocaSource::new(resolved(), route.clone());
        source.head().await.unwrap();
        source.newest(10).await.unwrap();
        source.exact(&Anchor::Mid("m20".into())).await.unwrap();
        let _lease = source.acquire().await.unwrap();
        for request in route.calls.lock().unwrap().iter() {
            // `role.describe` is the one non-session read: it only tells what a
            // head without `sender` means.
            assert!(matches!(
                request["method"].as_str(),
                Some(
                    "session.head"
                        | "session.read"
                        | "session.history"
                        | "run.status"
                        | "role.describe"
                )
            ));
            for forbidden in ["owner", "grant", "scope", "session", "run_id"] {
                assert!(
                    request["params"].get(forbidden).is_none(),
                    "read must not claim {forbidden}"
                );
            }
        }
        let identity = resolved().identity();
        assert_eq!(
            serde_json::to_value(identity).unwrap()["session"],
            "alfonso:s1"
        );
        assert_eq!(
            serde_json::to_value(target(&resolved())).unwrap()["module_id"],
            "broca"
        );
    }

    #[tokio::test]
    async fn source_cursors_refuse_a_new_log_rewrites_and_stuck_raw_cursors() {
        let route = FakeRoute::new();
        let source = BrocaSource::new(resolved(), route.clone());
        let cursor = source.newest(1).await.unwrap().older.unwrap();
        *route.incarnation.lock().unwrap() = "inc-recreated".into();
        assert_eq!(
            source
                .older(&Anchor::Cursor(cursor.clone()), 1)
                .await
                .unwrap_err()
                .code,
            "cursor_expired"
        );
        route.history.store(8, Ordering::Relaxed);
        assert_eq!(
            source
                .older(&Anchor::Cursor(cursor), 1)
                .await
                .unwrap_err()
                .code,
            "cursor_expired"
        );
        route.stuck.store(true, Ordering::Relaxed);
        assert_eq!(
            source.newest(1).await.unwrap_err().code,
            "invalid_provider_reply"
        );
    }

    #[tokio::test]
    async fn exact_read_pairs_from_the_selected_message_and_names_missing_results() {
        let route = FakeRoute::new();
        let call = |mid: &str, ordinal: u64| json!({"mid":mid, "ordinal":ordinal, "message":{"role":"assistant", "content":[{"type":"tool_call", "name":"read", "input":{"path":"x"}}]}});
        *route.entries.lock().unwrap() = vec![
            call("m20", 20),
            call("m21", 21),
            json!({"mid":"m22", "ordinal":22, "message":{"role":"tool", "content":[{"type":"tool_result", "output":[{"type":"text", "text":"body"}]}]}}),
        ];
        let source = BrocaSource::new(resolved(), route.clone());
        let row = source
            .exact(&Anchor::Mid("m21".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.display["parts"][0]["arguments"], json!({"path":"x"}));
        assert_eq!(row.display["parts"][0]["content"], "body");
        route.entries.lock().unwrap().pop();
        let error = source.exact(&Anchor::Mid("m21".into())).await.unwrap_err();
        assert_eq!(error.code, "full_content_unavailable");
        assert!(error.message.ends_with("for read"));
    }

    #[tokio::test]
    async fn malformed_event_ends_the_feed_instead_of_leaving_activity_live() {
        let route = FakeRoute::new();
        let source = BrocaSource::new(resolved(), route.clone());
        let mut lease = source.acquire().await.unwrap();
        assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Reset);
        route.emit(json!({"kind":"display", "event":[]})).await;
        assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Reset);
        assert!(!source.activity_feed_live());
        assert_eq!(route.detaches.load(Ordering::Relaxed), 1);
    }

    struct Lookup;
    #[async_trait]
    impl Entorhinal for Lookup {
        async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
            // A session named under another harness: Entorhinal knows none here.
            if method == "agent.session_for_harness" {
                return Ok(
                    json!({"code":"not_found", "detail":null, "message":"no agent session"}),
                );
            }
            assert_eq!(method, "agent.session_for_runner");
            assert_eq!(params, json!({"runner":"broca", "session":"alfonso:s1"}));
            Ok(
                json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref", "incarnation":"entorhinal-test", "ended_at_ms":null,
                "runner":"broca", "session":"alfonso:s1", "harness":"broca", "project_root":"/workspace/resolved-project",
                "network":"yes", "network_history":[{"network":"yes", "since_ms":700}]}}),
            )
        }
    }
    #[tokio::test]
    async fn resolved_identity_and_network_reach_the_source_without_owner_adoption() {
        let resolved = Resolver::new(Arc::new(Lookup))
            .resolve(&resolved().reference)
            .await
            .unwrap();
        // The runner's route names the session, never its folder; the folder
        // stays on the resolution, for placing uploads.
        let identity = serde_json::to_value(resolved.identity()).unwrap();
        assert_eq!(
            identity,
            json!({"harness": "broca", "session": "alfonso:s1"})
        );
        assert_eq!(
            resolved.project_root,
            std::path::Path::new("/workspace/resolved-project")
        );
        let source = BrocaSource::new(resolved, FakeRoute::new());
        assert_eq!(
            source.newest(1).await.unwrap().rows[0].display["network"],
            "yes"
        );
    }

    /// Entorhinal's answers for one head, with the runner it records: found by harness,
    /// and then, for a runner this reader serves, by that runner. The head is a Pi
    /// one under Thalamus's Pi runner and an OpenCode one otherwise.
    struct ByHarness(Option<&'static str>);
    impl ByHarness {
        fn harness(&self) -> &'static str {
            match self.0 {
                Some(PI_RUNNER_MODULE_ID) => "pi",
                _ => "opencode2",
            }
        }
    }
    #[async_trait]
    impl Entorhinal for ByHarness {
        async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
            match method {
                "agent.session_for_harness" => assert_eq!(
                    params,
                    json!({"harness": self.harness(), "session":"ses_head"})
                ),
                "agent.session_for_runner" => {
                    assert_eq!(params, json!({"runner": self.0, "session":"ses_head"}))
                }
                other => panic!("unexpected Entorhinal call {other}"),
            }
            Ok(match self.0 {
                Some("missing") => {
                    json!({"code":"not_found", "detail":null, "message":"no agent session"})
                }
                runner => {
                    json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref",
                    "incarnation":"entorhinal-test", "ended_at_ms":null, "runner":runner, "session":"ses_head",
                    "harness":self.harness(), "project_root":"/workspace/head", "network":"unknown", "network_history":[]}})
                }
            })
        }
    }

    /// A session the phone names under another harness is routed by the runner
    /// Entorhinal records for it: one this reader doesn't serve, or none, is refused
    /// `session_unknown` naming that runner, so the phone routes again; reads and watches
    /// open no Broca route for it.
    #[tokio::test]
    async fn a_session_of_a_runner_this_reader_does_not_serve_is_refused_naming_it() {
        let opened = FakeRoute::new();
        for (recorded, code, runner) in [
            (None, "session_unknown", json!(null)),
            (Some("missing"), "session_unknown", json!(null)),
            (Some("elsewhere"), "session_unknown", json!("elsewhere")),
            // This module hosts no sessions of its own.
            (
                Some(crate::MODULE_ID),
                "session_unknown",
                json!(crate::MODULE_ID),
            ),
        ] {
            let sources = BrocaSources::new(
                Arc::new(Resolver::new(Arc::new(ByHarness(recorded)))),
                Arc::new(Routes(opened.clone())),
            );
            let read = match SourceResolver::resolve(&sources, "ses_head", Some("opencode2")).await
            {
                Err(error) => error,
                Ok(_) => panic!("{recorded:?}: a source was opened"),
            };
            assert_eq!(read.code, code, "{recorded:?}: {read:?}");
            if code == "session_unknown" {
                assert_eq!(read.detail, Some(json!({"runner": runner})), "{recorded:?}");
            }
            let watch = match sources
                .watch(
                    "ses_head",
                    Some("opencode2"),
                    Arc::new(DisplayHub::default()),
                )
                .await
            {
                Err(error) => error,
                Ok(_) => panic!("{recorded:?}: a watch was opened"),
            };
            assert_eq!(watch.code, read.code);
            let checked = sources
                .validate_session("ses_head", Some("opencode2"))
                .await
                .unwrap_err();
            assert_eq!(checked.code, read.code, "{recorded:?}");
            assert_eq!(checked.detail, read.detail, "{recorded:?}");
        }
        assert!(
            opened.calls.lock().unwrap().is_empty(),
            "no Broca call was made"
        );
    }

    /// A display read of a session on Thalamus's OpenCode 2 or Pi runner checks the
    /// session under that runner, as a page read finds it, and opens no route: it is
    /// answered from what Occipital already holds.
    #[tokio::test]
    async fn a_display_read_checks_the_session_under_its_own_runner() {
        for (runner, harness) in [
            (OPENCODE_RUNNER_MODULE_ID, "opencode2"),
            (PI_RUNNER_MODULE_ID, "pi"),
        ] {
            let opened = FakeRoute::new();
            let sources = BrocaSources::new(
                Arc::new(Resolver::new(Arc::new(ByHarness(Some(runner))))),
                Arc::new(Routes(opened.clone())),
            );
            let app = super::super::AppReader::new(Arc::new(sources));
            let read = app
                .read(
                    super::super::DISPLAY_READ,
                    json!({"session": "ses_head", "harness": harness}),
                )
                .await
                .unwrap_or_else(|error| panic!("{harness}: {error:?}"));
            // No watch has attached a display ring for it here.
            assert!(read.get("absent").is_some(), "{harness}: {read}");
            assert!(opened.calls.lock().unwrap().is_empty(), "{harness}");
        }
    }

    /// Routes that only count the send paths handed out.
    #[derive(Default)]
    struct CountingSends(std::sync::atomic::AtomicUsize);
    struct NoCalls(SessionRef);
    #[async_trait]
    impl SendTarget for NoCalls {
        fn reference(&self) -> &SessionRef {
            &self.0
        }
        async fn call(
            &self,
            _: &'static str,
            _: Value,
        ) -> Result<Value, super::super::send::RunnerError> {
            unreachable!("routing only")
        }
    }
    #[async_trait]
    impl BrocaRoutes for CountingSends {
        async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
            unreachable!("routing only")
        }
        fn sender(&self, resolved: &Resolved) -> Option<Arc<dyn SendTarget>> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(Arc::new(NoCalls(resolved.reference.clone())))
        }
    }

    /// A send names its session's harness as a read does. A session Thalamus's
    /// OpenCode or Pi runner hosts gets that runner's send path. Another recorded runner is
    /// refused `session_unknown` with that runner in `detail.runner`, and no recorded
    /// runner with `detail.runner` null. Neither opens a send path.
    #[tokio::test]
    async fn a_send_routes_by_the_runner_entorhinal_records() {
        let routes = Arc::new(CountingSends::default());
        let ours = BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(ByHarness(Some(
                OPENCODE_RUNNER_MODULE_ID,
            ))))),
            routes.clone(),
        );
        let target = ours
            .send_target("ses_head", Some("opencode2"))
            .await
            .unwrap();
        assert_eq!(target.reference().runner, OPENCODE_RUNNER_MODULE_ID);
        assert_eq!(target.reference().session, "ses_head");
        let pi = BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(ByHarness(Some(
                PI_RUNNER_MODULE_ID,
            ))))),
            routes.clone(),
        );
        let target = pi.send_target("ses_head", Some("pi")).await.unwrap();
        assert_eq!(target.reference().runner, PI_RUNNER_MODULE_ID);
        for (recorded, runner) in [(None, json!(null)), (Some("elsewhere"), json!("elsewhere"))] {
            let sources = BrocaSources::new(
                Arc::new(Resolver::new(Arc::new(ByHarness(recorded)))),
                routes.clone(),
            );
            let error = match sources.send_target("ses_head", Some("opencode2")).await {
                Err(error) => error,
                Ok(_) => panic!("{recorded:?}: a send path was opened"),
            };
            assert_eq!(error.code, "session_unknown");
            assert_eq!(error.detail, Some(json!({"runner": runner})));
        }
        assert_eq!(routes.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    struct Routes(Arc<FakeRoute>);
    #[async_trait]
    impl BrocaRoutes for Routes {
        async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
            assert_eq!(
                resolved.project_root,
                std::path::PathBuf::from("/workspace/resolved-project")
            );
            assert_eq!(resolved.harness, "broca");
            assert_eq!(resolved.reference.session, "alfonso:s1");
            Ok(self.0.clone())
        }
    }

    fn serving(route: Arc<FakeRoute>) -> (Arc<BrocaSources>, Arc<super::super::AppReader>) {
        let sources = Arc::new(BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(Lookup))),
            Arc::new(Routes(route)),
        ));
        let app = Arc::new(super::super::AppReader::new(sources.clone()));
        (sources, app)
    }

    fn params() -> Value {
        json!({"session":"alfonso:s1", "harness":"broca", "limit":3})
    }

    struct GatedConnector {
        calls: AtomicUsize,
        block: AtomicBool,
        fail: AtomicBool,
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
        sources: Arc<BrocaSources>,
    }
    #[async_trait]
    impl SourceConnector for GatedConnector {
        async fn connect(&self) -> Result<Arc<BrocaSources>, ReadError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.block.swap(false, Ordering::Relaxed) {
                self.started.notify_one();
                self.release.notified().await;
            }
            if self.fail.load(Ordering::Relaxed) {
                Err(ReadError::new("connect_failed", "daemon unavailable"))
            } else {
                Ok(self.sources.clone())
            }
        }
    }

    #[tokio::test]
    async fn lazy_consumers_share_failed_attempts_retry_and_cache_success() {
        let (sources, _) = serving(FakeRoute::new());
        let connector = Arc::new(GatedConnector {
            calls: AtomicUsize::new(0),
            block: AtomicBool::new(true),
            fail: AtomicBool::new(true),
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            sources,
        });
        let lazy = Arc::new(LazyBrocaSources::new(connector.clone()));
        assert_eq!(connector.calls.load(Ordering::Relaxed), 0);
        let first = {
            let lazy = lazy.clone();
            tokio::spawn(async move { lazy.connected().await })
        };
        connector.started.notified().await;
        let second = {
            let lazy = lazy.clone();
            tokio::spawn(async move { lazy.connected().await })
        };
        tokio::task::yield_now().await;
        assert_eq!(connector.calls.load(Ordering::Relaxed), 1);
        connector.release.notify_one();
        assert_eq!(
            first.await.unwrap().err().unwrap().code,
            "provider_unavailable"
        );
        assert_eq!(
            second.await.unwrap().err().unwrap().code,
            "provider_unavailable"
        );
        assert_eq!(connector.calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            lazy.connected().await.err().unwrap().code,
            "provider_unavailable"
        );
        assert_eq!(connector.calls.load(Ordering::Relaxed), 2);
        connector.fail.store(false, Ordering::Relaxed);
        assert!(lazy
            .resolve("alfonso:s1", Some("broca"))
            .await
            .unwrap()
            .is_some());
        assert!(lazy
            .resolve("alfonso:s1", Some("broca"))
            .await
            .unwrap()
            .is_some());
        assert_eq!(connector.calls.load(Ordering::Relaxed), 3);
    }

    struct NetworkLookup(AtomicBool);
    #[async_trait]
    impl Entorhinal for NetworkLookup {
        async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
            let mut reply = Lookup.request(method, params).await?;
            if self.0.load(Ordering::Relaxed) {
                reply["result"]["network"] = json!("no");
                reply["result"]["network_history"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"network":"no", "since_ms":710}));
            }
            Ok(reply)
        }
    }

    #[tokio::test]
    async fn a_watched_source_refreshes_network_history_before_accepting_a_cache_token() {
        let route = FakeRoute::new();
        let lookup = Arc::new(NetworkLookup(AtomicBool::new(false)));
        let sources = BrocaSources::new(
            Arc::new(Resolver::new(lookup.clone())),
            Arc::new(Routes(route)),
        );
        let source = sources.source(&resolved().reference).await.unwrap();
        let _lease = source.acquire().await.unwrap();
        let reader = TranscriptReader::default();
        let first = reader.read(Some(source.as_ref()), params()).await.unwrap();
        assert_eq!(first["messages"][2]["network"], "yes");
        let token = first["activityToken"].clone();
        lookup.0.store(true, Ordering::Relaxed);
        let mut request = params();
        request["activityToken"] = token.clone();
        let next = reader.read(Some(source.as_ref()), request).await.unwrap();
        assert!(next.get("unchanged").is_none());
        assert_eq!(next["messages"][2]["network"], "no");
        assert_ne!(next["activityToken"], token);
    }

    struct SerialLookup {
        denied: AtomicBool,
        block: AtomicBool,
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
        inflight: AtomicUsize,
        maximum: AtomicUsize,
    }
    #[async_trait]
    impl Entorhinal for SerialLookup {
        async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
            let active = self.inflight.fetch_add(1, Ordering::Relaxed) + 1;
            self.maximum.fetch_max(active, Ordering::Relaxed);
            let denied = self.denied.load(Ordering::Relaxed);
            let mut reply = Lookup.request(method, params).await?;
            if self.block.swap(false, Ordering::Relaxed) {
                self.started.notify_one();
                self.release.notified().await;
            }
            if denied {
                reply["result"]["network"] = json!("no");
                reply["result"]["network_history"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"network":"no", "since_ms":710}));
            }
            self.inflight.fetch_sub(1, Ordering::Relaxed);
            Ok(reply)
        }
    }

    #[tokio::test]
    async fn concurrent_refreshes_cannot_publish_an_older_network_policy_last() {
        let lookup = Arc::new(SerialLookup {
            denied: AtomicBool::new(false),
            block: AtomicBool::new(false),
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            inflight: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
        });
        let sources = Arc::new(BrocaSources::new(
            Arc::new(Resolver::new(lookup.clone())),
            Arc::new(Routes(FakeRoute::new())),
        ));
        let source = sources.source(&resolved().reference).await.unwrap();
        lookup.block.store(true, Ordering::Relaxed);
        let first = {
            let source = source.clone();
            tokio::spawn(async move { source.head().await })
        };
        lookup.started.notified().await;
        lookup.denied.store(true, Ordering::Relaxed);
        let second = {
            let sources = sources.clone();
            tokio::spawn(async move { sources.source(&resolved().reference).await })
        };
        tokio::task::yield_now().await;
        lookup.release.notify_one();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(lookup.maximum.load(Ordering::Relaxed), 1);
        assert_eq!(source.network(), Network::No);
    }

    #[tokio::test]
    async fn a_plain_read_starts_no_subscription() {
        let route = FakeRoute::new();
        let (sources, app) = serving(route.clone());
        let page = app
            .read(super::super::TRANSCRIPT_PAGE, params())
            .await
            .unwrap();
        assert_eq!(page["messages"].as_array().unwrap().len(), 3);
        assert_eq!(route.attaches.load(Ordering::Relaxed), 0);
        assert!(app.display.projection("alfonso:s1").is_none());
        // A session named under another harness opens no Broca route either: one
        // Entorhinal doesn't know is refused naming no runner.
        for harness in ["opencode2", "pi"] {
            let refused =
                match SourceResolver::resolve(sources.as_ref(), "not-broca", Some(harness)).await {
                    Err(error) => error,
                    Ok(_) => panic!("{harness}: a source was opened"),
                };
            assert_eq!(refused.code, "session_unknown", "{harness}");
            assert_eq!(refused.detail, Some(json!({"runner": null})), "{harness}");
        }
        assert_eq!(route.attaches.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn idle_activity_time_survives_source_rebuilds_and_advances_with_a_run() {
        let route = FakeRoute::new();
        *route.run_state.lock().unwrap() = json!({"run_id":"r1", "state":"completed"});
        let (sources, _) = serving(route.clone());
        let reader = TranscriptReader::default();
        for _ in 0..2 {
            let source = sources.source(&resolved().reference).await.unwrap();
            let weak = Arc::downgrade(&source);
            let reply = reader.read(Some(source.as_ref()), params()).await.unwrap();
            assert_eq!(reply["activity"], "idle");
            assert_eq!(reply["activityAtMs"], 730);
            drop(source);
            assert!(weak.upgrade().is_none());
        }
        *route.run_state.lock().unwrap() = json!({"run_id":"r2", "state":"active"});
        route.updated_at.store(740, Ordering::Relaxed);
        let source = sources.source(&resolved().reference).await.unwrap();
        let reply = reader.read(Some(source.as_ref()), params()).await.unwrap();
        assert_eq!(reply["activity"], "busy");
        assert_eq!(reply["activityAtMs"], 740);
        assert_eq!(route.attaches.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn a_written_head_without_its_required_update_time_is_malformed() {
        let route = FakeRoute::new();
        *route.run_state.lock().unwrap() = json!({"run_id":"r1", "state":"completed"});
        route.updated_at.store(u64::MAX, Ordering::Relaxed);
        let (sources, _) = serving(route.clone());
        let reader = TranscriptReader::default();
        let source = sources.source(&resolved().reference).await.unwrap();
        // updated_at is absent only before the session is written; a written
        // session cannot report a partial head without its durable update time.
        let error = reader
            .read(Some(source.as_ref()), params())
            .await
            .unwrap_err();
        assert_eq!(error.code, "invalid_provider_reply");
        assert_eq!(
            error.message,
            "Broca session.head has an inconsistent snapshot"
        );
    }

    #[tokio::test]
    async fn durable_tokens_survive_rebuilds_and_reader_restarts_but_not_a_run() {
        let route = FakeRoute::new();
        *route.run_state.lock().unwrap() = json!({"run_id":"r1", "state":"completed"});
        let mut token = Value::Null;
        for _ in 0..2 {
            let (sources, app) = serving(route.clone());
            let first = app
                .read(super::super::TRANSCRIPT_PAGE, params())
                .await
                .unwrap();
            assert!(first["activityToken"].is_string());
            if !token.is_null() {
                assert_eq!(first["activityToken"], token);
            }
            token = first["activityToken"].clone();
            assert!(sources
                .sources
                .lock()
                .await
                .values()
                .all(|slot| { slot.try_lock().unwrap().upgrade().is_none() }));
            route.calls.lock().unwrap().clear();
            let mut request = params();
            request["activityToken"] = token.clone();
            let hit = app
                .read(super::super::TRANSCRIPT_PAGE, request)
                .await
                .unwrap();
            assert_eq!(hit["unchanged"], true);
            assert_eq!(hit["activityAtMs"], 730);
            assert!(route
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|call| call["method"] != "session.read"));
        }
        *route.run_state.lock().unwrap() = json!({"run_id":"r2", "state":"active"});
        route.updated_at.store(740, Ordering::Relaxed);
        route
            .entries
            .lock()
            .unwrap()
            .push(entry("m23", Some(23), 740));
        let (_, app) = serving(route.clone());
        let mut request = params();
        request["activityToken"] = token.clone();
        let next = app
            .read(super::super::TRANSCRIPT_PAGE, request)
            .await
            .unwrap();
        assert!(next.get("unchanged").is_none());
        assert_ne!(next["activityToken"], token);
        assert_eq!(next["activityAtMs"], 740);
        *route.incarnation.lock().unwrap() = "inc-replacement".into();
        let replaced = app
            .read(super::super::TRANSCRIPT_PAGE, params())
            .await
            .unwrap();
        assert_ne!(replaced["activityToken"], next["activityToken"]);
        assert_eq!(route.attaches.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn a_recycled_session_key_cannot_match_old_mids_at_history_zero() {
        let route = FakeRoute::new();
        route.history.store(0, Ordering::Relaxed);
        *route.entries.lock().unwrap() = vec![entry("m0", Some(1), 700), entry("m1", Some(2), 710)];
        let source = BrocaSource::new(resolved(), route.clone());
        let reader = TranscriptReader::default();
        let old = reader.read(Some(source.as_ref()), params()).await.unwrap();
        let cursor = source.newest(1).await.unwrap().older.unwrap();
        *route.incarnation.lock().unwrap() = "inc-recreated".into();
        route.entries.lock().unwrap()[0]["message"]["content"][0]["text"] =
            json!("unrelated replacement");
        let reply = reader.read(Some(source.as_ref()), json!({"session":"s1", "claim":{"heldFloorMessageId":"m0", "sinceMessageId":"m1", "heldVersion":old["version"]}})).await.unwrap();
        assert_eq!(reply["claim"], "cursor_expired");
        assert_ne!(reply["version"], old["version"]);
        assert_eq!(
            source
                .older(&Anchor::Cursor(cursor), 1)
                .await
                .unwrap_err()
                .code,
            "cursor_expired"
        );
    }

    #[tokio::test]
    async fn broca_written_at_reaches_page_history_full_and_claim_rows() {
        let route = FakeRoute::new();
        {
            let mut entries = route.entries.lock().unwrap();
            for entry in entries.iter_mut() {
                let at = entry
                    .as_object_mut()
                    .unwrap()
                    .remove("created_at_ms")
                    .unwrap();
                entry["written_at"] = at;
            }
            entries[2]["mid"] = json!("opaque-mid-not-ordinal-20");
            entries[0]["message"] = json!({"role":42});
        }
        let source = BrocaSource::new(resolved(), route);
        let reader = TranscriptReader::default();
        let page = reader.read(Some(source.as_ref()), params()).await.unwrap();
        assert_eq!(
            page["messages"][0]["messageId"],
            "opaque-mid-not-ordinal-20"
        );
        assert_eq!(page["messages"][0]["createdAtMs"], 705);
        let history = reader
            .read(
                Some(source.as_ref()),
                json!({"session":"s1", "beforeOrdinal":page["nextBeforeOrdinal"]}),
            )
            .await
            .unwrap();
        assert_eq!(history["messages"][0]["createdAtMs"], 699);
        assert_eq!(history["messages"][0]["unreadable"], true);
        let full = reader
            .read(
                Some(source.as_ref()),
                json!({"session":"s1", "messageOrdinal":20}),
            )
            .await
            .unwrap();
        assert_eq!(full["messages"][0]["createdAtMs"], 705);
        let delta = reader.read(Some(source.as_ref()), json!({"session":"s1", "claim":{"heldFloorMessageId":"opaque-mid-not-ordinal-20", "sinceMessageId":"m21", "heldVersion":page["version"]}})).await.unwrap();
        assert_eq!(delta["newTurns"][0]["createdAtMs"], 720);
    }

    #[tokio::test]
    async fn current_claim_matches_empty_while_current_activity_token_is_unchanged() {
        let route = FakeRoute::new();
        let (_, app) = serving(route);
        let page = app
            .read(super::super::TRANSCRIPT_PAGE, params())
            .await
            .unwrap();
        let mut request = params();
        request["activityToken"] = page["activityToken"].clone();
        let hit = app
            .read(super::super::TRANSCRIPT_PAGE, request)
            .await
            .unwrap();
        assert_eq!(hit["unchanged"], true);
        let mut request = params();
        request["claim"] = json!({"heldFloorMessageId":"m20", "sinceMessageId":"m22", "heldVersion":page["version"]});
        let matched = app
            .read(super::super::TRANSCRIPT_PAGE, request)
            .await
            .unwrap();
        assert_eq!(matched["claim"], "matched");
        assert_eq!(matched["newTurns"], json!([]));
        assert_eq!(matched["vouchedFromMessageId"], "m20");
        assert!(matched.get("unchanged").is_none());
    }

    async fn open_watch(
        connection: &mut super::super::tests::Connection,
        tx: &mpsc::Sender<subc_protocol::Frame>,
        corr: u64,
        method: &str,
    ) {
        use subc_protocol::{BindIdentity, Flags, Frame, FrameType, Principal, Priority};
        let mut params = params();
        if method == super::super::TRANSCRIPT_WATCH {
            params["claim"] = json!({"heldFloorMessageId":"m20", "sinceMessageId":"m22", "heldVersion":"history:7"});
        } else {
            params.as_object_mut().unwrap().remove("limit");
        }
        let frame = Frame::build(
            FrameType::Request,
            Flags::new(false, Priority::Interactive, false),
            1,
            1,
            corr,
            json!({"method":method, "params":params})
                .to_string()
                .into_bytes(),
        )
        .unwrap();
        let binding = (
            Some(Principal::Reserved {
                module_id: "callosum".into(),
            }),
            RouteTarget::ManagementSurface {
                module_id: crate::MODULE_ID.into(),
            },
            None,
            Some(BindIdentity::new("fed-phone:test", "caller")),
            None,
        );
        connection
            .dispatch(frame, Some(binding), method.into(), params, None, tx)
            .await
            .unwrap();
    }

    async fn users(source: &BrocaSource, expected: usize) {
        for _ in 0..1000 {
            if source
                .feed
                .lock()
                .await
                .as_ref()
                .is_some_and(|feed| *feed.users.borrow() == expected)
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("upstream watcher count did not reach {expected}");
    }

    async fn ended(rx: &mut mpsc::Receiver<subc_protocol::Frame>, corr: u64) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = rx.recv().await.unwrap();
                if frame.header.corr == corr
                    && frame.header.ty == subc_protocol::FrameType::StreamEnd
                {
                    break;
                }
                assert_ne!(
                    frame.header.ty,
                    subc_protocol::FrameType::Error,
                    "{}",
                    String::from_utf8_lossy(&frame.body)
                );
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn serving_watch_leases_share_feed_cancel_linger_and_drop_on_route_close() {
        let route = FakeRoute::new();
        let (sources, app) = serving(route.clone());
        let source = sources.source(&resolved().reference).await.unwrap();
        let mut connection = super::super::tests::Connection::new(app.clone());
        let (tx, mut rx) = mpsc::channel(64);
        open_watch(&mut connection, &tx, 1, super::super::TRANSCRIPT_WATCH).await;
        open_watch(&mut connection, &tx, 2, super::super::DISPLAY_WATCH).await;
        for _ in 0..1000 {
            if source
                .feed
                .lock()
                .await
                .as_ref()
                .is_some_and(|feed| *feed.users.borrow() == 2)
            {
                break;
            }
            if let Ok(frame) = rx.try_recv() {
                assert_ne!(
                    frame.header.ty,
                    subc_protocol::FrameType::Error,
                    "{}",
                    String::from_utf8_lossy(&frame.body)
                );
            }
            tokio::task::yield_now().await;
        }
        users(&source, 2).await;
        assert_eq!(route.attaches.load(Ordering::Relaxed), 1);
        assert!(app.display.projection("alfonso:s1").is_some());
        connection.cancel((1, 1, 1));
        ended(&mut rx, 1).await;
        users(&source, 1).await;
        connection.cancel((1, 1, 2));
        ended(&mut rx, 2).await;
        users(&source, 0).await;
        tokio::task::yield_now().await;
        let mut live = source.feed_state();
        tokio::time::advance(SUBSCRIPTION_LINGER).await;
        live.changed().await.unwrap();
        assert_eq!(route.detaches.load(Ordering::Relaxed), 1);
        assert!(app.display.projection("alfonso:s1").is_none());

        open_watch(&mut connection, &tx, 3, super::super::DISPLAY_WATCH).await;
        users(&source, 1).await;
        assert_eq!(route.attaches.load(Ordering::Relaxed), 2);
        connection.end_route(1, 1);
        ended(&mut rx, 3).await;
        users(&source, 0).await;
        tokio::task::yield_now().await;
        let mut live = source.feed_state();
        tokio::time::advance(SUBSCRIPTION_LINGER).await;
        live.changed().await.unwrap();
        assert_eq!(route.detaches.load(Ordering::Relaxed), 2);
        connection.shutdown().await;
    }

    #[tokio::test]
    async fn provider_loss_ends_the_device_watch_and_a_new_watch_can_attach() {
        let route = FakeRoute::new();
        let (sources, app) = serving(route.clone());
        let source = sources.source(&resolved().reference).await.unwrap();
        let mut connection = super::super::tests::Connection::new(app.clone());
        let (tx, mut rx) = mpsc::channel(64);
        open_watch(&mut connection, &tx, 1, super::super::DISPLAY_WATCH).await;
        users(&source, 1).await;
        let sender = route.senders.lock().unwrap().last().unwrap().clone();
        sender
            .send(Err(ReadError::new("provider_unavailable", "route gone")))
            .await
            .unwrap();
        ended(&mut rx, 1).await;
        users(&source, 0).await;
        assert!(!source.activity_feed_live());
        open_watch(&mut connection, &tx, 2, super::super::DISPLAY_WATCH).await;
        users(&source, 1).await;
        assert_eq!(route.attaches.load(Ordering::Relaxed), 2);
        connection.cancel((1, 1, 2));
        ended(&mut rx, 2).await;
        connection.shutdown().await;
    }

    struct MovingLookup(AtomicBool);
    #[async_trait]
    impl Entorhinal for MovingLookup {
        async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
            let mut reply = Lookup.request(method, params).await?;
            if self.0.load(Ordering::Relaxed) {
                reply["result"]["project_root"] = json!("/workspace/moved");
            }
            Ok(reply)
        }
    }
    struct MovingRoutes(Arc<FakeRoute>, Mutex<Vec<std::path::PathBuf>>);
    #[async_trait]
    impl BrocaRoutes for MovingRoutes {
        async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
            self.1.lock().unwrap().push(resolved.project_root.clone());
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn a_moved_binding_retires_the_old_feed_before_attaching_its_replacement() {
        let route = FakeRoute::new();
        let lookup = Arc::new(MovingLookup(AtomicBool::new(false)));
        let routes = Arc::new(MovingRoutes(route.clone(), Mutex::new(Vec::new())));
        let sources = BrocaSources::new(Arc::new(Resolver::new(lookup.clone())), routes.clone());
        let hub = Arc::new(DisplayHub::default());
        let old = sources.source(&resolved().reference).await.unwrap();
        let old_watch = sources
            .watch("alfonso:s1", Some("broca"), hub.clone())
            .await
            .unwrap();
        lookup.0.store(true, Ordering::Relaxed);
        let new = sources.source(&resolved().reference).await.unwrap();
        assert!(!Arc::ptr_eq(&old, &new));
        assert!(!old.activity_feed_live());
        assert_eq!(route.detaches.load(Ordering::Relaxed), 1);
        assert!(!old.observe(json!({"kind":"control", "unit":{"type":"RunStarted"}})));
        let _new_watch = sources
            .watch("alfonso:s1", Some("broca"), hub.clone())
            .await
            .unwrap();
        drop(old_watch);
        assert!(hub.projection("alfonso:s1").is_some());
        assert_eq!(
            *routes.1.lock().unwrap(),
            vec![
                std::path::PathBuf::from("/workspace/resolved-project"),
                std::path::PathBuf::from("/workspace/moved")
            ]
        );
    }

    mod bounded_reads;
    mod call_pins;
    mod client_message_ids;
    mod contract_0748_samples;
    mod contract_goldens;
    mod contract_heads;
    mod describe_samples;
    mod empty_claim_samples;
    mod files_samples;
    mod model_names_samples;
    mod opencode2_pages;
    mod outside_window;
    mod part_keys;
    mod pi_display;
    mod pi_pages;
    mod pi_samples;
    #[path = "../../../../../occipital-app-reader/tests/fixtures/broca_0_7.rs"]
    pub(crate) mod samples_0_7;
    pub(crate) mod served_samples;
    mod session_facts;
    mod tag_rows;
    mod tool_display_samples;
    mod tool_times_samples;
}
