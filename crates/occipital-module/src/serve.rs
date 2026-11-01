//! Occipital on subc: the handler `subc_client_rs` drives.
//!
//! subc-client-rs owns HELLO, route binding, health checks and the frame
//! lifecycle. Routes to the management surface carry the apps' chat
//! operations ([`crate::app_reader`]); routes to the tool provider carry the
//! reply teaching and `show_ui`. HELLO_ACK names the store, and a store that
//! cannot be served stops the process, because serving on without one could
//! only fail every send, upload and record.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(feature = "show-ui")]
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cortexkit_role_tool_provider::{catalog, describe};
use serde::Deserialize;
#[cfg(feature = "show-ui")]
use serde_json::value::RawValue;
use serde_json::Value;
use subc_client_rs::{
    BindDecision, ConsumerIdentity, HandlerOutcome, ModuleHandler, RequestClass, RequestCtx,
    RequestPools, RouteBindRequest, RouteCloseReason, RouteHandle,
};
use subc_protocol::session::{HealthReport, HealthStatus};
use subc_protocol::{ModuleHelloAckBody, RouteTarget};

use crate::app_reader::{self, AppReader, RouteBinding};
#[cfg(feature = "show-ui")]
use crate::show_ui::{self, PreparedCall, RouteStamp};
use crate::startup::{Fatal, EXIT_SERVE_FAILURE, EXIT_STORE_FAILURE};
use crate::store::{self, Store};

/// Admission has one exit into native execution, so a checker cannot receive
/// partially validated arguments or client-supplied provenance.
#[cfg(feature = "show-ui")]
pub trait NativeExecutor: Send + Sync {
    fn execute(&self, call: PreparedCall, store: Option<&Store>) -> HandlerOutcome;
}

fn internal(message: impl Into<String>) -> HandlerOutcome {
    HandlerOutcome::Error {
        code: "internal".into(),
        message: message.into(),
    }
}

fn response(value: &impl serde::Serialize) -> HandlerOutcome {
    match serde_json::to_vec(value) {
        Ok(bytes) => HandlerOutcome::Response(bytes),
        Err(error) => internal(format!("cannot encode response: {error}")),
    }
}

#[derive(Deserialize)]
struct Request {
    name: String,
    arguments: Value,
    // Keep explicit null distinct from an absent descriptor, and defer descriptor
    // validation until after the earlier ordered argument checks.
    #[cfg(feature = "show-ui")]
    #[serde(default, deserialize_with = "raw_member")]
    output_refs: Option<Box<RawValue>>,
}

#[cfg(feature = "show-ui")]
fn raw_member<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Box<RawValue>>, D::Error> {
    Box::<RawValue>::deserialize(deserializer).map(Some)
}

fn discovery(op: &str, arguments: &Value) -> HandlerOutcome {
    use cortexkit_role_tool_provider::{PROVIDES, REQUIRED_OPS};
    if op == "role.describe" {
        let described = describe::RoleDescribe::new(
            vec![describe::Major {
                version: PROVIDES.into(),
                ops: REQUIRED_OPS.iter().map(|op| (*op).into()).collect(),
                stability: "stable".into(),
            }],
            env!("CARGO_PKG_VERSION"),
        );
        // The display server's served limits ride `role.describe` as their own
        // group (tool-display §5), which the tool-provider type doesn't model.
        let mut described = match serde_json::to_value(described) {
            Ok(described) => described,
            Err(error) => return internal(error.to_string()),
        };
        described[cortexkit_role_tool_display::DESCRIBE_GROUP] =
            match serde_json::to_value(crate::display::store::served_limits()) {
                Ok(limits) => limits,
                Err(error) => return internal(error.to_string()),
            };
        return response(&described);
    }
    let request: catalog::CatalogRequest = match serde_json::from_value(arguments.clone()) {
        Ok(request) => request,
        Err(error) => {
            return HandlerOutcome::Error {
                code: "invalid_request".into(),
                message: error.to_string(),
            }
        }
    };
    if request.preset.is_some() {
        return HandlerOutcome::ErrorWithDetail {
            code: "invalid_request".into(),
            message: "occipital serves no presets".into(),
            detail: serde_json::json!({"field":"preset"}),
        };
    }
    if !request.params.is_empty() {
        return HandlerOutcome::ErrorWithDetail {
            code: "invalid_request".into(),
            message: "occipital serves no catalog params".into(),
            detail: serde_json::json!({"field":"params"}),
        };
    }
    let tools = {
        #[cfg(feature = "show-ui")]
        {
            let schema = crate::manifest::show_ui_schema();
            let digest = match catalog::schema_digest(&schema) {
                Ok(digest) => digest,
                Err(error) => return internal(error.to_string()),
            };
            vec![catalog::CatalogTool::new(
                crate::manifest::SHOW_UI,
                digest,
                1,
                schema,
            )]
        }
        #[cfg(not(feature = "show-ui"))]
        Vec::<catalog::CatalogTool>::new()
    };
    let system_text = match request
        .system_text
        .as_ref()
        .map(|item| crate::system_text::answer(item, request.composition.as_ref()))
    {
        None => None,
        Some(Ok(answer)) => Some(answer),
        Some(Err(crate::system_text::Refusal::Invalid { field, message })) => {
            return HandlerOutcome::ErrorWithDetail {
                code: "invalid_request".into(),
                message: message.into(),
                detail: serde_json::json!({ "field": field }),
            }
        }
        Some(Err(crate::system_text::Refusal::Internal(message))) => return internal(message),
    };
    // The catalog digest covers the tools and, when requested, the
    // system-text answer, so a digest-only fetch with the same system-text
    // item notices a changed text. Without a system-text item the digest
    // stays the digest of the tools alone, as before.
    let encoded = match &system_text {
        None => serde_json::to_vec(&tools),
        Some(text) => serde_json::to_vec(&(&tools, text)),
    };
    let encoded = match encoded {
        Ok(bytes) => bytes,
        Err(error) => return internal(error.to_string()),
    };
    let digest = hmac_sha256::Hash::hash(&encoded)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let answer = catalog::CatalogAnswer::new(digest.clone(), digest);
    if request.digest_only == Some(true) {
        return response(&answer);
    }
    let mut answer = answer.with_tools(tools);
    // A runner checks this against the digest of the composition it sent.
    // Nothing Occipital serves depends on the composition; the digest only
    // confirms which one the answer was resolved against.
    if let Some(composition) = &request.composition {
        match catalog::composition_digest(&Value::Object(composition.clone())) {
            Ok(digest) => answer = answer.with_composition_digest(digest),
            Err(error) => return internal(error.to_string()),
        }
    }
    response(&match system_text {
        None => answer,
        Some(text) => answer.with_system_text(text),
    })
}

/// What the process does when startup after HELLO_ACK fails. Production
/// writes the line and exits, so supervision restarts the module; tests
/// record the failure instead, because exiting would end the test binary.
pub type OnFatal = Box<dyn Fn(Fatal) + Send + Sync>;

/// Write the failure's one stderr line and exit with its status.
pub fn exit_with(fatal: Fatal) {
    eprintln!("{}", fatal.line);
    std::process::exit(i32::from(fatal.code));
}

/// What the app reader needs to reach the runners, Entorhinal and the other
/// modules: the daemon's connection file and the identity this process proves
/// with its launch nonce. Absent in tests that serve no reads.
pub struct Consumer {
    pub connection_file: PathBuf,
    pub identity: ConsumerIdentity,
}

pub struct OccipitalHandler {
    store: OnceLock<Arc<Store>>,
    app: OnceLock<Arc<AppReader>>,
    /// Providers' display writes, in their own file beside the store.
    display: OnceLock<crate::display::State>,
    /// Fetches writers' views off the write path; absent without a daemon
    /// connection to reach them.
    display_views: OnceLock<crate::display::views::Fetcher>,
    /// Where the app reader's pages report their calls' pins, set once the
    /// display store is open. The reader is built first, so it holds the slot.
    call_pins: crate::display::call_pins::CallPinSlot,
    /// The apps' tool display reads over the display store, once it is open.
    tool_display: OnceLock<Arc<crate::display::app::ToolDisplay>>,
    consumer: Option<Consumer>,
    /// The connection to other modules, made from `consumer` when first needed.
    daemon_calls: OnceLock<Arc<crate::display::daemon::DaemonCalls>>,
    on_fatal: OnFatal,
    /// What each live route was bound with: admission and `show_ui`'s provenance
    /// read only these daemon-attested facts.
    routes: Mutex<HashMap<RouteHandle, RouteBinding>>,
    /// The app reader's background work, each task with its name: receipt
    /// pruning, the follow-up of held sends, and upload cleanup. They run for
    /// the module's whole life, so one that has finished has failed.
    background: Mutex<Vec<(&'static str, tokio::task::JoinHandle<()>)>>,
    #[cfg(feature = "show-ui")]
    executor: Box<dyn NativeExecutor>,
}

impl OccipitalHandler {
    pub fn new(consumer: Option<Consumer>, on_fatal: OnFatal) -> Self {
        Self {
            store: OnceLock::new(),
            app: OnceLock::new(),
            display: OnceLock::new(),
            display_views: OnceLock::new(),
            call_pins: Default::default(),
            tool_display: OnceLock::new(),
            consumer,
            daemon_calls: OnceLock::new(),
            on_fatal,
            routes: Mutex::new(HashMap::new()),
            background: Mutex::new(Vec::new()),
            #[cfg(feature = "show-ui")]
            executor: Box::new(crate::native::Executor(crate::native::Language)),
        }
    }

    /// Serve the app reader with `app` instead of the one HELLO_ACK builds, so
    /// tests can drive it with their own runners.
    pub fn with_app_reader(self, app: Arc<AppReader>) -> Self {
        let _ = self.app.set(app);
        self
    }

    /// Serve display writes from `display` instead of the file HELLO_ACK opens,
    /// for tests whose daemon names no store.
    pub fn with_display(self, display: crate::display::State) -> Self {
        let _ = self.display.set(display);
        self
    }

    fn binding(&self, handle: RouteHandle) -> Option<RouteBinding> {
        self.routes
            .lock()
            .ok()
            .and_then(|routes| routes.get(&handle).cloned())
    }

    fn dispatch(&self, handle: RouteHandle, body: &[u8]) -> HandlerOutcome {
        let request: Request = match serde_json::from_slice(body) {
            Ok(request) => request,
            Err(error) => {
                return HandlerOutcome::Error {
                    code: "invalid_request".into(),
                    message: error.to_string(),
                }
            }
        };
        if matches!(request.name.as_str(), "role.describe" | "tool.catalog") {
            return discovery(&request.name, &request.arguments);
        }
        #[cfg(feature = "show-ui")]
        {
            self.dispatch_show_ui(handle, request)
        }
        #[cfg(not(feature = "show-ui"))]
        {
            let _ = handle;
            HandlerOutcome::Error {
                code: "unknown_tool".into(),
                message: "ck-occipital serves no tools in this build".into(),
            }
        }
    }

    #[cfg(feature = "show-ui")]
    fn dispatch_show_ui(&self, handle: RouteHandle, request: Request) -> HandlerOutcome {
        if request.name != crate::manifest::SHOW_UI {
            return HandlerOutcome::Error {
                code: "unknown_tool".into(),
                message: "ck-occipital serves only show_ui".into(),
            };
        }
        let stamp = match self.routes.lock() {
            Ok(routes) => routes
                .get(&handle)
                .map(|(principal, _, scope, _, _)| RouteStamp {
                    scope: scope.clone(),
                    principal: principal.clone(),
                })
                .unwrap_or_default(),
            Err(_) => return internal("route stamps are unavailable"),
        };
        let now_ms = match SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|time| i64::try_from(time.as_millis()).ok())
        {
            Some(now) => now,
            None => return internal("receipt time is unavailable"),
        };
        let now = match occipital_lang::Decimal::from_integer(i128::from(now_ms)) {
            Ok(now) => now,
            Err(error) => return internal(error.code()),
        };
        match show_ui::prepare(
            &request.arguments,
            request.output_refs.as_ref().map(|raw| raw.get().as_bytes()),
            &stamp,
            now_ms,
            now,
        ) {
            Ok(call) => self
                .executor
                .execute(call, self.store.get().map(AsRef::as_ref)),
            Err(refusal) => crate::native::refused(refusal),
        }
    }

    /// Build the app reader over the store HELLO_ACK opened, and start its
    /// background work. A table that cannot be opened stops the process, as the
    /// store itself does.
    fn start_app_reader(&self, store: &Store) -> Result<(), Fatal> {
        if self.app.get().is_some() {
            return Ok(());
        }
        let fatal =
            |error: crate::Error| Fatal::new(EXIT_STORE_FAILURE, store::UNAVAILABLE_PREFIX, error);
        let receipts = app_reader::send_store::Receipts::open(store.shared()).map_err(fatal)?;
        let uploads =
            app_reader::uploads::Uploads::open(store.shared(), store.data_root()).map_err(fatal)?;
        let resolver = Arc::new(match &self.consumer {
            Some(consumer) => app_reader::broca::LazyBrocaSources::live(
                consumer.connection_file.clone(),
                consumer.identity.clone(),
                self.call_pins.clone(),
            ),
            None => app_reader::broca::LazyBrocaSources::unavailable(),
        });
        let sends = Arc::new(app_reader::send::Sender::new(
            resolver.clone(),
            Some(receipts),
        ));
        let mut background = sends.start();
        background.push(("upload cleanup", uploads.start()));
        let app = AppReader::with_sender(resolver, sends).with_uploads(Some(uploads));
        if let Some(calls) = self.daemon_calls() {
            app.set_model_names(Arc::new(crate::display::model_names::ModelNames::new(
                Arc::new(crate::display::model_names::DaemonNameSources::new(calls)),
            )));
        }
        if self.app.set(Arc::new(app)).is_ok() {
            if let Ok(mut tasks) = self.background.lock() {
                tasks.extend(background);
            }
        } else {
            background.iter().for_each(|(_, task)| task.abort());
        }
        Ok(())
    }
}

/// Watches are held open for as long as an app shows a chat, so they wait in
/// their own pool and can never take the slots reads and sends need. The pool
/// is a little larger than the watch limit, so the watch past the limit gets
/// its `watch_capacity` refusal at once instead of waiting for a slot.
const WATCH_SLOTS: usize = occipital_app_reader::watches::MAX_OPEN_WATCHES + 4;

#[async_trait]
impl ModuleHandler for OccipitalHandler {
    async fn handle(&self, ctx: RequestCtx, body: Vec<u8>) -> HandlerOutcome {
        let handle = ctx.route_handle();
        let binding = self.binding(handle);
        let to_app_reader = binding.as_ref().is_some_and(|(_, target, ..)| {
            matches!(target, RouteTarget::ManagementSurface { module_id } if module_id == crate::MODULE_ID)
        });
        if !to_app_reader {
            #[derive(Deserialize)]
            struct Named {
                name: String,
                #[serde(default)]
                arguments: Value,
            }
            if let Ok(named) = serde_json::from_slice::<Named>(&body) {
                if named.name == cortexkit_role_tool_display::DISPLAY_WRITE {
                    let writer = binding.and_then(|(principal, ..)| principal);
                    return crate::display::write(
                        self.display.get(),
                        self.display_views.get(),
                        self.tool_display.get().map(|tool| tool.hints()),
                        writer,
                        &named.arguments,
                    )
                    .await;
                }
            }
            return self.dispatch(handle, &body);
        }
        match self.app.get() {
            Some(app) => app.serve(binding, ctx, body).await,
            None => HandlerOutcome::Error {
                code: "provider_unavailable".into(),
                message: "the app reader starts once the daemon names the store; retry".into(),
            },
        }
    }

    async fn on_bind(&self, request: &RouteBindRequest) -> BindDecision {
        match self.routes.lock() {
            Ok(mut routes) => {
                routes.insert(request.handle, app_reader::binding(request));
                BindDecision::accept()
            }
            Err(_) => BindDecision::reject("internal", "route stamps are unavailable"),
        }
    }

    async fn on_route_gone(&self, handle: &RouteHandle) {
        if let Ok(mut routes) = self.routes.lock() {
            routes.remove(handle);
        }
        if let Some(app) = self.app.get() {
            app.end_route(handle.channel, handle.epoch);
        }
    }

    fn request_pools(&self) -> RequestPools {
        RequestPools::new().with_long_held(WATCH_SLOTS, |body| {
            if app_reader::is_watch(body) {
                RequestClass::long_held().with_op("app_watch")
            } else {
                RequestClass::default_pool()
            }
        })
    }

    async fn on_draining(&self, _reason: RouteCloseReason, _deadline: std::time::SystemTime) {
        // Each watch ends with its StreamEnd now, before the daemon's GOODBYE,
        // so an app reconnects to the next instance instead of waiting on a
        // route that will never answer again.
        if let Some(app) = self.app.get() {
            app.shutdown();
        }
    }

    async fn on_hello_ack(&self, ack: &ModuleHelloAckBody) {
        // A reconnect delivers HELLO_ACK again. The store stays open across
        // it: reopening would contend with this process's own lease.
        if self.store.get().is_some() {
            return;
        }
        let opened = match store::open_from_ack(ack) {
            Ok(opened) => Arc::new(opened),
            Err(fatal) => return (self.on_fatal)(fatal),
        };
        if let Err(fatal) = self.start_app_reader(&opened) {
            return (self.on_fatal)(fatal);
        }
        self.open_display(ack, &opened);
        // Without the catalog, Engram skips the store; serving goes on.
        if let Err(error) = crate::backup::write_catalog(opened.data_root(), opened.file_name()) {
            tracing::warn!(%error, "cannot write the Engram backup catalog");
        }
        let _ = self.store.set(opened);
    }

    async fn health(&self) -> HealthReport {
        // Read from memory only: a health check must answer even while a
        // write holds the store. Dispatch saturation is folded in by the SDK.
        match (self.store.get(), self.app.get()) {
            (Some(store), Some(_)) => match self.stopped_background().as_slice() {
                [] => HealthReport {
                    status: HealthStatus::Ok,
                    detail: Some(format!("store open at lease epoch {}", store.epoch())),
                    metrics: None,
                },
                // Reads still work, but held sends are no longer followed or
                // expired receipts no longer pruned, and nothing else says so.
                stopped => HealthReport {
                    status: HealthStatus::Failing,
                    detail: Some(format!(
                        "app reader background work stopped: {}",
                        stopped.join(", ")
                    )),
                    metrics: None,
                },
            },
            _ => HealthReport {
                status: HealthStatus::Degraded,
                detail: Some("waiting for HELLO_ACK to name the store".into()),
                metrics: None,
            },
        }
    }
}

impl OccipitalHandler {
    /// Open the display file beside the store and start its pruning. A file
    /// that can't be opened is logged, and writes are refused from then on:
    /// chats are served without display data.
    fn open_display(&self, ack: &ModuleHelloAckBody, store: &Store) {
        use crate::display::{store::DisplayStore, State};
        if self.display.get().is_some() {
            return;
        }
        let opened = store::descriptor_from_ack(ack)
            .and_then(|descriptor| DisplayStore::open(&descriptor, store.data_root()));
        let state = match opened {
            Ok(display) => {
                let display = Arc::new(display);
                let hints = tokio::sync::broadcast::channel(crate::display::app::HINTS).0;
                let mut started = vec![(
                    "display pruning",
                    crate::display::store::start_pruning(display.clone()),
                )];
                if let Some(calls) = self.daemon_calls() {
                    let providers = crate::display::views::DaemonProviders::new(calls);
                    let (fetcher, task) = crate::display::views::start(
                        display.clone(),
                        Arc::new(providers),
                        Some(hints.clone()),
                    );
                    let _ = self.display_views.set(fetcher);
                    started.push(("display view fetching", task));
                }
                let (call_pins, task) = crate::display::call_pins::start(
                    display.clone(),
                    self.display_views.get().cloned(),
                );
                let _ = self.call_pins.set(call_pins);
                started.push(("call pin keeping", task));
                let tool = Arc::new(crate::display::app::ToolDisplay::new(
                    display.clone(),
                    self.display_views.get().cloned(),
                    hints,
                ));
                if let Some(app) = self.app.get() {
                    app.set_tool_display(tool.clone());
                }
                let _ = self.tool_display.set(tool);
                if let Ok(mut tasks) = self.background.lock() {
                    tasks.extend(started);
                }
                State::Open(display)
            }
            Err(error) => {
                let reason = error.fatal().line;
                tracing::error!(%reason, "cannot open the display store; display writes are refused");
                State::Failed(reason)
            }
        };
        let _ = self.display.set(state);
    }

    /// The one connection this module calls other modules on, for views and
    /// model names; `None` when the daemon gave no connection file.
    fn daemon_calls(&self) -> Option<Arc<crate::display::daemon::DaemonCalls>> {
        let consumer = self.consumer.as_ref()?;
        Some(
            self.daemon_calls
                .get_or_init(|| {
                    Arc::new(crate::display::daemon::DaemonCalls::new(
                        consumer.connection_file.clone(),
                        consumer.identity.clone(),
                    ))
                })
                .clone(),
        )
    }

    /// The names of background tasks that have finished. The list is written
    /// once at startup, so `try_lock` only fails then; a health check never
    /// waits for it.
    fn stopped_background(&self) -> Vec<&'static str> {
        match self.background.try_lock() {
            Ok(tasks) => tasks
                .iter()
                .filter(|(_, task)| task.is_finished())
                .map(|(name, _)| *name)
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

impl Drop for OccipitalHandler {
    fn drop(&mut self) {
        if let Ok(tasks) = self.background.get_mut() {
            tasks.iter().for_each(|(_, task)| task.abort());
        }
    }
}

/// Connect to the daemon and serve until it ends the connection.
pub fn run(consumer: Consumer) -> Result<(), Fatal> {
    let manifest = crate::manifest::manifest()
        .map_err(|error| Fatal::new(EXIT_SERVE_FAILURE, "ck-occipital:", error))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            Fatal::new(
                EXIT_SERVE_FAILURE,
                "ck-occipital: cannot start the runtime:",
                error,
            )
        })?;
    runtime
        .block_on(subc_client_rs::serve(
            manifest,
            OccipitalHandler::new(Some(consumer), Box::new(exit_with)),
        ))
        .map_err(|error| Fatal::new(EXIT_SERVE_FAILURE, "ck-occipital: serving failed:", error))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "show-ui")]
    use crate::show_ui::tests::{arguments, descriptor};
    #[cfg(feature = "show-ui")]
    use cortexkit_role_tool_provider::result::ToolResult;
    use serde_json::json;
    use subc_client_rs::test_support::ModuleHarness;
    use subc_protocol::{FrameType, Principal};

    /// A route to the management surface from a phone, as Callosum binds it.
    fn app_route(channel: u16) -> RouteBindRequest {
        let mut request = RouteBindRequest::new(
            RouteHandle::detached(channel, 1),
            RouteTarget::ManagementSurface {
                module_id: crate::MODULE_ID.into(),
            },
            subc_protocol::BindIdentity::new("fed-phone:0123456789abcdef0123456789abcdef", "app"),
        );
        request.principal = Some(Principal::Reserved {
            module_id: "callosum".into(),
        });
        request
    }

    fn app_request(method: &str, params: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({ "method": method, "params": params })).unwrap()
    }

    fn reply_body(frame: &subc_protocol::Frame) -> Value {
        serde_json::from_slice(&frame.body).unwrap()
    }

    /// Through the SDK, a management-surface route reaches the app reader: a read
    /// answers `{"result": …}`, a watch streams until the app cancels it, and a
    /// refusal keeps its code. A tool-provider route on the same connection still
    /// reaches the tools.
    #[tokio::test]
    async fn app_routes_reach_the_reader_and_tool_routes_the_tools() {
        let app = Arc::new(AppReader::default());
        let serving = app.clone();
        let mut harness = ModuleHarness::start(
            |_| OccipitalHandler::new(None, Box::new(|_| {})).with_app_reader(serving),
            [],
        )
        .await;
        let route = harness.bind_route(app_route(7)).await;

        let corr = harness
            .send_request(
                route,
                app_request("session.display_read", json!({"session": "s"})),
            )
            .await;
        let reply = harness.read_reply(route, corr).await;
        assert_eq!(reply.header.ty, FrameType::Response);
        assert_eq!(
            reply_body(&reply),
            json!({"result": {"absent": "no_ring_attached"}})
        );

        let corr = harness
            .send_request(
                route,
                app_request("session.display_watch", json!({"session": "s"})),
            )
            .await;
        for _ in 0..100 {
            if app.watches.open_count() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(app.watches.open_count(), 1);
        harness.cancel_request(route, corr).await;
        let ended = harness.read_reply(route, corr).await;
        assert_eq!(ended.header.ty, FrameType::StreamEnd, "{ended:?}");

        let corr = harness
            .send_request(route, app_request("session.unknown", json!({})))
            .await;
        let refused = harness.read_reply(route, corr).await;
        assert_eq!(refused.header.ty, FrameType::Error);
        assert_eq!(reply_body(&refused)["code"], "invalid_request");

        let mut tools = RouteBindRequest::new(
            RouteHandle::detached(8, 1),
            RouteTarget::ToolProvider {
                module_id: crate::MODULE_ID.into(),
            },
            subc_protocol::BindIdentity::new("broca", "s"),
        );
        tools.principal = Some(Principal::Reserved {
            module_id: "broca".into(),
        });
        let tools = harness.bind_route(tools).await;
        let corr = harness
            .send_request(
                tools,
                serde_json::to_vec(&json!({"name": "role.describe", "arguments": {}})).unwrap(),
            )
            .await;
        let reply = harness.read_reply(tools, corr).await;
        assert_eq!(reply.header.ty, FrameType::Response);
        describe::check_describe(&reply_body(&reply)).unwrap();
        // The display server's served limits (tool-display §5).
        assert_eq!(
            reply_body(&reply)["tool_display"],
            json!({"max_record_bytes": 16384, "max_detail_bytes": 3145728,
                   "max_completion_bytes": 16384, "loading_window_ms": 5000})
        );
        harness.shutdown().await;
    }

    /// A route to Occipital's tool provider, opened by `principal`.
    fn provider_route(channel: u16, principal: Option<Principal>) -> RouteBindRequest {
        let mut request = RouteBindRequest::new(
            RouteHandle::detached(channel, 1),
            RouteTarget::ToolProvider {
                module_id: crate::MODULE_ID.into(),
            },
            subc_protocol::BindIdentity::new("aft", "s"),
        );
        request.principal = principal;
        request
    }

    fn reserved(module: &str) -> Principal {
        Principal::Reserved {
            module_id: module.into(),
        }
    }

    fn display_write(arguments: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"name": "display.write", "arguments": arguments})).unwrap()
    }

    /// A record for call `call-1` of a Broca session, claiming to come from
    /// `claimed`; the store must ignore that and keep the route's opener.
    fn record_write(claimed: &str) -> Value {
        json!({
            "runner": serde_json::to_value(reserved("broca")).unwrap(),
            "call_key": "call-1",
            "kind": "record",
            "schema_pin": "tp1:bash:0123:1",
            "writer": claimed,
            "body": {"durationMs": 5, "outcome": "success"},
        })
    }

    /// Through the SDK, `display.write` on a tool-provider route is stored once
    /// under the route's attested opener, never a writer named in the params, and
    /// a route the daemon couldn't verify is refused before anything is stored.
    #[tokio::test]
    async fn display_writes_are_stored_under_the_route_s_attested_opener() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-serve-display");
        let descriptor = crate::store::tests::descriptor(&dir);
        let display =
            Arc::new(crate::display::store::DisplayStore::open(&descriptor, dir.path()).unwrap());
        let serving = crate::display::State::Open(display.clone());
        let mut harness = ModuleHarness::start(
            |_| OccipitalHandler::new(None, Box::new(|_| {})).with_display(serving),
            [],
        )
        .await;
        let aft = harness
            .bind_route(provider_route(9, Some(reserved("aft"))))
            .await;
        for expected in ["stored", "already_stored"] {
            let corr = harness
                .send_request(aft, display_write(record_write("reserved:intruder")))
                .await;
            let reply = harness.read_reply(aft, corr).await;
            assert_eq!(reply.header.ty, FrameType::Response, "{reply:?}");
            assert_eq!(reply_body(&reply), json!({"result": expected}));
        }
        let mut missing = record_write("");
        missing.as_object_mut().unwrap().remove("call_key");
        let corr = harness.send_request(aft, display_write(missing)).await;
        let refused = harness.read_reply(aft, corr).await;
        assert_eq!(refused.header.ty, FrameType::Error);
        assert_eq!(reply_body(&refused)["code"], "invalid_params");
        assert_eq!(reply_body(&refused)["detail"], json!({"field": "call_key"}));

        for (channel, opener) in [(10, Some(Principal::Direct)), (11, None)] {
            let route = harness.bind_route(provider_route(channel, opener)).await;
            let corr = harness
                .send_request(route, display_write(record_write("")))
                .await;
            let refused = harness.read_reply(route, corr).await;
            assert_eq!(refused.header.ty, FrameType::Error);
            assert_eq!(reply_body(&refused)["code"], "display_writer_unverified");
        }
        harness.shutdown().await;
        let stored = display.writes(&reserved("broca"), "call-1").unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].writer, reserved("aft"));
    }

    /// Before HELLO_ACK names the data folder there is no display store, and a
    /// write gets the contract's retryable not-ready refusal.
    #[tokio::test]
    async fn a_display_write_before_the_store_is_named_is_refused_as_warming() {
        let mut harness =
            ModuleHarness::start(|_| OccipitalHandler::new(None, Box::new(|_| {})), []).await;
        let route = harness
            .bind_route(provider_route(9, Some(reserved("aft"))))
            .await;
        let corr = harness
            .send_request(route, display_write(record_write("")))
            .await;
        let refused = harness.read_reply(route, corr).await;
        assert_eq!(refused.header.ty, FrameType::Error);
        assert_eq!(reply_body(&refused)["code"], "module_warming");
        harness.shutdown().await;
    }

    /// The daemon's operator-app stamp reaches admission through the SDK: a route
    /// whose opener the daemon could not verify reads when the stamp names the
    /// desktop app, and is refused without it.
    #[tokio::test]
    async fn the_operator_app_stamp_on_a_bind_reaches_admission() {
        let app = Arc::new(AppReader::default());
        let serving = app.clone();
        let mut harness = ModuleHarness::start(
            |_| OccipitalHandler::new(None, Box::new(|_| {})).with_app_reader(serving),
            [],
        )
        .await;
        for (channel, stamp, admitted) in [
            (
                7,
                Some(subc_protocol::peer::OperatorApp::new(
                    "alfonso-desktop",
                    4242,
                )),
                true,
            ),
            (8, None, false),
        ] {
            let mut request = app_route(channel);
            request.principal = Some(Principal::Unverified);
            request.peer.operator_app = stamp;
            let route = harness.bind_route(request).await;
            let corr = harness
                .send_request(
                    route,
                    app_request("session.display_read", json!({"session": "s"})),
                )
                .await;
            let reply = harness.read_reply(route, corr).await;
            if admitted {
                assert_eq!(reply.header.ty, FrameType::Response, "{reply:?}");
            } else {
                assert_eq!(reply.header.ty, FrameType::Error);
                assert_eq!(reply_body(&reply)["code"], "carrier_op_not_permitted");
            }
        }
        harness.shutdown().await;
    }

    /// Before HELLO_ACK names the store there is no app reader, and an app
    /// request is refused with a code the apps retry.
    #[tokio::test]
    async fn an_app_request_before_the_store_is_named_is_refused_as_retryable() {
        let mut harness =
            ModuleHarness::start(|_| OccipitalHandler::new(None, Box::new(|_| {})), []).await;
        let route = harness.bind_route(app_route(7)).await;
        let corr = harness
            .send_request(
                route,
                app_request("session.display_read", json!({"session": "s"})),
            )
            .await;
        let refused = harness.read_reply(route, corr).await;
        assert_eq!(refused.header.ty, FrameType::Error);
        assert_eq!(reply_body(&refused)["code"], "provider_unavailable");
        harness.shutdown().await;
    }

    /// A route that goes ends the watches open on it, and only those; a drain
    /// ends every watch and refuses new ones.
    #[tokio::test]
    async fn route_loss_and_drains_end_the_watches_they_cover() {
        use occipital_app_reader::watches::{OpenRefusal, WatchRoute};
        let app = Arc::new(AppReader::default());
        let handler = OccipitalHandler::new(None, Box::new(|_| {})).with_app_reader(app.clone());
        let (gone, kept) = (app_route(7), app_route(8));
        handler.on_bind(&gone).await;
        handler.on_bind(&kept).await;
        let watch = |channel: u16| {
            app.watches
                .open(WatchRoute {
                    channel: channel.into(),
                    epoch: 1,
                })
                .unwrap()
        };
        let (on_gone, on_kept) = (watch(7), watch(8));
        handler.on_route_gone(&gone.handle).await;
        assert!(*on_gone.ended().borrow());
        assert!(!*on_kept.ended().borrow());
        assert!(handler.binding(gone.handle).is_none());
        assert!(handler.binding(kept.handle).is_some());
        handler
            .on_draining(
                RouteCloseReason::Restart,
                std::time::SystemTime::now() + std::time::Duration::from_secs(30),
            )
            .await;
        assert!(*on_kept.ended().borrow());
        assert!(matches!(
            app.watches.open(WatchRoute {
                channel: 9,
                epoch: 1
            }),
            Err(OpenRefusal::Stopping)
        ));
    }

    #[test]
    fn only_the_two_watches_wait_in_the_long_held_pool() {
        for (method, watch) in [
            ("session.transcript_watch", true),
            ("session.display_watch", true),
            ("session.transcript_page", false),
            ("session.enqueue_user_message", false),
            ("upload.chunk", false),
        ] {
            assert_eq!(
                app_reader::is_watch(&app_request(method, json!({"session": "s"}))),
                watch,
                "{method}"
            );
        }
        assert!(!app_reader::is_watch(b"\0\0\0\x02{}bytes"));
        let pools = OccipitalHandler::new(None, Box::new(|_| {})).request_pools();
        assert_eq!(pools.long_held_limit(), Some(WATCH_SLOTS));
    }

    #[cfg(feature = "show-ui")]
    use std::sync::Mutex;

    #[cfg(feature = "show-ui")]
    struct Recorder(Arc<Mutex<Vec<PreparedCall>>>);

    #[cfg(feature = "show-ui")]
    impl NativeExecutor for Recorder {
        fn execute(&self, call: PreparedCall, _store: Option<&Store>) -> HandlerOutcome {
            self.0.lock().unwrap().push(call);
            internal("recorded at native execution boundary")
        }
    }

    #[cfg(feature = "show-ui")]
    fn bind_request(scoped: bool) -> RouteBindRequest {
        let sample: Value = serde_json::from_str(include_str!("../tests/fixtures/pinned/subc-protocol-0.33.1/module_control_request_route_bind_with_scope.json")).unwrap();
        let mut request = RouteBindRequest::new(
            RouteHandle::detached(42, 7),
            serde_json::from_value(sample["target"].clone()).unwrap(),
            serde_json::from_value(sample["identity"].clone()).unwrap(),
        );
        request.principal = Some(serde_json::from_value(sample["principal"].clone()).unwrap());
        if scoped {
            request.scope = Some(serde_json::from_value(sample["scope"].clone()).unwrap());
        }
        request
    }

    fn decode(outcome: HandlerOutcome) -> Value {
        match outcome {
            HandlerOutcome::Response(bytes) => serde_json::from_slice(&bytes).unwrap(),
            other => panic!("expected response: {other:?}"),
        }
    }

    #[cfg(feature = "show-ui")]
    fn body(args: Value, refs: Option<Value>) -> Vec<u8> {
        let mut request = json!({"name":"show_ui", "arguments":args});
        if let Some(refs) = refs {
            request["output_refs"] = refs;
        }
        serde_json::to_vec(&request).unwrap()
    }

    #[cfg(feature = "show-ui")]
    fn rows(dir: &cortexkit_test_support::ScratchDir) -> i64 {
        let descriptor = crate::store::tests::descriptor(dir);
        let cortexkit_store::StorageBackend::Sqlite { path } = descriptor.backend else {
            panic!("sqlite descriptor")
        };
        rusqlite::Connection::open(path)
            .unwrap()
            .query_row("SELECT count(*) FROM native_records", [], |row| row.get(0))
            .unwrap()
    }

    #[cfg(feature = "show-ui")]
    async fn with_recorder(
        dir: &cortexkit_test_support::ScratchDir,
    ) -> (OccipitalHandler, Arc<Mutex<Vec<PreparedCall>>>) {
        let calls = Arc::new(Mutex::new(vec![]));
        let mut handler = OccipitalHandler::new(
            None,
            Box::new(|fatal| panic!("unexpected fatal: {}", fatal.line)),
        );
        handler.executor = Box::new(Recorder(calls.clone()));
        let storage = serde_json::to_value(crate::store::tests::descriptor(dir)).unwrap();
        handler.on_hello_ack(&ack(Some(storage))).await;
        (handler, calls)
    }

    #[tokio::test]
    #[cfg(feature = "show-ui")]
    async fn refusal_samples_have_literal_codes_exact_members_and_zero_rows() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-refusals");
        let (handler, calls) = with_recorder(&dir).await;
        let cases: Value =
            serde_json::from_str(include_str!("../test-vectors/show-ui-v1/refusals.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let request = bind_request(case["scoped"].as_bool().unwrap());
            handler.on_bind(&request).await;
            let output = decode(handler.dispatch(
                request.handle,
                &body(case["arguments"].clone(), case.get("output_refs").cloned()),
            ));
            assert_eq!(output["isError"], true, "{}", case["name"]);
            assert_eq!(output["errorCode"], case["code"]);
            let content = &output["structuredContent"];
            assert_eq!(content["schema"], "occipital.show_ui/1");
            assert_eq!(content["outcome"], "refused");
            assert_eq!(content["code"], case["code"]);
            let mut keys: Vec<_> = content
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort();
            assert_eq!(keys, ["code", "message", "outcome", "schema"]);
            let mirrored: Value =
                serde_json::from_str(output["content"][0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(&mirrored, content);
            let wire: ToolResult = serde_json::from_value(output).unwrap();
            cortexkit_role_tool_provider::result::validate(&wire).unwrap();
            assert!(calls.lock().unwrap().is_empty());
            assert_eq!(rows(&dir), 0);
        }
        let mut args = arguments();
        args["data_columns"] = Value::Object(serde_json::Map::from_iter([(
            "x".repeat(1_048_576),
            "text".into(),
        )]));
        let request = bind_request(true);
        handler.on_bind(&request).await;
        let output = decode(handler.dispatch(request.handle, &body(args, None)));
        assert_eq!(output["structuredContent"]["code"], "argument_invalid");
        assert_eq!(rows(&dir), 0);
    }

    #[tokio::test]
    #[cfg(feature = "show-ui")]
    async fn provenance_comes_only_from_route_stamp_not_bind_identity() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-provenance");
        let (handler, calls) = with_recorder(&dir).await;
        let mut request = bind_request(true);
        request.identity = serde_json::from_value(json!({"project_root":"unattested/root", "harness":"unattested-harness", "session":"unattested-session", "project_id":"unattested-project"})).unwrap();
        handler.on_bind(&request).await;
        handler.dispatch(request.handle, &body(arguments(), Some(descriptor(0))));
        // Changing every identity field must leave the entire prepared provenance
        // unchanged: the caller's configuration cannot rename the scope owner.
        request.identity = subc_protocol::BindIdentity::new("", "");
        handler.on_bind(&request).await;
        handler.dispatch(request.handle, &body(arguments(), None));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        for call in calls.iter() {
            assert_eq!(
                call.provenance.owner,
                subc_protocol::Principal::Reserved {
                    module_id: "prefrontal-core".into()
                }
            );
            assert_eq!(call.provenance.scope_ref, "head-1");
            assert_eq!(call.provenance.scope_epoch, 3);
            assert_eq!(
                call.provenance.principal,
                subc_protocol::Principal::Reserved {
                    module_id: "broca".into()
                }
            );
            assert_eq!(call.source.as_bytes(), "source\n雪".as_bytes());
            assert_eq!(call.context.show_ui.title, "Title 🦉");
            assert!(call.context.references.is_empty());
            assert!(call.host.data.is_none());
            assert!(call.host.snapshots.is_empty());
            assert_eq!(
                call.host.now,
                occipital_lang::Decimal::from_integer(i128::from(call.now_ms)).unwrap()
            );
        }
        assert_eq!(rows(&dir), 0);
    }

    #[tokio::test]
    #[cfg(feature = "show-ui")]
    async fn empty_descriptors_are_admitted_but_nonempty_sets_are_not() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-descriptor");
        let (handler, calls) = with_recorder(&dir).await;
        let request = bind_request(true);
        handler.on_bind(&request).await;
        let raw = format!("{{ \"set_id\" : \"s\\u0065t\", \"mid\":\"mid\", \"dialect\":\"rich-ui/1\", \"count\":0, \"digest\":\"{}\", \"future\":true }}", "a".repeat(64));
        let input = format!(
            "{{\"name\":\"show_ui\",\"arguments\":{},\"output_refs\":{raw}}}",
            arguments()
        );
        handler.dispatch(request.handle, input.as_bytes());
        let decoded: Request = serde_json::from_slice(input.as_bytes()).unwrap();
        assert_eq!(
            decoded.output_refs.unwrap().get().as_bytes(),
            raw.as_bytes()
        );
        assert!(calls.lock().unwrap()[0].host.data.is_none());
        let sample: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/pinned/commons-d75cf9966453/output-refs.json"
        ))
        .unwrap();
        assert_eq!(sample["records"].as_array().unwrap().len(), 4);
        assert!(sample["descriptor"].get("records").is_none());
        let output = decode(handler.dispatch(
            request.handle,
            &body(arguments(), Some(sample["descriptor"].clone())),
        ));
        assert_eq!(
            output["structuredContent"]["code"],
            "references_unavailable"
        );
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(rows(&dir), 0);
    }

    #[test]
    fn the_system_text_item_gets_the_stamped_reply_teaching() {
        let full = decode(discovery("tool.catalog", &json!({"system_text": {}})));
        let catalog: catalog::CatalogAnswer = serde_json::from_value(full.clone()).unwrap();
        let text = catalog.system_text.clone().unwrap();
        assert_eq!(
            text.text.as_deref(),
            Some(occipital_lang::reply_teaching().text.as_str())
        );
        assert_eq!(text.rich_ui, Some(crate::system_text::stamp().unwrap()));
        // The text is composed for exactly the tools served beside it.
        let names: Vec<String> = catalog.tools.iter().map(|t| t.name.clone()).collect();
        assert_eq!(text.tool_names, Some(names));
        // A digest-only fetch with the same item carries the same digest,
        // and the text is part of what that digest covers.
        let cheap = decode(discovery(
            "tool.catalog",
            &json!({"system_text": {}, "digest_only": true}),
        ));
        assert_eq!(cheap["catalog_digest"], full["catalog_digest"]);
        assert!(cheap.get("system_text").is_none());
        let plain = decode(discovery("tool.catalog", &json!({})));
        assert_ne!(plain["catalog_digest"], full["catalog_digest"]);
    }

    #[test]
    fn a_sent_composition_is_named_on_the_catalog_and_the_system_text() {
        let composition = json!({"persona": "x", "n": 1});
        let expected = catalog::composition_digest(&composition).unwrap();
        let answer: catalog::CatalogAnswer = serde_json::from_value(decode(discovery(
            "tool.catalog",
            &json!({"composition": composition, "system_text": {}}),
        )))
        .unwrap();
        assert_eq!(
            answer.composition_digest.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            answer.system_text.unwrap().composition_digest.as_deref(),
            Some(expected.as_str())
        );
        let preflight = decode(discovery("tool.catalog", &json!({})));
        assert!(preflight.get("composition_digest").is_none());
    }

    #[test]
    fn an_answer_without_the_teaching_text_carries_no_stamp() {
        for request in [json!({}), json!({"digest_only": true})] {
            let answer = decode(discovery("tool.catalog", &request));
            assert!(answer.get("system_text").is_none(), "{answer}");
            assert!(!answer.to_string().contains("rich_ui"), "{answer}");
        }
    }

    #[test]
    fn a_system_text_preset_or_param_is_refused_naming_the_member() {
        for (item, field) in [
            (json!({"preset": "default"}), "system_text.preset"),
            (json!({"params": {"tone": "x"}}), "system_text.params"),
        ] {
            match discovery("tool.catalog", &json!({ "system_text": item })) {
                HandlerOutcome::ErrorWithDetail { code, detail, .. } => {
                    assert_eq!(code, "invalid_request");
                    assert_eq!(detail, json!({ "field": field }));
                }
                other => panic!("expected a refusal: {other:?}"),
            }
        }
    }

    #[tokio::test]
    #[cfg(feature = "show-ui")]
    async fn unscoped_discovery_and_show_ui_run_through_the_real_handler() {
        use subc_client_rs::test_support::ModuleHarness;
        let mut harness =
            ModuleHarness::start(|_| OccipitalHandler::new(None, Box::new(|_| {})), []).await;
        let route = harness.bind_route(bind_request(false)).await;
        for name in ["role.describe", "tool.catalog"] {
            let corr = harness
                .send_request(
                    route,
                    serde_json::to_vec(&json!({"name":name,"arguments":{}})).unwrap(),
                )
                .await;
            let reply = harness.read_reply(route, corr).await;
            assert_eq!(reply.header.ty, subc_protocol::FrameType::Response);
            let value: Value = serde_json::from_slice(&reply.body).unwrap();
            if name == "role.describe" {
                describe::check_describe(&value).unwrap();
            } else {
                let catalog: catalog::CatalogAnswer = serde_json::from_value(value).unwrap();
                assert_eq!(catalog.tools.len(), 1);
                assert_eq!(catalog.tools[0].name, "show_ui");
                assert_eq!(
                    catalog.tools[0].input_schema,
                    crate::manifest::show_ui_schema()
                );
            }
        }
        let corr = harness
            .send_request(
                route,
                serde_json::to_vec(&json!({"name":"tool.catalog","arguments":{"system_text":{}}}))
                    .unwrap(),
            )
            .await;
        let reply = harness.read_reply(route, corr).await;
        let catalog: catalog::CatalogAnswer = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(
            catalog.system_text.unwrap().rich_ui,
            Some(crate::system_text::stamp().unwrap())
        );
        let corr = harness.send_request(route, body(arguments(), None)).await;
        let reply = harness.read_reply(route, corr).await;
        let value: Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(value["structuredContent"]["code"], "route_unscoped");
        harness.shutdown().await;
    }

    #[tokio::test]
    #[cfg(feature = "show-ui")]
    async fn route_teardown_drops_authority_and_new_epochs_do_not_inherit_it() {
        let (handler, _) = recording();
        let request = bind_request(true);
        handler.on_bind(&request).await;
        let other = RouteHandle::detached(request.handle.channel, request.handle.epoch + 1);
        assert_eq!(
            decode(handler.dispatch(other, &body(arguments(), None)))["structuredContent"]["code"],
            "route_unscoped"
        );
        handler.on_route_gone(&request.handle).await;
        assert_eq!(
            decode(handler.dispatch(request.handle, &body(arguments(), None)))["structuredContent"]
                ["code"],
            "route_unscoped"
        );
    }

    #[tokio::test]
    #[cfg(feature = "show-ui")]
    async fn production_handler_checks_and_stores_with_route_not_bind_provenance() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-native-handler");
        let (handler, _) = recording();
        let descriptor = crate::store::tests::descriptor(&dir);
        handler
            .on_hello_ack(&ack(Some(serde_json::to_value(&descriptor).unwrap())))
            .await;
        let mut request = bind_request(true);
        request.identity =
            subc_protocol::BindIdentity::new("unattested-harness", "unattested-session");
        handler.on_bind(&request).await;
        let mut args = arguments();
        args["program"] = "Text(\"Hello 雪\")".into();
        let output = decode(handler.dispatch(request.handle, &body(args, None)));
        assert_eq!(output["structuredContent"]["outcome"], "stored");
        assert_eq!(rows(&dir), 1);
        let scope = request.scope.unwrap();
        let expected = (
            crate::store::principal_text(&scope.owner),
            scope.scope_ref,
            scope.scope_epoch,
            crate::store::principal_text(&request.principal.unwrap()),
        );
        drop(handler);
        // The app reader's background tasks hold the store too; dropping the
        // handler aborts them, and the runtime drops them on its next turn.
        let mut reopened = crate::store::open(&descriptor);
        for _ in 0..10 {
            if reopened.is_ok() {
                break;
            }
            tokio::task::yield_now().await;
            reopened = crate::store::open(&descriptor);
        }
        let reopened = reopened.unwrap();
        let cortexkit_store::StorageBackend::Sqlite { path } = descriptor.backend else {
            panic!("sqlite");
        };
        let conn = rusqlite::Connection::open(path).unwrap();
        let actual: (String, String, u64, String) = conn
            .query_row(
                "SELECT owner, ref, scope_epoch, principal FROM native_records",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(actual, expected);
        assert!(
            !format!("{actual:?}").contains("unattested"),
            "client-supplied bind fields must not be stored"
        );
        drop(reopened);
    }

    #[test]
    #[cfg(feature = "show-ui")]
    fn pinned_wire_samples_decode_with_the_same_request_and_result_types() {
        let raw = include_bytes!(
            "../tests/fixtures/pinned/subc-protocol-0.33.1/tool_call_request_with_output_refs.json"
        );
        let request: Request = serde_json::from_slice(raw).unwrap();
        let sdk: subc_protocol::tool_call::ToolCallRequest = serde_json::from_slice(raw).unwrap();
        assert_eq!(request.name, sdk.name);
        assert_eq!(request.arguments, sdk.arguments);
        let refs: Value = serde_json::from_str(request.output_refs.unwrap().get()).unwrap();
        assert_eq!(refs, serde_json::to_value(sdk.output_refs).unwrap());
        assert_eq!(refs["count"], 2);
        let results: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/pinned/cortexkit-role-tool-provider-0.13.0/tool-result-v1.json"
        ))
        .unwrap();
        for vector in results["valid"].as_array().unwrap() {
            let result: ToolResult = serde_json::from_value(vector["body"].clone()).unwrap();
            cortexkit_role_tool_provider::result::validate(&result).unwrap();
        }
        let describes: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/pinned/cortexkit-role-tool-provider-0.13.0/role-describe.json"
        ))
        .unwrap();
        for vector in describes["accepted"].as_array().unwrap() {
            let describe = describe::check_describe(&vector["answer"]).unwrap();
            assert!(describe
                .major(cortexkit_role_tool_provider::PROVIDES)
                .is_some());
        }
        let catalogs: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/pinned/cortexkit-role-tool-provider-0.13.0/catalog-answers.json"
        ))
        .unwrap();
        let full: catalog::CatalogAnswer =
            serde_json::from_value(catalogs["full"].clone()).unwrap();
        assert_eq!(full.tools[0].name, "read");
        assert_eq!(full.tools[0].semantics, 1);
    }

    fn recording() -> (OccipitalHandler, Arc<Mutex<Vec<Fatal>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let handler = OccipitalHandler::new(
            None,
            Box::new(move |fatal| {
                sink.lock().unwrap().push(fatal);
            }),
        );
        (handler, seen)
    }

    fn ack(storage: Option<serde_json::Value>) -> ModuleHelloAckBody {
        ModuleHelloAckBody {
            negotiated_ver: subc_protocol::PROTOCOL_VERSION,
            subc_ops: vec![],
            subc_capabilities: vec![],
            storage,
            machine_id: None,
        }
    }

    #[tokio::test]
    async fn hello_ack_opens_the_named_store_once() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-serve-open");
        let descriptor = crate::store::tests::descriptor(&dir);
        let (handler, seen) = recording();
        assert_eq!(handler.health().await.status, HealthStatus::Degraded);
        let ack = ack(Some(serde_json::to_value(&descriptor).unwrap()));
        handler.on_hello_ack(&ack).await;
        // A second HELLO_ACK after a reconnect must not try to take the
        // leases this process already holds, the store's or the display file's.
        handler.on_hello_ack(&ack).await;
        assert!(
            seen.lock().unwrap().is_empty(),
            "{:?}",
            seen.lock().unwrap()
        );
        assert_eq!(handler.health().await.status, HealthStatus::Ok);
        assert!(matches!(
            handler.display.get(),
            Some(crate::display::State::Open(_))
        ));
        let data_root = dir.path().join("cortexkit").join("occipital");
        assert!(data_root.join(crate::display::store::FILE).exists());
        let pruning = handler
            .background
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| *name == "display pruning")
            .count();
        assert_eq!(pruning, 1);
    }

    /// The background tasks run for the module's whole life, so one that has
    /// finished (a panic, say) leaves held sends unfollowed with no other sign.
    #[tokio::test]
    async fn a_stopped_background_task_fails_the_health_check_by_name() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-serve-background");
        let descriptor = crate::store::tests::descriptor(&dir);
        let (handler, _) = recording();
        handler
            .on_hello_ack(&ack(Some(serde_json::to_value(&descriptor).unwrap())))
            .await;
        assert_eq!(handler.health().await.status, HealthStatus::Ok);
        let (name, task) = {
            let tasks = handler.background.lock().unwrap();
            let (name, task) = tasks
                .iter()
                .find(|(name, _)| *name == "held send follow-up")
                .unwrap();
            task.abort();
            (*name, task.abort_handle())
        };
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
        let report = handler.health().await;
        assert_eq!(report.status, HealthStatus::Failing);
        assert_eq!(
            report.detail.as_deref(),
            Some(format!("app reader background work stopped: {name}").as_str())
        );
    }

    #[tokio::test]
    async fn hello_ack_with_a_newer_store_reports_exit_1() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-serve-newer");
        let descriptor = crate::store::tests::descriptor(&dir);
        drop(
            crate::store::open(&descriptor)
                .map_err(|e| e.fatal())
                .unwrap(),
        );
        let cortexkit_store::StorageBackend::Sqlite { path } = &descriptor.backend else {
            panic!("the test descriptor is sqlite");
        };
        rusqlite::Connection::open(path)
            .unwrap()
            .pragma_update(None, "user_version", 2)
            .unwrap();
        let (handler, seen) = recording();
        handler
            .on_hello_ack(&ack(Some(serde_json::to_value(&descriptor).unwrap())))
            .await;
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].code, 1);
        assert!(
            seen[0]
                .line
                .starts_with(crate::store::VERSION_UNSUPPORTED_PREFIX),
            "{}",
            seen[0].line
        );
    }
}
