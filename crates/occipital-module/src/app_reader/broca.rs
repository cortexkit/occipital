//! Owner-free Broca reads and one lingering feed shared by transcript/display watches.

mod metadata;
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
    capabilities,
    describe::{check_describe, RoleDescribe},
    display::DisplayPage,
    history::{HistoryForm, HistoryRequest},
    plan_view::{PlanView, PlanViewRequest},
    read::{MediaMode, ReadRequest, SenderGrant, ToolMedia},
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
use subc_protocol::RouteTarget;
use tokio::sync::{broadcast, watch};

use crate::display::call_pins::CallPinSlot;
use crate::notices::NoticeSlot;

use super::{
    display_hub::DisplayHub,
    resolve::{
        served_harnesses, DaemonEntorhinal, Resolved, Resolver, SessionRef, BROCA_RUNNER_MODULE_ID,
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
    /// this module. Once each is set, pages report to `call_pins` the tool
    /// version (schema pin) the runner stored for each call, which joins a
    /// provider's records to the call, and to `notices` the notices that rode
    /// their calls.
    pub fn live(
        connection: std::path::PathBuf,
        call_pins: CallPinSlot,
        notices: NoticeSlot,
    ) -> Self {
        Self::new(Arc::new(LiveConnector {
            connection,
            call_pins,
            notices,
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
    call_pins: CallPinSlot,
    notices: NoticeSlot,
}

#[async_trait]
impl SourceConnector for LiveConnector {
    async fn connect(&self) -> Result<Arc<BrocaSources>, ReadError> {
        BrocaSources::live(
            &self.connection,
            subc_protocol::BindIdentity::new(crate::MODULE_ID, "app-reader"),
        )
        .await
        .map(|sources| {
            Arc::new(
                sources
                    .with_call_pins(self.call_pins.clone())
                    .with_notices(self.notices.clone()),
            )
        })
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
    async fn display_runner(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<String, ReadError> {
        self.connected()
            .await?
            .display_runner(session, harness)
            .await
    }

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

    async fn prompt_fit(
        &self,
        session: &str,
        harness: Option<&str>,
        mime: &str,
    ) -> Result<Option<super::PromptFit>, ReadError> {
        self.connected()
            .await?
            .prompt_fit(session, harness, mime)
            .await
    }

    async fn media_read(
        &self,
        session: &str,
        harness: Option<&str>,
        sha256: &str,
        offset: u64,
        length: u64,
    ) -> Result<Value, ReadError> {
        self.connected()
            .await?
            .media_read(session, harness, sha256, offset, length)
            .await
    }

    async fn plan_view(&self, session: &str, harness: Option<&str>) -> Result<PlanView, ReadError> {
        self.connected().await?.plan_view(session, harness).await
    }

    async fn message_tool(&self, session: &str, harness: Option<&str>) -> Option<&'static str> {
        self.connected()
            .await
            .ok()?
            .message_tool(session, harness)
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
/// The log identity Broca gives a session it has never written, and no other.
const UNWRITTEN: &str = "unwritten";

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
    /// Bytes of an image or file the session's history references.
    MediaRead,
    /// The system text and tools of the plan in force.
    PlanView,
}

impl ReadOp {
    pub(super) fn method(self) -> &'static str {
        match self {
            Self::Head => "session.head",
            Self::Read => "session.read",
            Self::History => "session.history",
            Self::Status => "run.status",
            Self::Describe => "role.describe",
            Self::MediaRead => "session.media_read",
            Self::PlanView => "session.plan_view",
        }
    }
}

/// The longest a page waits to learn the sizes of the images it shows. Images
/// still unknown then come without a size, and get one on a later read.
const SIZE_WAIT: Duration = Duration::from_secs(1);

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
    /// Which build of the runner serves this session, as the daemon's catalog
    /// names it, or `None` when that isn't known. A cached `role.describe`
    /// answer is kept only while this stays the same.
    async fn runner_build(&self) -> Option<String> {
        None
    }
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

/// Calls and streams to the runners. Every one is made as this module,
/// `reserved:occipital`: the daemon client proves it on each connection (by
/// the connection's registration claim, or else the launch nonce), so no call
/// names an identity of its own, and a claim-bound connection would refuse
/// one that did.
struct DaemonRoutes {
    calls: Arc<SubcConsumer>,
    streams: Arc<SubcConsumer>,
    builds: Arc<RunnerBuilds>,
}

#[async_trait]
impl BrocaRoutes for DaemonRoutes {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(Arc::new(DaemonRoute {
            calls: self.calls.clone(),
            streams: self.streams.clone(),
            resolved: resolved.clone(),
            builds: self.builds.clone(),
        }))
    }

    fn sender(&self, resolved: &Resolved) -> Option<Arc<dyn SendTarget>> {
        Some(Arc::new(DaemonSender {
            calls: self.calls.clone(),
            resolved: resolved.clone(),
        }))
    }
}

/// Calls into one runner session as this module, `reserved:occipital`. The runner
/// checks that attested route principal, never a request field, against the
/// senders the session's plan allows.
struct DaemonSender {
    calls: Arc<SubcConsumer>,
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
    resolved: Resolved,
    builds: Arc<RunnerBuilds>,
}

/// Each runner module's build, as the daemon's catalog lists it: its
/// `module_version`, plus `+<commit>` when its provenance names one. Shared
/// by every session's route, and read again when the catalog's generation
/// moves (see [`crate::catalog_generation`]).
///
/// A runner can't change what it declares in `role.describe` without a
/// restart, and a new build is how it gains or loses an op or group. Nothing
/// else tells this reader that a runner restarted: the daemon client reopens
/// a call route silently, and a session nobody watches has no live stream to
/// lose. So the describe cache keys on this build instead.
#[derive(Default)]
struct RunnerBuilds {
    generation: crate::catalog_generation::CatalogGeneration,
    state: tokio::sync::Mutex<BuildsState>,
}

#[derive(Default)]
struct BuildsState {
    checked: Option<crate::catalog_generation::Checked>,
    builds: HashMap<String, String>,
}

impl RunnerBuilds {
    /// Builds read again when `calls`'s catalog generation moves.
    fn following(calls: Arc<SubcConsumer>) -> Self {
        use crate::catalog_generation::{CatalogGeneration, Held};
        Self {
            generation: CatalogGeneration::follow("runner builds", Arc::new(Held(calls))),
            state: Default::default(),
        }
    }

    /// `runner`'s build, reading the catalog first when the last read is no
    /// longer fresh. A failed read keeps the builds read before, so a daemon
    /// hiccup doesn't make every cache look stale.
    async fn of(&self, calls: &SubcConsumer, runner: &str) -> Option<String> {
        let mut state = self.state.lock().await;
        if !self.generation.fresh(state.checked.as_ref()) {
            state.checked = Some(self.generation.check());
            match calls.catalog_list().await {
                Ok(listed) => {
                    state.builds = listed
                        .modules
                        .into_iter()
                        .filter_map(|module| {
                            let version = module.module_version?;
                            let build = match module
                                .provenance
                                .and_then(|provenance| provenance.build_git_sha)
                            {
                                Some(commit) => format!("{version}+{commit}"),
                                None => version,
                            };
                            Some((module.module_id, build))
                        })
                        .collect();
                }
                Err(error) => {
                    tracing::debug!(%error, "catalog.list for runner builds failed");
                }
            }
        }
        state.builds.get(runner).cloned()
    }
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

/// The runner refusals that reach the app under their own code, because each
/// is a definite answer the app must not ask again for. A media read's: a
/// digest the session's history doesn't reference (`unknown_media`), and an
/// offset or length out of range (`invalid_params`). A plan view's: the session
/// has no plan in force (`no_plan`), or the plan doesn't fit one reply
/// (`too_large`). Every other refusal of a read, and these on other reads,
/// reach it as the runner being unavailable ([`call_error`], [`result`]).
fn final_refusal(op: ReadOp, code: Option<&str>) -> Option<&'static str> {
    match (op, code?) {
        (ReadOp::MediaRead, "unknown_media") => Some("unknown_media"),
        (ReadOp::MediaRead, "invalid_params") => Some("invalid_params"),
        (ReadOp::PlanView, "no_plan") => Some("no_plan"),
        (ReadOp::PlanView, "too_large") => Some("too_large"),
        _ => None,
    }
}

/// A plan view the runner can't serve yet for this session, and will from the
/// session's next applied plan: runner contract §4.9's `transient` with
/// `detail.cause: "awaiting_plan_applied"`. The app gets `context_not_ready`, so it
/// says the view isn't available for this agent yet rather than that the runner
/// is away, and reads again the next time the person opens the app's view of
/// the session's model context, never on a timer. Every
/// other `transient` still reaches it as the runner being unavailable.
fn not_ready(op: ReadOp, code: Option<&str>, detail: Option<&Value>) -> Option<&'static str> {
    let awaiting =
        detail.and_then(|detail| detail["cause"].as_str()) == Some("awaiting_plan_applied");
    (matches!(op, ReadOp::PlanView) && code == Some("transient") && awaiting)
        .then_some("context_not_ready")
}

/// A `too_large` plan-view refusal does not say which tools the plan contains.
const NO_MESSAGE_TOOL: [&str; 2] = ["unsupported", "no_plan"];

/// The app code a runner's refusal of `op` keeps, if any: [`final_refusal`], or
/// [`not_ready`].
fn own_refusal(op: ReadOp, code: Option<&str>, detail: Option<&Value>) -> Option<&'static str> {
    final_refusal(op, code).or_else(|| not_ready(op, code, detail))
}

/// A failed call of `op` to `runner`, as the app sees it: under its own code
/// when [`own_refusal`] keeps one, or as a chat that moved away
/// ([`held_elsewhere`]), else as [`call_error`] says.
fn read_call_error(op: ReadOp, runner: &str, error: CallError) -> ReadError {
    let detail = match &error {
        CallError::Module(body) => body.detail.as_ref(),
        _ => None,
    };
    if let Some(moved) = held_elsewhere(error.code(), detail) {
        return moved;
    }
    match own_refusal(op, error.code(), detail) {
        Some(code) => ReadError::new(code, format!("{runner}: {error}")),
        None => call_error(runner, error),
    }
}

/// A read of a session whose agent moved to another computer, where the
/// runner here no longer serves it. It's final on this computer, so the app
/// gets the same `session_held_elsewhere` a send to it gets, naming the new
/// computer when the runner does, and looks the agent up again instead of
/// retrying as it would for a runner that's away.
fn held_elsewhere(code: Option<&str>, detail: Option<&Value>) -> Option<ReadError> {
    if code != Some(crate::moves::SESSION_HELD_ELSEWHERE) {
        return None;
    }
    let moved = ReadError::new(
        crate::moves::SESSION_HELD_ELSEWHERE,
        "this chat's agent lives on another computer now",
    );
    let holder = crate::moves::runner_holder(detail);
    Some(if holder.is_empty() {
        moved
    } else {
        moved.with_detail(json!({ "holder": holder }))
    })
}

/// A runner's reply: its result, or its refusal as the app sees it.
fn result(value: Value) -> Result<Value, ReadError> {
    if let Some(code) = value.get("code").and_then(Value::as_str) {
        if let Some(moved) = held_elsewhere(Some(code), value.get("detail")) {
            return Err(moved);
        }
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
                read_call_error(op, &self.resolved.reference.runner, error)
            })?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
        if matches!(op, ReadOp::History) {
            if let Some(code) = value.get("code").and_then(Value::as_str) {
                tracing::debug!(code, "Broca display history refused");
            }
        }
        if let Some(code) = own_refusal(
            op,
            value.get("code").and_then(Value::as_str),
            value.get("detail"),
        ) {
            let mut error = ReadError::new(
                code,
                format!(
                    "{} {code}: {}",
                    self.resolved.reference.runner, value["message"]
                ),
            );
            if let Some(detail) = value.get("detail").filter(|detail| !detail.is_null()) {
                error = error.with_detail(detail.clone());
            }
            return Err(error);
        }
        result(value).map_err(|mut error| {
            error.message = format!("{} {}", self.resolved.reference.runner, error.message);
            error
        })
    }

    async fn runner_build(&self) -> Option<String> {
        self.builds
            .of(&self.calls, &self.resolved.reference.runner)
            .await
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
    metadata: Arc<Mutex<metadata::Metadata>>,
    call_pins: CallPinSlot,
    notices: NoticeSlot,
    /// The image sizes every session's pages share.
    sizes: Arc<crate::media::Sizes>,
}

type SourceSlot = Arc<tokio::sync::Mutex<Weak<BrocaSource>>>;

/// A session's successfully checked `role.describe` answer, cached and shared by
/// every source built for the session until its route is lost or rebound, or
/// the runner's build changes. Two reads that miss the cache at once may each
/// ask; either answer is kept. A plain read builds a fresh source each time, so
/// the cache lives here rather than in the source.
type DescribeSlot = Arc<Mutex<Option<KeptDescribe>>>;

/// A cached `role.describe` answer and the runner build it came from
/// ([`BrocaRoute::runner_build`]).
#[derive(Clone)]
struct KeptDescribe {
    describe: Arc<RoleDescribe>,
    build: Option<String>,
}

impl BrocaSources {
    pub fn new(resolver: Arc<Resolver>, routes: Arc<dyn BrocaRoutes>) -> Self {
        Self {
            resolver,
            routes,
            sources: tokio::sync::Mutex::new(HashMap::new()),
            metadata: Arc::default(),
            call_pins: CallPinSlot::default(),
            notices: NoticeSlot::default(),
            sizes: Arc::default(),
        }
    }

    /// Report each built page's call pins to `call_pins` once it is set.
    pub fn with_call_pins(mut self, call_pins: CallPinSlot) -> Self {
        self.call_pins = call_pins;
        self
    }

    /// Report the notices that rode each page's calls, and those the live
    /// stream delivers, to `notices` once it is set.
    pub fn with_notices(mut self, notices: NoticeSlot) -> Self {
        self.notices = notices;
        self
    }

    pub async fn live(
        connection: &Path,
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
        })));
        Self::connect(connection, resolver).await
    }
    pub async fn connect(connection: &Path, resolver: Arc<Resolver>) -> Result<Self, ReadError> {
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
                builds: Arc::new(RunnerBuilds::following(calls.clone())),
                calls,
                streams,
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
                Ok(()) => {
                    self.metadata.guard().touch(reference);
                    return Ok(source);
                }
                Err(error) if error.code == "session_binding_changed" => source.retire().await,
                Err(error) => return Err(error),
            }
        }
        // A Broca session Entorhinal has no record of is read straight from
        // Broca, which decides whether it exists and whether this module may
        // read it. Only reads come this way: sends, uploads and Stop resolve
        // through Entorhinal themselves and still refuse it.
        let (resolved, recorded) = match self.resolver.resolve(reference).await {
            Ok(resolved) => (resolved, true),
            Err(error) if super::resolve::unrecorded_broca(reference, &error) => {
                (Resolved::unrecorded_broca(reference.clone()), false)
            }
            Err(error) => return Err(error),
        };
        let route = self.routes.open(&resolved).await?;
        let sendable = recorded && self.routes.sender(&resolved).is_some();
        let (lease, describe, message_tool) =
            metadata::Metadata::acquire(&self.metadata, reference);
        let source = BrocaSource::with_send_path(resolved, route, sendable, describe);
        let _ = source.metadata.set(lease);
        *source.message_tool.guard() = message_tool;
        source.recorded.store(recorded, Ordering::Release);
        *source.resolver.guard() = Some(self.resolver.clone());
        *source.call_pins.guard() = self.call_pins.clone();
        *source.notices.guard() = self.notices.clone();
        *source.sizes.guard() = self.sizes.clone();
        *slot = Arc::downgrade(&source);
        Ok(source)
    }
}

impl BrocaSources {
    /// The session the phone named, under the runner that serves it. A session named
    /// without a harness, or as Broca's, is a Broca session. Any other harness is routed
    /// by the runner Entorhinal records for the session: Thalamus's two runners (one for
    /// OpenCode 2 and Claude Code, one for Pi) are read like Broca, and any other runner,
    /// or none, is refused
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
                if runner != BROCA_RUNNER_MODULE_ID && !served_harnesses(&runner).is_empty() =>
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
    async fn display_runner(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<String, ReadError> {
        let reference = self.reference(session, harness).await?;
        match self.resolver.resolve(&reference).await {
            Ok(_) => {}
            Err(error) if super::resolve::unrecorded_broca(&reference, &error) => {
                // Only a successful read from Broca admits an unregistered chat.
                self.source(&reference).await?.head().await?;
            }
            Err(error) => return Err(error),
        }
        Ok(reference.runner)
    }

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

    async fn prompt_fit(
        &self,
        session: &str,
        harness: Option<&str>,
        mime: &str,
    ) -> Result<Option<super::PromptFit>, ReadError> {
        let reference = self.reference(session, harness).await?;
        let source = self.source(&reference).await?;
        // A runner without the group would ignore the send's `content` and drop
        // the file silently, so its description must name the group.
        let Some(describe) = source
            .describe()
            .await
            .filter(|describe| describe.declares("attachments"))
        else {
            return Ok(None);
        };
        let input = source.head().await?.input;
        if input.accepts(mime) == Some(false) {
            return Ok(None);
        }
        Ok(Some(super::PromptFit {
            media_only: describe.declares("media_only_send"),
            max_image_bytes: input.max_image_bytes,
            max_image_dimension: input.max_image_dimension,
        }))
    }

    async fn media_read(
        &self,
        session: &str,
        harness: Option<&str>,
        sha256: &str,
        offset: u64,
        length: u64,
    ) -> Result<Value, ReadError> {
        let reference = self.reference(session, harness).await?;
        self.source(&reference)
            .await?
            .media_read(sha256, offset, length)
            .await
    }

    async fn plan_view(&self, session: &str, harness: Option<&str>) -> Result<PlanView, ReadError> {
        let reference = self.reference(session, harness).await?;
        self.source(&reference).await?.plan_view().await
    }

    async fn message_tool(&self, session: &str, harness: Option<&str>) -> Option<&'static str> {
        let reference = self.reference(session, harness).await.ok()?;
        self.source(&reference).await.ok()?.message_tool().await
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
            display: Some(hub.runner(&reference.runner)),
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
    display_targets: Mutex<Vec<(String, Weak<super::display_hub::RunnerDisplay>)>>,
    live: watch::Sender<bool>,
    activity: Mutex<Option<(&'static str, i64)>>,
    /// The rich UI stamp of the run the last head read found still running, as the
    /// apps read it (`{dialect, catalogDigest}`, from `broca_entries::rich_ui`), with
    /// that run's id. The feed subscribes from the live tail, so a run already going
    /// when it attaches never replays its `run_started`; this seeds its stamp, and
    /// its id lets a late end of an earlier run pass it by.
    running_rich_ui: Mutex<Option<(Value, Option<String>)>>,
    mids: Mutex<HashSet<String>>,
    feed: tokio::sync::Mutex<Option<Feed>>,
    feed_live: AtomicBool,
    /// Whether this module has a send path to this session. Without one, the apps'
    /// send refuses it, so replies say the app may not send whatever the runner grants.
    sendable: bool,
    /// Whether Entorhinal records this session. One it doesn't is read only
    /// (see [`Resolved::unrecorded_broca`]), and is read again through
    /// Entorhinal once it is recorded.
    recorded: AtomicBool,
    describe: DescribeSlot,
    metadata: std::sync::OnceLock<metadata::Lease>,
    /// Where the pins of the calls this source's pages carry are kept.
    call_pins: Mutex<CallPinSlot>,
    /// Where the notices that rode this session's calls are reported.
    notices: Mutex<NoticeSlot>,
    /// The sizes of images this source's pages show.
    sizes: Mutex<Arc<crate::media::Sizes>>,
    /// Whether the plan gives the agent prefrontal's `say`, kept for the
    /// session: rows draw its calls as messages only then.
    message_tool: Mutex<super::message_tool::MessageToolSlot>,
}

mod windows;

impl BrocaSource {
    /// The phone's rows for stored entries, by the session's message shape.
    fn rows(
        &self,
        entries: &[Value],
        full: bool,
        pruned_reasoning: &[PrunedReasoning],
    ) -> Result<Vec<Row>, ReadError> {
        self.rows_in_window(entries, entries.len(), full, pruned_reasoning)
    }

    fn rows_in_window(
        &self,
        entries: &[Value],
        served: usize,
        full: bool,
        pruned_reasoning: &[PrunedReasoning],
    ) -> Result<Vec<Row>, ReadError> {
        let profile = match self.harness() {
            "opencode2" => Some(broca_entries::DisplayProfile::OpenCode2),
            "claude-code" => Some(broca_entries::DisplayProfile::ClaudeCode),
            "pi" => Some(broca_entries::DisplayProfile::Pi),
            _ => None,
        };
        let mut rows = match profile {
            Some(profile) => broca_entries::display_rows(entries, full, profile, pruned_reasoning),
            None => broca_entries::rows_with_pruned_reasoning(entries, full, pruned_reasoning),
        }?;
        // Look-ahead supplies outcomes only. Its rows must not be decorated,
        // recorded as seen, or included in the reply or its cursors.
        rows.truncate(served);
        let entries = &entries[..served];
        let call_pins = self.call_pins.guard().clone();
        if let Some(call_pins) = call_pins.get() {
            let calls: Vec<_> = rows
                .iter()
                .flat_map(|row| occipital_app_reader::tool_times::call_identities(&row.display))
                .collect();
            if !calls.is_empty() {
                let reference = self.resolved.guard().reference.clone();
                call_pins.seen(&reference.runner, &reference.session, calls);
            }
        }
        // Each `say` call's message record, filed under the runner that minted
        // the call's key, so read while its carrier is still on the part.
        if let Some(call_pins) = call_pins.get() {
            let runner = self.resolved.guard().reference.runner.clone();
            for row in &mut rows {
                for (call_key, carrier) in occipital_app_reader::say::marked_calls(&row.display) {
                    let minted_by = carrier.as_deref().unwrap_or(&runner);
                    if let Some(record) = call_pins.message_record(minted_by, &call_key) {
                        occipital_app_reader::say::attach_record(
                            &mut row.display,
                            &call_key,
                            record,
                        );
                    }
                }
            }
        }
        // A moved call's minting runner is this module's to know, not the app's.
        for row in &mut rows {
            occipital_app_reader::tool_times::take_carriers(&mut row.display);
        }
        let notices = self.notices.guard().clone();
        if let Some(notices) = notices.get() {
            let reference = self.resolved.guard().reference.clone();
            notices.seen(crate::notices::from_entries(
                entries,
                &reference.runner,
                &reference.session,
            ));
            notices.sent(crate::notices::wakes::sends_of(
                entries,
                &reference.runner,
                &reference.session,
            ));
        }
        mark_module_messages(&mut rows, entries);
        Ok(rows)
    }

    /// Bytes of an image or file this session's history references, from its
    /// runner's `session.media_read`: `{data?, size, mime}`. A runner without the
    /// contract's `attachments` group serves no media, and the read is refused
    /// `unsupported` without asking it.
    pub async fn media_read(
        &self,
        sha256: &str,
        offset: u64,
        length: u64,
    ) -> Result<Value, ReadError> {
        let describe = self.describe().await.ok_or_else(|| {
            ReadError::new(
                "provider_unavailable",
                "this session's runner can't describe itself now",
            )
        })?;
        if !describe.declares("attachments") {
            return Err(ReadError::new(
                "unsupported",
                "this session's runner serves no media",
            ));
        }
        let request =
            cortexkit_role_llm_runner::attachments::MediaReadRequest::new(sha256, offset, length);
        self.remote_read(ReadOp::MediaRead, json_of(&request)?)
            .await
    }

    /// Draw the rows' `say` calls as the agent's messages where the plan gives
    /// the agent prefrontal's `say`, and as tool calls elsewhere
    /// (`occipital_app_reader::say`). The plan is read only for rows that have
    /// a `say` call, and its answer is kept for a minute.
    ///
    /// Rows whose `say` calls can't be settled yet are refused, retryably,
    /// rather than served either way: an app keeps a page until its version
    /// moves, and settling the calls doesn't move it, so a guess would stay on
    /// the app's screen after the plan can be read again.
    async fn settle_say(&self, rows: &mut [Row]) -> Result<(), ReadError> {
        if !occipital_app_reader::say::any_marked(rows.iter().map(|row| &row.display)) {
            return Ok(());
        }
        let tool = self.read_message_tool().await.map_err(|error| {
            ReadError::new(
                "provider_unavailable",
                format!(
                    "this session's plan can't be read now, which says how its say calls draw: {}",
                    error.message
                ),
            )
        })?;
        for row in rows {
            occipital_app_reader::say::resolve(&mut row.display, tool.is_some());
        }
        Ok(())
    }

    /// The tool the agent talks to the person through, as `session.details`
    /// names it: none while the session's plan can't be read and no earlier
    /// answer is kept (see [`Self::read_message_tool`]).
    pub async fn message_tool(&self) -> Option<&'static str> {
        self.read_message_tool().await.ok().flatten()
    }

    /// Make the kept message-tool answer due for a fresh plan read, as a minute
    /// passing would, so a test can read past it without waiting.
    #[cfg(test)]
    pub(crate) fn expire_message_tool(&self) {
        let slot = self.message_tool.guard().clone();
        let mut kept = slot.guard();
        if let Some((at, _)) = kept.as_mut() {
            if let Some(earlier) = at.checked_sub(super::message_tool::FRESH) {
                *at = earlier;
            }
        }
    }

    /// The tool the agent talks to the person through, from the session's
    /// kept answer or its plan (`super::message_tool`). A runner that serves
    /// no plan view, or holds no plan for the session, answers none. A plan
    /// read that fails for now gives the answer last kept, however old, and
    /// isn't kept, so the next read asks again; with none kept, it's `Err`.
    async fn read_message_tool(&self) -> Result<Option<&'static str>, ReadError> {
        let slot = self.message_tool.guard().clone();
        let kept = *slot.guard();
        if let Some((_, tool)) = kept.filter(|(at, _)| at.elapsed() < super::message_tool::FRESH) {
            return Ok(tool);
        }
        let tool = match self.plan_view().await {
            Ok(view) => super::message_tool::from_plan(&view),
            Err(error) if NO_MESSAGE_TOOL.contains(&error.code) => None,
            Err(error) => {
                if let Some((_, tool)) = kept {
                    return Ok(tool);
                }
                tracing::warn!(code = error.code, message = %error.message,
                    "say calls wait on the session's plan, which can't be read now");
                return Err(error);
            }
        };
        *slot.guard() = Some((std::time::Instant::now(), tool));
        Ok(tool)
    }

    /// The system text and tools of the plan in force, from the runner's
    /// `session.plan_view`. The reply is checked against the contract's closed
    /// type, which refuses item ranges outside the text, so a reply that fails is
    /// never passed on. A runner without the contract's `plan_view` group isn't
    /// asked, and the read is refused `unsupported`.
    pub async fn plan_view(&self) -> Result<PlanView, ReadError> {
        // A description that can't be read now says nothing about the
        // runner, so it's refused for now, not as unsupported, which is final.
        let Some(describe) = self.describe().await else {
            return Err(ReadError::new(
                "provider_unavailable",
                "this session's runner can't describe itself now",
            ));
        };
        if !describe.declares(capabilities::PLAN_VIEW) {
            return Err(ReadError::new(
                "unsupported",
                "this session's runner doesn't serve its plan's system text and tools",
            ));
        }
        let value = self
            .remote_read(ReadOp::PlanView, json_of(&PlanViewRequest::new())?)
            .await?;
        serde_json::from_value(value).map_err(|error| {
            ReadError::new(
                "invalid_provider_reply",
                format!("session.plan_view: {error}"),
            )
        })
    }

    /// Give each image on `rows` its drawn size: from the sizes already learned,
    /// and, for images not yet read, from their first bytes, read one after
    /// another until [`SIZE_WAIT`] runs out. A read that fails leaves the image
    /// unknown, to be read again on a later page; bytes that hold no size this
    /// module reads are remembered as such.
    async fn size_images(&self, rows: &mut [Row]) {
        let sizes = self.sizes.guard().clone();
        let unknown: Vec<String> = rows
            .iter()
            .flat_map(|row| sizes.unknown(&row.display))
            .collect();
        if !unknown.is_empty() {
            let deadline = tokio::time::Instant::now() + SIZE_WAIT;
            for sha256 in unknown {
                if sizes.get(&sha256).is_some() {
                    continue;
                }
                let read = self.media_read(&sha256, 0, crate::media::HEADER_BYTES);
                match tokio::time::timeout_at(deadline, read).await {
                    Err(_) => break,
                    Ok(Err(error)) if error.code == "unsupported" => break,
                    Ok(Err(_)) => {}
                    Ok(Ok(reply)) => {
                        if let Ok(piece) = crate::media::media_piece(reply) {
                            sizes.put(&sha256, crate::media::header_size(&piece.bytes));
                        }
                    }
                }
            }
        }
        for row in rows {
            sizes.apply(&mut row.display);
        }
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
            recorded: AtomicBool::new(true),
            describe,
            metadata: Default::default(),
            call_pins: Mutex::new(CallPinSlot::default()),
            notices: Mutex::new(NoticeSlot::default()),
            sizes: Mutex::default(),
            message_tool: Mutex::default(),
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
        let hub = hub.runner(&self.runner());
        let weak = Arc::downgrade(&hub);
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
        let runner = self.runner();
        let stream =
            super::observe::remote(&runner, "session.subscribe", self.route.subscribe(live_ids))
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
        self.display_input(DisplayInput::Restart);
        // Events queued since the subscribe come after this, so a run that started or
        // ended after the cold read still sets or clears the stamp in order.
        let running = self.running_rich_ui.guard().clone();
        if let Some((rich_ui, run_id)) = running {
            self.display_input(DisplayInput::RunStart {
                rich_ui: Some(rich_ui),
                run_id,
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
            let current = match resolver.resolve(&reference).await {
                Ok(current) => current,
                // Still unrecorded: nothing to refresh.
                Err(error)
                    if !self.recorded.load(Ordering::Acquire)
                        && super::resolve::unrecorded_broca(&reference, &error) =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            // Recorded since it was read without a record: open it again through
            // Entorhinal, so it gains its folder, network facts and send path.
            if !self.recorded.load(Ordering::Acquire) {
                self.retired.store(true, Ordering::Release);
                self.lost();
                return Err(ReadError::new(
                    "session_binding_changed",
                    format!("Broca session {} is now recorded", reference.session),
                ));
            }
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
        self.display_input(DisplayInput::Restart);
        for (session, target) in self.display_targets.guard().drain(..) {
            if let Some(hub) = target.upgrade() {
                hub.remove(&session);
            }
        }
        self.bump();
    }

    async fn remote_read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        let runner = self.runner();
        super::observe::remote(&runner, op.method(), self.route.read(op, params)).await
    }

    /// The module id of the runner this session is on, which names its calls in
    /// the request log (`thalamus-harness.session.read`, `broca.session.head`).
    fn runner(&self) -> String {
        self.resolved.guard().reference.runner.clone()
    }

    async fn read_head(&self) -> Result<Head, ReadError> {
        let value = self.remote_read(ReadOp::Head, json!({})).await?;
        let meta = head_meta(&value)?;
        // Broca answers a session it never wrote as an empty one, with log
        // identity `unwritten`, so a head can be read before its first send. A
        // session Entorhinal records may be such a head; one it doesn't record
        // and Broca never wrote is no session at all, such as an id the app
        // named under the wrong harness, and is refused rather than drawn as an
        // empty chat.
        if meta.incarnation == UNWRITTEN && !self.recorded.load(Ordering::Acquire) {
            let session = self.resolved.guard().reference.session.clone();
            return Err(ReadError::new(
                "session_unknown",
                format!("no session {session} on runner broca"),
            )
            .with_detail(json!({"runner": null})));
        }
        // Only a written session has a durable transcript update time.
        let at = meta.updated_at.and_then(|at| i64::try_from(at).ok());
        *self.activity.guard() = at.and_then(|at| {
            meta.last_run_state
                .as_ref()
                .and_then(|run| activity_for(Some(&run.state)))
                .map(|activity| (activity, at))
        });
        let running = meta
            .last_run_state
            .as_ref()
            .filter(|run| activity_for(Some(&run.state)) == Some("busy"));
        *self.running_rich_ui.guard() = meta
            .last_run_rich_ui
            .as_ref()
            .filter(|_| running.is_some())
            .and_then(|stamp| serde_json::to_value(stamp).ok())
            .as_ref()
            .and_then(occipital_app_reader::broca_entries::rich_ui)
            .map(|stamp| (stamp, running.map(|run| run.run_id.clone())));
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
        // A head without `sender` needs the capability declaration to tell no
        // grant from an older runner's silence, and a granted one to say whether
        // the runner takes "Send now". It's read once per session and cached;
        // display history consults the same declaration.
        let describe = if self.sendable {
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
                max_image_bytes: value["input"]["max_image_bytes"].as_u64(),
                max_image_dimension: value["input"]["max_image_dimension"].as_u64(),
            },
            model: meta.model.as_ref().map(app_model).transpose()?,
            // Passed on as the head states it. A runner declaring `head_selection`
            // leaves it out when it is standard; an app that offers the choice
            // knows the runner selects heads, so it reads a missing tier as
            // standard there. Reading the description to say so here would add a
            // runner call to every head read.
            service_tier: meta.service_tier.clone(),
            history_intake: meta.history_intake_running(),
            host: meta.host.clone().map(String::from),
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

    /// The runner's checked `role.describe`, cached while the runner's build
    /// stays the same: a runner placed on a new build is asked again, so an op
    /// or group it gains or loses counts from the next read. A failed or
    /// malformed answer isn't kept, so the next read asks again; until then it
    /// reads as `None`, even when an answer from an earlier build is held.
    async fn describe(&self) -> Option<Arc<RoleDescribe>> {
        let build = self.route.runner_build().await;
        if let Some(kept) = self.describe.guard().clone() {
            if kept.build == build {
                return Some(kept.describe);
            }
        }
        let value = self.remote_read(ReadOp::Describe, json!({})).await.ok()?;
        let describe = Arc::new(check_describe(&value).ok()?);
        *self.describe.guard() = Some(KeptDescribe {
            describe: describe.clone(),
            build,
        });
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
                page.messages.len(),
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
        // A notice riding a tool result changes no message, so it neither wakes
        // the transcript watches nor goes to the display hub that streams live
        // text: it's kept for the notices read, and the notices watch says so.
        if name == cortexkit_role_llm_runner::live::event_types::NOTICE_DELIVERED {
            let reference = self.resolved.guard().reference.clone();
            let notices = self.notices.guard().clone();
            if let (Some(notices), Some(notice)) = (
                notices.get(),
                crate::notices::from_event(event, &reference.runner, &reference.session),
            ) {
                notices.seen(vec![notice]);
            }
            return true;
        }
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
        // A display `run_status` says the live run's state changed, such as a
        // provider retry starting or ending (llm-runner 0.8.47). It carries no
        // record of its own, so the chat's watches re-read the head, which is.
        if normalized["kind"] == "display"
            && name == cortexkit_role_llm_runner::live::event_types::RUN_STATUS
        {
            self.bump();
        }
        if normalized["kind"] == "control" {
            if matches!(name.as_str(), "run_started" | "run_finished" | "run_paused") {
                // A control event wakes the reader; only the next durable head
                // can supply a timestamp, never the wall clock of this observer.
                *self.activity.guard() = None;
            }
            self.bump();
        }
        if let Some(mut input) = display_events::map(&normalized) {
            if input.tool_name() == Some(super::message_tool::SAY) {
                input.mark_message_tool(self.kept_message_tool());
            }
            self.display_input(input);
        }
        true
    }

    /// The tool the agent talks to the person through, as last read from its
    /// plan however long ago: a live event can't wait for a plan read. Page
    /// reads keep the answer fresh while the chat holds such calls, and the
    /// page read that opens the live feed is one of them. With nothing read
    /// yet, a call's text isn't shown live, and its message shows whole once
    /// stored.
    fn kept_message_tool(&self) -> Option<&'static str> {
        let slot = self.message_tool.guard().clone();
        let kept = *slot.guard();
        kept.and_then(|(_, tool)| tool)
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
        // Why the feed ends, for the log: a feed that ends while apps watch
        // leaves their live text without a source until one reopens it.
        let ended: std::borrow::Cow<'static, str> = tokio::select! {
            biased;
            changed = users.changed() => { if changed.is_ok() { continue; } else { "no_watchers".into() } }
            _ = async { match deadline { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => "linger_elapsed".into(),
            event = stream.next() => match event {
                Ok(Some(event)) => {
                    let kind = event_kind(&event);
                    if source.observe(event) { continue; }
                    format!("event_unreadable kind={kind}").into()
                }
                Ok(None) => "stream_ended".into(),
                Err(error) => format!("stream_failed code={} {}", error.code, error.message).into(),
            }
        };
        // Serialize linger expiry with acquisition so a reconnect at the deadline
        // cannot receive a lease for a feed that is already being detached.
        let _feed = source.feed.lock().await;
        if ended == "linger_elapsed" && *users.borrow() != 0 {
            continue;
        }
        let reference = source.resolved.guard().reference.clone();
        tracing::info!(target: "app_reader", runner = %reference.runner,
            session = %reference.session, reason = %ended, "live subscription ended");
        source.lost();
        break;
    }
}

/// A live frame's kind and event type, such as `display/text_delta`, naming
/// what it is without any of its text.
fn event_kind(envelope: &Value) -> String {
    let kind = envelope["kind"].as_str().unwrap_or("?");
    let event = envelope["event"]["type"]
        .as_str()
        .or_else(|| envelope["unit"]["type"].as_str())
        .unwrap_or("?");
    format!("{kind}/{event}")
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

/// Mark each row a module other than this one sent, from the runner's display
/// facts on its entry (llm-runner's `message_senders` and `message_marks`), so
/// the apps never draw it as the person's own message:
///
/// - `speaksFor: {agentId}`: another agent's words the module carried, such as
///   a head's brief or steer to its worker, or a peer message to a head. The
///   apps draw it as that agent's.
/// - `notice: {sender, deliveredAs}`: the module's own message to the agent,
///   such as prefrontal's status line or Magic Context's reminders. The apps fold it.
///   `deliveredAs` is the runner's delivery mode, when it names one.
/// - `sentBy: {module}`: a module's message with neither mark. The apps show it
///   labelled with the module, so nothing is hidden on a sender's mistake.
///
/// The person's messages come through this module, so a row it sent, or one
/// whose entry names no sender, gets no mark. Neither does a relayed message:
/// its `origin` already says which module relayed the person's words.
fn mark_module_messages(rows: &mut [Row], entries: &[Value]) {
    let by_mid: HashMap<&str, &Value> = entries
        .iter()
        .filter_map(|entry| entry["mid"].as_str().map(|mid| (mid, entry)))
        .collect();
    for row in rows {
        let Some(entry) = by_mid.get(row.mid.as_str()) else {
            continue;
        };
        let Some(sender) = entry["sender"].as_str().filter(|sender| !sender.is_empty()) else {
            continue;
        };
        if sender == crate::MODULE_ID || row.display.get("origin").is_some() {
            continue;
        }
        if let Some(agent) = entry["speaks_for"]["agent_id"]
            .as_str()
            .filter(|agent| !agent.is_empty())
        {
            row.display["speaksFor"] = json!({ "agentId": agent });
        } else if entry["notice"] == true {
            let mut notice = json!({ "sender": sender });
            if let Some(mode) = entry["delivered_as"]["mode"].as_str() {
                notice["deliveredAs"] = json!(mode);
            }
            row.display["notice"] = notice;
        } else {
            row.display["sentBy"] = json!({ "module": sender });
        }
    }
}

/// The `mid` of every entry a runner wrote for a send it delivered with a tool
/// result (`delivered_as.mode` is `tool_result`). Runners following llm-runner
/// 0.8's rule that every delivered send is an entry of its own wrote one for
/// each such send; those entries stay stored, so readers leave out the user
/// rows among them. Broca also marks the tool-result message that carried a
/// send this way. That row holds the tool's result, so it stays.
fn tool_result_rides(entries: &[Value]) -> HashSet<String> {
    entries
        .iter()
        .filter(|entry| {
            entry["delivered_as"]["mode"]
                == cortexkit_role_llm_runner::delivery_order::modes::TOOL_RESULT
        })
        .filter_map(|entry| entry["mid"].as_str().map(str::to_owned))
        .collect()
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
            empty_before: self
                .entries
                .first()
                .filter(|_| self.rows.is_empty() && self.older)
                .and_then(|entry| entry["ordinal"].as_u64()),
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
    fn recorded(&self) -> bool {
        self.recorded.load(Ordering::Acquire)
    }

    fn harness(&self) -> &'static str {
        match self.resolved.guard().harness.as_str() {
            "opencode2" => "opencode2",
            "claude-code" => "claude-code",
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
                    empty_before: None,
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
        self.settle_say(&mut rows).await?;
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
    /// The wake rows among `rows` whose record holds the person's answer to
    /// an ask, from the wake records prefrontal wrote before each wake. The
    /// rows were read from this session, which named each one's send then.
    /// Records that can't be read now answer none, so those rows stay out of
    /// this read only.
    async fn answered_asks(&self, rows: &[Row]) -> std::collections::HashSet<String> {
        let wakes: Vec<&str> = rows
            .iter()
            .filter(|row| row.display.get("notice").is_some())
            .map(|row| row.mid.as_str())
            .collect();
        let notices = self.notices.guard().clone();
        let Some(notices) = notices.get().filter(|_| !wakes.is_empty()) else {
            return std::collections::HashSet::new();
        };
        let reference = self.resolved.guard().reference.clone();
        let records = notices
            .wake_records(
                &reference.runner,
                &reference.session,
                &json!({ "messageIds": wakes }),
                None,
            )
            .await;
        let Ok(records) = records else {
            return std::collections::HashSet::new();
        };
        records["messages"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, record)| {
                record["items"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item["kind"] == "ask_answer"))
            })
            .map(|(mid, _)| mid.clone())
            .collect()
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

/// The media forms a read asks for. A default read renders each stored photo or
/// file as a text marker (`[media not shown: … reason=not_requested]`). A runner
/// declaring `media_refs` serves a message's photo instead as a reference
/// (sha256, mime, size and pixel size), which rows show as a file part, keeping
/// every other payload as a default read serves it. One also declaring
/// `tool_media_refs` does the same for a picture or document in a tool's answer
/// when asked (`tool_media: "refs"`), so the call lists it rather than carrying a
/// marker the model never saw. Any other runner refuses either value, so it's
/// asked for neither and its pages keep the markers.
fn stored_media(describe: Option<&RoleDescribe>) -> (Option<MediaMode>, Option<ToolMedia>) {
    use cortexkit_role_llm_runner::capabilities::{MEDIA_REFS, TOOL_MEDIA_REFS};
    let declares = |capability| describe.is_some_and(|describe| describe.declares(capability));
    if !declares(MEDIA_REFS) {
        return (None, None);
    }
    (
        Some(MediaMode::MediaRefs),
        declares(TOOL_MEDIA_REFS).then_some(ToolMedia::Refs),
    )
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
///
/// A granted sender gains `promote: true` when the app may offer "Send now" for a
/// queued message (`session.promote_send`): the runner's description for this
/// session declares `send_promote`, and the grant includes `steer`, which
/// promotion needs. The contract's description check already refuses
/// `send_promote` without the `steer` group. Without the member the app offers
/// no such button, so a chat whose runner can't move a message never shows one.
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
            Ok(Value::Object(mut sender)) => {
                let steers = sender
                    .get("modes")
                    .and_then(Value::as_array)
                    .is_some_and(|modes| modes.iter().any(|mode| mode == "steer"));
                let promotes = describe.is_some_and(|describe| {
                    describe.declares(cortexkit_role_llm_runner::capabilities::SEND_PROMOTE)
                });
                if steers && promotes {
                    sender.insert("promote".into(), Value::Bool(true));
                }
                SendGrant::Granted(sender)
            }
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
        serde_json::from_value(without_malformed_wait(value).into_owned())
            .map_err(|error| invalid(error.to_string()))?;
    if !meta.snapshot_consistent() {
        return Err(invalid("Broca session.head has an inconsistent snapshot"));
    }
    Ok(meta)
}

/// `head` without its last run's `awaits` when that doesn't decode, such as an
/// `attempt` of 0 or an overlong message. The contract reads an absent wait
/// as no information, so a malformed one costs only the wait, never the
/// whole head: the chat still reads and still takes sends.
pub(crate) fn without_malformed_wait(head: &Value) -> std::borrow::Cow<'_, Value> {
    let wait = &head["last_run_state"]["awaits"];
    if wait.is_null()
        || serde_json::from_value::<cortexkit_role_llm_runner::awaits::RunAwaits>(wait.clone())
            .is_ok()
    {
        return std::borrow::Cow::Borrowed(head);
    }
    tracing::debug!("a runner's last-run wait didn't decode; it is left out");
    let mut head = head.clone();
    if let Some(run) = head
        .get_mut("last_run_state")
        .and_then(Value::as_object_mut)
    {
        run.remove("awaits");
    }
    std::borrow::Cow::Owned(head)
}

fn activity_for(state: Option<&str>) -> Option<&'static str> {
    // The runner contract's own run states (`RunState` in the llm-runner role
    // crate): a running or paused run keeps the chat busy, and every state the
    // crate calls terminal leaves it idle, including a run that ended at its
    // step limit, its token budget or the context ceiling. A state the crate
    // doesn't know says nothing.
    match cortexkit_role_llm_runner::run::RunState::parse(state?) {
        cortexkit_role_llm_runner::run::RunState::Active
        | cortexkit_role_llm_runner::run::RunState::Paused => Some("busy"),
        state if state.is_terminal() => Some("idle"),
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
/// The head's next-send model in the apps' spelling: `{provider, model,
/// variant?, defaultVariant?}`. `defaultVariant` is the reasoning effort the
/// runner sends when no variant is chosen, for the apps to label ("xhigh
/// (default)"); it's kept apart from `variant` so an app that sends the
/// selection back never pins the vendor's default as a choice.
fn app_model(model: &cortexkit_role_llm_runner::read::HeadModel) -> Result<Value, ReadError> {
    let mut value = json_of(model)?;
    if let Some(fields) = value.as_object_mut() {
        if let Some(default) = fields.remove("default_variant") {
            fields.insert("defaultVariant".into(), default);
        }
    }
    Ok(value)
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
pub(crate) mod tests;
