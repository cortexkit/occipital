//! App-facing reads share the page logic, but authorize from daemon route stamps.

pub mod broca;
pub mod display_hub;
mod observe;
pub mod resolve;
mod route;
pub mod send;
pub mod send_queue;
pub mod send_store;
pub mod uploads;

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use occipital_app_reader::{
    display::DisplayReadRequest,
    page::PageRequest,
    source::{ReadError, TranscriptReader, TranscriptSource},
    watches::{OpenRefusal, WatchRegistry, WatchRoute},
};
use serde_json::Value;
use subc_client_rs::{HandlerOutcome, RequestCtx};
use subc_protocol::{manifest::*, Principal, RouteTarget};

/// Taking a lock a panicked holder left behind. These mutexes guard caches and
/// bookkeeping that later requests read again, so one request that panicked
/// must not turn every later request on the same session into a panic too.
pub(crate) trait Guard<T> {
    fn guard(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> Guard<T> for std::sync::Mutex<T> {
    fn guard(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub const TRANSCRIPT_PAGE: &str = "session.transcript_page";
pub const TRANSCRIPT_WATCH: &str = "session.transcript_watch";
pub const DISPLAY_READ: &str = "session.display_read";
pub const DISPLAY_WATCH: &str = "session.display_watch";
pub const DETAILS: &str = occipital_app_reader::details::OPERATION;
/// An image in the chat: its thumbnail or its original bytes (`crate::media`).
pub const MEDIA: &str = crate::media::OPERATION;
/// The system text and tools the session's model is given (`crate::request_context`).
pub const REQUEST_CONTEXT: &str = crate::request_context::OPERATION;
pub const METHODS: [&str; 7] = [
    TRANSCRIPT_PAGE,
    TRANSCRIPT_WATCH,
    DISPLAY_READ,
    DISPLAY_WATCH,
    DETAILS,
    MEDIA,
    REQUEST_CONTEXT,
];
/// The apps' send keeps core's operation name, so the apps change only the module
/// they address.
pub const SEND: &str = occipital_app_reader::send::OPERATION;
/// The apps' Stop for the session's running turn (`send::stop`).
pub const STOP: &str = send::stop::OPERATION;
/// The apps' Stop everything: the turn, and what the session's tools run (`send::stop_all`).
pub const STOP_ALL: &str = send::stop_all::OPERATION;

/// The tool display reads (`crate::display::app`): views by pin, templates by
/// digest, a session's records and a call's detail, and the watch saying when
/// to read records again; and the names models are shown by
/// (`crate::display::model_names`). They are reads like the transcript's,
/// admitted the same way.
pub const DISPLAY_METHODS: [&str; 6] = [
    crate::display::app::VIEWS,
    crate::display::app::TEMPLATE,
    crate::display::app::RECORDS,
    crate::display::app::DETAIL,
    crate::display::app::RECORDS_WATCH,
    crate::display::model_names::OPERATION,
];

/// The notices that rode a session's tool calls (`crate::notices`), the
/// watch saying when one arrives, and the wake records of a session's
/// messages (`crate::notices::wakes`). They are reads, so [`admit`] lets in
/// the same callers as the transcript reads, the desktop app included.
pub const NOTICE_METHODS: [&str; 3] = [
    crate::notices::READ,
    crate::notices::WATCH,
    crate::notices::wakes::READ,
];

pub fn operations() -> Vec<ManagementOperation> {
    METHODS
        .into_iter()
        .chain(DISPLAY_METHODS)
        .chain(NOTICE_METHODS)
        .map(|name| ManagementOperation {
            name: name.into(),
            kind: ManagementOperationKind::Query,
            description: None,
        })
        .chain([SEND, STOP, STOP_ALL].map(|name| ManagementOperation {
            name: name.into(),
            kind: ManagementOperationKind::Mutate,
            description: None,
        }))
        .chain(uploads::operations())
        .collect()
}

pub fn operation_delivery() -> BTreeMap<String, OperationDelivery> {
    [
        TRANSCRIPT_WATCH,
        DISPLAY_WATCH,
        crate::display::app::RECORDS_WATCH,
        crate::notices::WATCH,
    ]
    .into_iter()
    .map(|name| (name.into(), OperationDelivery::WATCH))
    .collect()
}

/// What a session's prompt takes, for a file going into it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptFit {
    /// The runner stores a message of files with no text (the runner contract's
    /// `media_only_send` group).
    pub media_only: bool,
    /// The largest image file the model takes, in bytes as uploaded, and the
    /// longest side of one image in pixels, when the runner reports them.
    pub max_image_bytes: Option<u64>,
    pub max_image_dimension: Option<u64>,
}

pub type RouteBinding = (
    Option<Principal>,
    RouteTarget,
    Option<subc_protocol::scope::ScopeStamp>,
    Option<subc_protocol::BindIdentity>,
    Option<subc_protocol::peer::OperatorApp>,
);

/// Caller identity comes from daemon route metadata, never request parameters.
#[derive(Debug, PartialEq, Eq)]
pub enum Caller {
    /// An admitted local operator, not a paired device with an upload fingerprint.
    Operator,
    Device {
        class: String,
        fingerprint: String,
    },
}

/// Development admission. While the desktop app is being built it connects as a plain
/// local `direct` caller, which the daemon cannot yet tell apart from any other local
/// key-holder; until the daemon's `operator_app` stamp ships, such a caller gets what a
/// paired device gets: the reads, the send and uploads. The release replaces exactly
/// this admission with the stamp check, so it is one named place, never a setting.
const DEVELOPMENT_DIRECT_ADMITTED: bool = true;

/// The device fingerprint a development `direct` caller's uploads are bound to. Every
/// such caller shares it, which is acceptable only while direct callers are admitted
/// for development.
pub(crate) const DEVELOPMENT_DIRECT_FINGERPRINT: &str = "00000000000000000000000000000000";

pub fn admit(binding: Option<&RouteBinding>, method: &str) -> Result<Caller, ReadError> {
    if let Some(caller) = admit_principal(binding) {
        return Ok(caller);
    }
    // The daemon's operator-app stamp adds the five reads for the verified desktop app,
    // and nothing else. It is checked after `admit_principal` so that, while direct callers
    // are admitted for development, the stamp changes nothing for the desktop (its
    // sends, Stop and uploads keep working); once that admission is off, the desktop
    // reads and is refused everything else.
    let desktop_app = binding.is_some_and(|(_, target, _, _, operator_app)| {
        matches!(target, RouteTarget::ManagementSurface { module_id } if module_id == crate::MODULE_ID)
            && operator_app.as_ref().is_some_and(|app| app.app == "alfonso-desktop")
    });
    if desktop_app
        && (METHODS.contains(&method)
            || DISPLAY_METHODS.contains(&method)
            || NOTICE_METHODS.contains(&method))
    {
        return Ok(Caller::Operator);
    }
    Err(admission_refusal(binding, method))
}

/// The admission a route's principal earns on its own, whichever method it calls.
fn admit_principal(binding: Option<&RouteBinding>) -> Option<Caller> {
    if let Some((Some(Principal::Direct), target, None, _, _)) = binding {
        if DEVELOPMENT_DIRECT_ADMITTED
            && matches!(target, RouteTarget::ManagementSurface { module_id } if module_id == crate::MODULE_ID)
        {
            return Some(Caller::Device {
                class: "direct".into(),
                fingerprint: DEVELOPMENT_DIRECT_FINGERPRINT.into(),
            });
        }
    }
    if let Some((Some(Principal::Reserved { module_id }), target, None, identity, _)) = binding {
        if module_id == "callosum"
            && matches!(target, RouteTarget::ManagementSurface { module_id } if module_id == crate::MODULE_ID)
        {
            if let Some((class, fingerprint)) = identity
                .as_ref()
                .and_then(|identity| identity.harness.strip_prefix("fed-"))
                .and_then(|label| label.split_once(':'))
                .filter(|(class, fingerprint)| !class.is_empty() && !fingerprint.is_empty())
            {
                return Some(Caller::Device {
                    class: class.into(),
                    fingerprint: fingerprint.into(),
                });
            }
            return Some(Caller::Operator);
        }
    }
    None
}

fn admission_refusal(binding: Option<&RouteBinding>, method: &str) -> ReadError {
    ReadError::new(
        "carrier_op_not_permitted",
        format!(
            "principal {:?} is not permitted to call {method}",
            binding.and_then(|b| b.0.as_ref())
        ),
    )
}

/// Resolution is deliberately separate from authorization: refused callers never reach a source.
#[async_trait]
pub trait SourceResolver: Send + Sync {
    /// Check the session's placement, found as [`SourceResolver::resolve`] finds it
    /// (`harness` names a session of that harness, whatever runner hosts it; without
    /// it the session is a Broca one), without opening a transcript route: for a
    /// read served from what Occipital already holds, and before replaying a kept
    /// send receipt.
    async fn validate_session(
        &self,
        _session: &str,
        _harness: Option<&str>,
    ) -> Result<(), ReadError> {
        Ok(())
    }

    async fn resolve(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError>;

    /// The session's project root, from its Entorhinal binding and never from the
    /// caller, for placing uploads. `harness` names a session of that harness, whatever
    /// runner hosts it; without it the session is a Broca one. A session held on another
    /// machine is refused `session_on_other_machine`.
    async fn project_root(
        &self,
        _session: &str,
        _harness: Option<&str>,
    ) -> Result<PathBuf, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "no session resolver is configured for uploads",
        ))
    }

    /// The runner's `session.media_read` answer for an image or file `sha256`
    /// that the session's history references, as the runner contract serves it:
    /// `{data?, size, mime}`, with up to `length` bytes from `offset` in base64.
    async fn media_read(
        &self,
        _session: &str,
        _harness: Option<&str>,
        _sha256: &str,
        _offset: u64,
        _length: u64,
    ) -> Result<Value, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "no runner connection is configured for media reads",
        ))
    }

    /// The system text and tools of the plan in force on the session, from its
    /// runner's `session.plan_view`, checked against the contract's closed type.
    async fn plan_view(
        &self,
        _session: &str,
        _harness: Option<&str>,
    ) -> Result<cortexkit_role_llm_runner::plan_view::PlanView, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "no runner connection is configured for plan views",
        ))
    }

    /// Whether a file of type `mime` can go into the session's prompt, and what
    /// the prompt takes: `Some` when its runner takes uploads (the runner
    /// contract's `attachments` group) and its model isn't known to refuse the
    /// type. Without a resolver that can tell, the file goes to the project folder.
    async fn prompt_fit(
        &self,
        _session: &str,
        _harness: Option<&str>,
        _mime: &str,
    ) -> Result<Option<PromptFit>, ReadError> {
        Ok(None)
    }

    async fn watch(
        &self,
        session: &str,
        harness: Option<&str>,
        _: Arc<display_hub::DisplayHub>,
    ) -> Result<SourceWatch, ReadError> {
        Ok(SourceWatch {
            source: self.resolve(session, harness).await?,
            lease: None,
            live: None,
        })
    }

    /// The runner hosting the app's session, and the session as that runner
    /// keys it, resolved as a read resolves it. Tool display reads key calls
    /// this way.
    async fn reference(
        &self,
        _session: &str,
        _harness: Option<&str>,
    ) -> Result<resolve::SessionRef, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "no session resolver is configured for tool display reads",
        ))
    }
}

/// The lease keeps the source subscription active until the device watch ends.
#[derive(Default)]
pub struct SourceWatch {
    pub source: Option<Arc<dyn TranscriptSource>>,
    pub lease: Option<Box<dyn Send>>,
    /// Feed loss ends the device watch so its next subscription can attach afresh.
    pub live: Option<tokio::sync::watch::Receiver<bool>>,
}

struct UnwiredResolver;

#[async_trait]
impl SourceResolver for UnwiredResolver {
    async fn resolve(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        Ok(None)
    }
}

struct UnwiredTargets;

#[async_trait]
impl send::SendTargets for UnwiredTargets {
    async fn open(&self, _: &str, _: Option<&str>) -> Result<Arc<dyn send::SendTarget>, ReadError> {
        Err(ReadError::new(
            "provider_unavailable",
            "no runner connection is configured for app sends",
        ))
    }
}

pub struct AppReader {
    resolver: Arc<dyn SourceResolver>,
    reader: TranscriptReader,
    pub display: Arc<display_hub::DisplayHub>,
    pub watches: Arc<WatchRegistry>,
    pub sends: Arc<send::Sender>,
    /// Absent when the module's store could not be opened: uploads are then refused.
    pub uploads: Option<Arc<uploads::Uploads>>,
    /// The thumbnails `session.media` has made.
    thumbs: crate::media::Thumbs,
    /// The tool display reads, once the display store is open; refused until then.
    tool_display: std::sync::OnceLock<Arc<crate::display::app::ToolDisplay>>,
    /// The model names, once the module can call other modules; refused until
    /// then.
    model_names: std::sync::OnceLock<Arc<crate::display::model_names::ModelNames>>,
    /// The notices that rode sessions' calls, once their file is open;
    /// refused until then.
    notices: std::sync::OnceLock<Arc<crate::notices::Notices>>,
}

/// What a message to the session `head` describes may carry, as its details
/// report it (`crate::display::model_names::attachment_limits`).
async fn attachment_limits(
    names: Option<&Arc<crate::display::model_names::ModelNames>>,
    head: &occipital_app_reader::source::Head,
) -> crate::display::model_names::AttachmentLimits {
    crate::display::model_names::attachment_limits(
        names,
        head.model.as_ref(),
        head.input.max_image_bytes,
        head.input.max_image_dimension,
    )
    .await
}

impl Default for AppReader {
    fn default() -> Self {
        Self::new(Arc::new(UnwiredResolver))
    }
}

impl AppReader {
    pub fn new(resolver: Arc<dyn SourceResolver>) -> Self {
        Self::with_sender(
            resolver,
            Arc::new(send::Sender::new(Arc::new(UnwiredTargets), None)),
        )
    }

    pub fn with_sender(resolver: Arc<dyn SourceResolver>, sends: Arc<send::Sender>) -> Self {
        Self {
            resolver,
            reader: TranscriptReader::default(),
            display: Arc::default(),
            watches: Arc::default(),
            sends,
            uploads: None,
            thumbs: crate::media::Thumbs::default(),
            tool_display: std::sync::OnceLock::new(),
            model_names: std::sync::OnceLock::new(),
            notices: std::sync::OnceLock::new(),
        }
    }

    /// Serve `session.notices` and its watch from `notices`. The notices file
    /// opens after the reader is built, so this is set later, once.
    pub fn set_notices(&self, notices: Arc<crate::notices::Notices>) {
        let _ = self.notices.set(notices);
    }

    pub(crate) fn notices(&self) -> Result<&Arc<crate::notices::Notices>, ReadError> {
        self.notices.get().ok_or_else(|| {
            ReadError::new(
                "provider_unavailable",
                "notices are unavailable: the notices file isn't open",
            )
            .with_detail(serde_json::json!({"retryable": true}))
        })
    }

    /// Serve `display.model_names` from `names`, once, and hold sends to the
    /// attachment limits it reads.
    pub fn set_model_names(&self, names: Arc<crate::display::model_names::ModelNames>) {
        self.sends.set_model_names(names.clone());
        let _ = self.model_names.set(names);
    }

    /// Serve the tool display reads from `tool`. The display store opens
    /// after the reader is built, so this is set later, once.
    pub fn set_tool_display(&self, tool: Arc<crate::display::app::ToolDisplay>) {
        let _ = self.tool_display.set(tool);
    }

    fn tool_display(&self) -> Result<&Arc<crate::display::app::ToolDisplay>, ReadError> {
        self.tool_display.get().ok_or_else(|| {
            ReadError::new(
                "provider_unavailable",
                "tool display data is unavailable: the display store isn't open",
            )
            .with_detail(serde_json::json!({"retryable": true}))
        })
    }

    /// A tool display read: views and templates need no session; records and
    /// a detail are read for the session the app names, resolved as a
    /// transcript read resolves it.
    async fn tool_display_read(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        // Model names need no display store.
        if method == crate::display::model_names::OPERATION {
            let names = self.model_names.get().ok_or_else(|| {
                ReadError::new(
                    "provider_unavailable",
                    "model names are unavailable: this module can't reach the daemon",
                )
            })?;
            return names.answer(&params).await;
        }
        let tool = self.tool_display()?;
        match method {
            crate::display::app::VIEWS => tool.views(&params).await,
            crate::display::app::TEMPLATE => tool.template(&params).await,
            _ => {
                let reference = self.session_reference(&params).await?;
                if method == crate::display::app::RECORDS {
                    tool.records(&reference.runner, &reference.session, &params)
                        .await
                } else {
                    tool.detail(&reference.runner, &reference.session, &params)
                        .await
                }
            }
        }
    }

    /// The runner and runner-side session for a request's `session` and
    /// optional `harness`.
    async fn session_reference(&self, params: &Value) -> Result<resolve::SessionRef, ReadError> {
        let session = params["session"]
            .as_str()
            .filter(|session| !session.is_empty())
            .ok_or_else(|| {
                ReadError::new("invalid_params", "session is missing or malformed")
                    .with_detail(serde_json::json!({"field": "session"}))
            })?;
        let harness = occipital_app_reader::page::optional_transcript_harness(params)
            .map_err(|error| ReadError::new(error.code, error.message))?;
        observe::step("resolution", self.resolver.reference(session, harness)).await
    }

    pub fn with_uploads(mut self, uploads: Option<Arc<uploads::Uploads>>) -> Self {
        self.uploads = uploads;
        self
    }

    async fn upload(
        &self,
        caller: &Caller,
        method: &str,
        params: Value,
        payload: Option<Vec<u8>>,
    ) -> Result<Value, ReadError> {
        let Some(uploads) = &self.uploads else {
            return Err(ReadError::new(
                "provider_unavailable",
                "upload storage is unavailable",
            ));
        };
        uploads
            .serve(caller, method, params, payload, self.resolver.clone())
            .await
    }

    async fn source(&self, params: &Value) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        let request = PageRequest::parse(params).map_err(ReadError::from)?;
        observe::step(
            "resolution",
            self.resolver
                .resolve(&request.session_id, request.requested_harness.as_deref()),
        )
        .await
    }

    /// A request from no particular caller, as tests send them.
    #[cfg(test)]
    async fn read(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        self.read_as(None, method, params).await
    }

    /// Answer one request from `caller`, the route's admitted caller. Only a send
    /// reads it: the files a send names must be uploads the same device opened.
    async fn read_as(
        &self,
        caller: Option<&Caller>,
        method: &str,
        params: Value,
    ) -> Result<Value, ReadError> {
        if DISPLAY_METHODS.contains(&method) {
            return self.tool_display_read(method, params).await;
        }
        if method == crate::notices::READ {
            let notices = self.notices()?;
            let reference = self.session_reference(&params).await?;
            return notices
                .read(&reference.runner, &reference.session, &params)
                .await;
        }
        if method == crate::notices::wakes::READ {
            let notices = self.notices()?;
            let reference = self.session_reference(&params).await?;
            return notices
                .wake_records(&reference.runner, &reference.session, &params)
                .await;
        }
        if method == STOP {
            return self.sends.stop(params).await.map_err(|refusal| ReadError {
                code: refusal.code,
                message: refusal.message,
                detail: refusal.detail,
            });
        }
        if method == STOP_ALL {
            return self
                .sends
                .stop_all(params)
                .await
                .map_err(|refusal| ReadError {
                    code: refusal.code,
                    message: refusal.message,
                    detail: refusal.detail,
                });
        }
        if method == SEND {
            let device = match caller {
                Some(Caller::Device { fingerprint, .. }) => Some(fingerprint.as_str()),
                _ => None,
            };
            return self
                .sends
                .send_from(device, params)
                .await
                .map_err(|refusal| ReadError {
                    code: refusal.code,
                    message: refusal.message,
                    detail: refusal.detail,
                });
        }
        if method == DISPLAY_READ {
            let request = DisplayReadRequest::parse(&params)
                .map_err(|e| ReadError::new(e.code, e.message))?;
            let harness = occipital_app_reader::page::optional_transcript_harness(&params)
                .map_err(|e| ReadError::new(e.code, e.message))?;
            observe::step(
                "resolution",
                self.resolver.validate_session(request.session, harness),
            )
            .await?;
            return serde_json::to_value(
                request.read(self.display.projection(request.session).as_deref()),
            )
            .map_err(|error| {
                ReadError::new(
                    "internal",
                    format!("cannot encode the display read: {error}"),
                )
            });
        }
        if method == MEDIA {
            return observe::step(
                "media",
                crate::media::answer(&params, self.resolver.as_ref(), &self.thumbs),
            )
            .await;
        }
        if method == REQUEST_CONTEXT {
            return observe::step(
                "request_context",
                crate::request_context::answer(&params, self.resolver.as_ref()),
            )
            .await;
        }
        if method == DETAILS {
            let request = occipital_app_reader::details::DetailsRequest::parse(&params)?;
            // The same resolver as the transcript reads, so a session they refuse
            // is refused here too.
            let source = observe::step(
                "resolution",
                self.resolver
                    .resolve(&request.session, request.harness.as_deref()),
            )
            .await?;
            let (mut reply, head) =
                occipital_app_reader::details::read_with_head(source.as_deref(), &request).await?;
            // Only for an app that asks, so the details every other app reads
            // keep their bytes.
            if params["attachmentLimits"] == true {
                let limits = observe::step(
                    "attachment_limits",
                    attachment_limits(self.model_names.get(), &head),
                )
                .await;
                reply["attachments"] = limits.to_json();
            }
            self.sends.attach_queued(&mut reply, &request.session).await;
            return Ok(reply);
        }
        // The reader owns typed claim refusals as well as malformed request errors.
        let Ok(request) = PageRequest::parse(&params) else {
            return self.reader.read(None, params).await;
        };
        let session = request.session_id.clone();
        let source = self.source(&params).await?;
        let mut reply = self.reader.read(source.as_deref(), params).await?;
        // Every page reply carries the session's queued and unsent sends, whatever its
        // verdict: a queued send changes nothing stored, so an `unchanged` claim is the
        // only reply a polling phone gets while one waits.
        self.sends.attach_queued(&mut reply, &session).await;
        Ok(reply)
    }
}

fn served(method: &str) -> bool {
    METHODS.contains(&method)
        || DISPLAY_METHODS.contains(&method)
        || NOTICE_METHODS.contains(&method)
        || method == SEND
        || method == STOP
        || method == STOP_ALL
        || uploads::METHODS.contains(&method)
}

/// The route facts admission reads, captured from the daemon's `route.bind`.
///
/// The operator-app stamp arrives because the manifest declares
/// `operator-app/v1`, and only once the daemon's operator-app policy is on.
/// Released daemons keep that policy off until the desktop app's first release.
/// Until then the desktop connects as a plain `direct` caller and is admitted
/// by [`DEVELOPMENT_DIRECT_ADMITTED`].
pub fn binding(request: &subc_client_rs::RouteBindRequest) -> RouteBinding {
    (
        request.principal.clone(),
        request.target.clone(),
        request.scope.clone(),
        Some(request.identity.clone()),
        request.peer.operator_app.clone(),
    )
}

/// Whether a request body names one of the app's watches, peeked without
/// parsing its params. The SDK classifies every request before it takes a handler slot,
/// so this must stay a cheap scan.
pub fn is_watch(body: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Method<'a> {
        #[serde(borrow)]
        method: Option<std::borrow::Cow<'a, str>>,
    }
    serde_json::from_slice::<Method<'_>>(body)
        .ok()
        .and_then(|head| head.method)
        .is_some_and(|method| {
            matches!(
                method.as_ref(),
                TRANSCRIPT_WATCH
                    | DISPLAY_WATCH
                    | crate::display::app::RECORDS_WATCH
                    | crate::notices::WATCH
            )
        })
}

/// An app operation a request carries: its method, its params, and for a framed
/// request (`FLAG_BINARY`) the raw payload after the JSON head.
pub(crate) struct AppRequest {
    pub method: String,
    pub params: Value,
    pub payload: Option<Vec<u8>>,
    /// The whole request body's length, payload included.
    pub body_len: usize,
}

/// Recognize the method name before inspecting params, so a caller admission refuses
/// cannot learn from the refusal whether a session exists.
///
/// A framed body (`binary`, the request's `FLAG_BINARY`) is `u32 big-endian head
/// length | JSON head | payload`; the head is the same `{method, params}` object an
/// unframed request carries. The payload is moved out of `body` rather than copied.
/// Anything else is refused `invalid_request`, naming a framing error when there is one.
pub(crate) fn request(mut body: Vec<u8>, binary: bool) -> Result<AppRequest, HandlerOutcome> {
    let body_len = body.len();
    let unknown = || {
        refusal(
            "invalid_request",
            if body_len > MAX_REQUEST_BYTES {
                "request exceeds 64 KiB"
            } else {
                "unknown management method"
            },
        )
    };
    let (mut head, payload_len) = if binary {
        match subc_protocol::framed_body::decode(&body) {
            Ok((head, payload)) => (head, Some(payload.len())),
            Err(error) => {
                return Err(refusal(
                    "invalid_request",
                    format!("malformed framed body: {error}"),
                ))
            }
        }
    } else {
        (
            serde_json::from_slice::<Value>(&body).map_err(|_| unknown())?,
            None,
        )
    };
    let method = match head.get("method").and_then(Value::as_str) {
        Some(method) if served(method) => method.to_owned(),
        _ => return Err(unknown()),
    };
    let params = head
        .get_mut("params")
        .map(Value::take)
        .unwrap_or(Value::Null);
    let payload = payload_len.map(|len| body.split_off(body_len - len));
    Ok(AppRequest {
        method,
        params,
        payload,
        body_len,
    })
}

/// The largest app request apart from an upload chunk's payload.
const MAX_REQUEST_BYTES: usize = 64 * 1024;

fn refusal(code: &str, message: impl Into<String>) -> HandlerOutcome {
    HandlerOutcome::Error {
        code: code.into(),
        message: message.into(),
    }
}

impl AppReader {
    /// Serve one app request on a management-surface route. `binding` is what the
    /// route was bound with; `None` for a route this module never bound.
    pub async fn serve(
        self: &Arc<Self>,
        binding: Option<RouteBinding>,
        ctx: RequestCtx,
        body: Vec<u8>,
    ) -> HandlerOutcome {
        // The SDK decodes a framed body only when the request carried FLAG_BINARY;
        // a malformed one is still framed, and refused below.
        let binary = !matches!(ctx.request_payload(), Ok(None));
        self.serve_body(binding, body, binary, Box::new(route::Sdk(ctx)))
            .await
    }

    /// Serve a request's body as it arrived, `binary` when it carried FLAG_BINARY.
    pub(crate) async fn serve_body(
        self: &Arc<Self>,
        binding: Option<RouteBinding>,
        body: Vec<u8>,
        binary: bool,
        exchange: Box<dyn route::Exchange>,
    ) -> HandlerOutcome {
        match request(body, binary) {
            Ok(request) => self.serve_request(binding, request, exchange).await,
            Err(refused) => refused,
        }
    }

    /// Serve a request already taken apart, answering through `exchange`.
    pub(crate) async fn serve_request(
        self: &Arc<Self>,
        binding: Option<RouteBinding>,
        request: AppRequest,
        exchange: Box<dyn route::Exchange>,
    ) -> HandlerOutcome {
        let AppRequest {
            method,
            params,
            payload,
            body_len,
        } = request;
        let caller = match admit(binding.as_ref(), &method) {
            Ok(caller) => caller,
            Err(error) => return route::error_outcome(&error),
        };
        let (channel, epoch, corr) = exchange.key();
        let correlation = format!("{channel}:{epoch}:{corr}");
        if uploads::METHODS.contains(&method.as_str()) {
            // The JSON head is held to the same 64 KiB as any app request; the
            // payload is bounded by the operation's declaration instead.
            let head_len = body_len - payload.as_ref().map_or(0, Vec::len);
            if head_len > 4 + subc_protocol::framed_body::MAX_FRAMED_HEAD_BYTES {
                return refusal("invalid_request", "request exceeds 64 KiB");
            }
            // Logged like every other app request, so each upload call leaves
            // its line: `upload.finish`, `upload.abort` and `upload.place` at
            // INFO even when fast. Only `upload.begin` names the session; the
            // later calls name their upload instead.
            let subject = params["session"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    params["upload_id"]
                        .as_str()
                        .map(|id| format!("upload:{id}"))
                })
                .unwrap_or_default();
            return observe::request(&method, &subject, correlation, async {
                match self.upload(&caller, &method, params, payload).await {
                    Ok(value) => route::result_outcome(value),
                    Err(error) => {
                        observe::outcome(error.code);
                        route::error_outcome(&error)
                    }
                }
            })
            .await;
        }
        if payload.is_some() {
            return refusal("invalid_request", format!("{method} takes no payload"));
        }
        if body_len > MAX_REQUEST_BYTES {
            return refusal("invalid_request", "request exceeds 64 KiB");
        }
        let records_watch = method == crate::display::app::RECORDS_WATCH;
        let notices_watch = method == crate::notices::WATCH;
        if !records_watch
            && !notices_watch
            && !matches!(method.as_str(), TRANSCRIPT_WATCH | DISPLAY_WATCH)
        {
            return route::serve_read(self.clone(), caller, method, params, correlation).await;
        }
        let slot = match self.watches.open(WatchRoute {
            channel: channel.into(),
            epoch: epoch.into(),
        }) {
            Ok(slot) => slot,
            Err(error) => {
                // Both limits answer `watch_capacity`, which an app retries once
                // it has closed a watch.
                let (code, message) = match error {
                    OpenRefusal::Capacity => ("watch_capacity", "app watch cannot be opened"),
                    OpenRefusal::RouteCapacity => (
                        "watch_capacity",
                        "this connection already holds 24 watches; close one before opening another",
                    ),
                    OpenRefusal::Stopping => ("module_stopping", "app watch cannot be opened"),
                };
                return refusal(code, message);
            }
        };
        if records_watch {
            return route::serve_records_watch(self.clone(), params, exchange, slot).await;
        }
        if notices_watch {
            return route::serve_notices_watch(self.clone(), params, exchange, slot).await;
        }
        route::serve(
            self.clone(),
            binding,
            method,
            params,
            exchange,
            correlation,
            slot,
        )
        .await
    }

    /// End the watches open on a route that has gone.
    pub fn end_route(&self, channel: u16, epoch: u32) {
        self.watches.end_route(WatchRoute {
            channel: channel.into(),
            epoch: epoch.into(),
        });
    }

    /// End every open watch and refuse new ones: the module is stopping.
    pub fn shutdown(&self) {
        self.watches.end_all();
    }
}

#[cfg(test)]
mod tests;
