use super::*;
use occipital_app_reader::{
    display::DisplayPartType,
    display_events::DisplayInput,
    source::{Anchor, Head, HistoryPage, ReadError, Row},
};
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use subc_protocol::{
    scope::{ScopeKind, ScopeStamp},
    BindIdentity, Flags, Frame, FrameType, Principal, Priority,
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct FakeSource {
    reads: AtomicUsize,
    stalled: AtomicBool,
    head_started: tokio::sync::Notify,
    head_release: tokio::sync::Notify,
    changes: watch::Sender<u64>,
}

impl FakeSource {
    fn page(&self) -> HistoryPage {
        self.reads.fetch_add(1, Ordering::SeqCst);
        HistoryPage {
            rows: vec![Row {
                mid: "m1".into(),
                ordinal: Some(1),
                written_at: Some(42),
                display: json!({"role":"user", "text":"hello", "clientMessageId":"phone-1", "blockSummaries":[], "parts":[]}),
            }],
            history_version: Some(1),
            ..Default::default()
        }
    }
}

#[async_trait]
impl TranscriptSource for FakeSource {
    fn harness(&self) -> &'static str {
        "opencode2"
    }
    async fn head(&self) -> Result<Head, ReadError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.stalled.load(Ordering::SeqCst) {
            self.head_started.notify_one();
            self.head_release.notified().await;
        }
        Ok(Head {
            first_ordinal: Some(1),
            history_version: Some(1),
            ..Default::default()
        })
    }
    async fn newest(&self, _: u32) -> Result<HistoryPage, ReadError> {
        Ok(self.page())
    }
    async fn older(&self, _: &Anchor, _: u32) -> Result<HistoryPage, ReadError> {
        Ok(self.page())
    }
    async fn newer(&self, _: &str, _: u32) -> Result<HistoryPage, ReadError> {
        Ok(HistoryPage {
            rows: vec![],
            ..self.page()
        })
    }
    async fn exact(&self, _: &Anchor) -> Result<Option<Row>, ReadError> {
        Ok(self.page().rows.pop())
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    fn activity(&self) -> Option<(&'static str, i64)> {
        Some(("idle", 42))
    }
}

#[derive(Default)]
struct FakeResolver {
    source: Arc<FakeSource>,
    resolutions: AtomicUsize,
}

#[async_trait]
impl SourceResolver for FakeResolver {
    async fn resolve(
        &self,
        session: &str,
        _: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        self.resolutions.fetch_add(1, Ordering::SeqCst);
        Ok((session == "s").then(|| self.source.clone() as Arc<dyn TranscriptSource>))
    }

    /// Session `s` references one three-byte image.
    async fn media_read(
        &self,
        session: &str,
        _: Option<&str>,
        _: &str,
        offset: u64,
        length: u64,
    ) -> Result<Value, ReadError> {
        self.resolutions.fetch_add(1, Ordering::SeqCst);
        if session != "s" {
            return Err(ReadError::new("unknown_media", "not in this session"));
        }
        let bytes = &b"png"[offset.min(3) as usize..(offset + length).min(3) as usize];
        Ok(json!({"size": 3, "mime": "image/png", "data": crate::base64::encode(bytes)}))
    }
}

/// Serve one request on `method` through the module's own handler, as the device
/// route does, and return the body of the handler's first frame: a read's
/// `{"result":…}` reply or a watch's first notice.
pub(crate) async fn serve(app: &Arc<AppReader>, method: &str, params: &Value) -> Vec<u8> {
    serve_on_channel(app, method, params, 1).await
}

pub(crate) async fn serve_on_channel(
    app: &Arc<AppReader>,
    method: &str,
    params: &Value,
    channel: u16,
) -> Vec<u8> {
    let frame = serve_frame(app, method, params, channel, Duration::from_secs(5)).await;
    let expected = if method == TRANSCRIPT_WATCH {
        FrameType::StreamData
    } else {
        FrameType::Response
    };
    assert_eq!(
        frame.header.ty,
        expected,
        "{method}: {}",
        String::from_utf8_lossy(&frame.body)
    );
    frame.body
}

pub(crate) async fn serve_frame(
    app: &Arc<AppReader>,
    method: &str,
    params: &Value,
    channel: u16,
    response_timeout: Duration,
) -> Frame {
    let request = Frame::build(
        FrameType::Request,
        Flags::new(false, Priority::Interactive, false),
        channel,
        1,
        1,
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
        Some(BindIdentity::new("phone:test", "caller")),
        None,
    );
    let (tx, mut rx) = mpsc::channel(16);
    let mut connection = Connection::new(app.clone());
    connection
        .dispatch(
            request,
            Some(binding),
            method.into(),
            params.clone(),
            None,
            &tx,
        )
        .await
        .unwrap();
    let frame = tokio::time::timeout(response_timeout, rx.recv())
        .await
        .unwrap()
        .unwrap();
    connection.end_route(channel, 1);
    connection.join().await;
    frame
}

/// One request's exchange in these tests: its notices and its terminal frame go
/// to `frames`, as the SDK writes them to the daemon.
struct TestExchange {
    key: (u16, u32, u64),
    frames: mpsc::Sender<Frame>,
    cancelled: CancellationToken,
}

fn reply_frame(key: (u16, u32, u64), ty: FrameType, body: Vec<u8>) -> Frame {
    Frame::build(
        ty,
        Flags::new(false, Priority::Passive, false),
        key.0,
        key.1,
        key.2,
        body,
    )
    .unwrap()
}

#[async_trait]
impl route::Exchange for TestExchange {
    fn key(&self) -> (u16, u32, u64) {
        self.key
    }
    async fn emit(&self, body: Vec<u8>) -> Result<(), ()> {
        // The SDK drops a notice emitted after its request was cancelled.
        if self.cancelled.is_cancelled() {
            return Ok(());
        }
        self.frames
            .send(reply_frame(self.key, FrameType::StreamData, body))
            .await
            .map_err(|_| ())
    }
    fn cancellation(&self) -> CancellationToken {
        self.cancelled.clone()
    }
}

/// The frame the SDK writes for a handler's outcome.
fn terminal(key: (u16, u32, u64), outcome: HandlerOutcome) -> Frame {
    match outcome {
        HandlerOutcome::Response(body) => reply_frame(key, FrameType::Response, body),
        HandlerOutcome::Error { code, message } => reply_frame(
            key,
            FrameType::Error,
            serde_json::to_vec(&json!({"code": code, "message": message})).unwrap(),
        ),
        HandlerOutcome::ErrorWithDetail {
            code,
            message,
            detail,
        } => reply_frame(
            key,
            FrameType::Error,
            serde_json::to_vec(&json!({"code": code, "message": message, "detail": detail}))
                .unwrap(),
        ),
        HandlerOutcome::Streamed => reply_frame(key, FrameType::StreamEnd, Vec::new()),
    }
}

/// Serves requests the way the module SDK does, without a daemon: each request
/// in its own task through [`AppReader::serve_request`], a Cancel through the
/// request's token, and every frame written to the sender the request names.
pub(crate) struct Connection {
    app: Arc<AppReader>,
    tasks: tokio::task::JoinSet<()>,
    cancels: std::collections::BTreeMap<(u16, u32, u64), CancellationToken>,
}

impl Connection {
    pub(crate) fn new(app: Arc<AppReader>) -> Self {
        Self {
            app,
            tasks: tokio::task::JoinSet::new(),
            cancels: std::collections::BTreeMap::new(),
        }
    }

    /// Serve one request, as the SDK hands it to the handler.
    pub(crate) async fn dispatch(
        &mut self,
        frame: Frame,
        binding: Option<RouteBinding>,
        method: String,
        params: Value,
        payload: Option<Vec<u8>>,
        tx: &mpsc::Sender<Frame>,
    ) -> Result<(), crate::Error> {
        let key = (frame.header.channel, frame.header.epoch, frame.header.corr);
        let cancelled = CancellationToken::new();
        self.cancels.insert(key, cancelled.clone());
        let exchange = TestExchange {
            key,
            frames: tx.clone(),
            cancelled,
        };
        let request = AppRequest {
            method,
            params,
            body_len: frame.body.len() + payload.as_ref().map_or(0, Vec::len),
            payload,
        };
        let app = self.app.clone();
        let tx = tx.clone();
        self.tasks.spawn(async move {
            let outcome = app
                .serve_request(binding, request, Box::new(exchange))
                .await;
            let _ = tx.send(terminal(key, outcome)).await;
        });
        Ok(())
    }

    /// Serve one request from its raw body, framed when `binary`, as the handler
    /// receives it from the SDK.
    pub(crate) fn dispatch_body(
        &mut self,
        key: (u16, u32, u64),
        binding: Option<RouteBinding>,
        body: Vec<u8>,
        binary: bool,
        tx: &mpsc::Sender<Frame>,
    ) {
        let cancelled = CancellationToken::new();
        self.cancels.insert(key, cancelled.clone());
        let exchange = TestExchange {
            key,
            frames: tx.clone(),
            cancelled,
        };
        let app = self.app.clone();
        let tx = tx.clone();
        self.tasks.spawn(async move {
            let outcome = app
                .serve_body(binding, body, binary, Box::new(exchange))
                .await;
            let _ = tx.send(terminal(key, outcome)).await;
        });
    }

    /// The app's Cancel for one request.
    pub(crate) fn cancel(&mut self, key: (u16, u32, u64)) {
        if let Some(cancelled) = self.cancels.get(&key) {
            cancelled.cancel();
        }
    }

    /// The route went: what the handler's `on_route_gone` does.
    pub(crate) fn end_route(&self, channel: u16, epoch: u32) {
        self.app.end_route(channel, epoch);
    }

    /// Wait for every request to send its terminal frame.
    pub(crate) async fn join(&mut self) {
        while self.tasks.join_next().await.is_some() {}
    }

    /// The module is stopping: end every watch, then wait for every request.
    pub(crate) async fn shutdown(&mut self) {
        self.app.shutdown();
        self.join().await;
    }
}

#[tokio::test(start_paused = true)]
async fn reader_capture_survives_reads_started_before_collector_installation() {
    // Other tests may install logging before this request starts. A separate
    // process preserves the order: start without logging, then install capture.
    const CHILD: &str = "THALAMUS_READER_CAPTURE_STARTUP_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app_reader::tests::reader_capture_survives_reads_started_before_collector_installation",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated reader capture failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let resolver = Arc::new(FakeResolver::default());
    resolver.source.stalled.store(true, Ordering::SeqCst);
    let app = Arc::new(AppReader::new(resolver.clone()));
    let old_app = app.clone();
    let started_before_capture = tokio::spawn(async move {
        serve(
            &old_app,
            TRANSCRIPT_PAGE,
            &json!({"session":"s", "limit":1}),
        )
        .await
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        resolver.source.head_started.notified(),
    )
    .await
    .unwrap();

    let (log, _capture) = super::observe::tests::capture();
    resolver.source.stalled.store(false, Ordering::SeqCst);
    resolver.source.head_release.notify_one();
    started_before_capture.await.unwrap();
    serve(&app, TRANSCRIPT_PAGE, &json!({"session":"s", "limit":1})).await;
    let text = log.text();
    assert_eq!(
        text.matches("app reader request finished").count(),
        2,
        "{text}"
    );
}

/// A runner session behind the app send: a worker's (when `worker` is set), whose
/// plan grants only steers, or a head's. A held steer expires once `expired` is set.
#[derive(Default)]
struct FakeSends {
    opened: AtomicUsize,
    calls: AtomicUsize,
    worker: AtomicBool,
    expired: AtomicBool,
}

struct FakeSendTarget(Arc<FakeSends>, resolve::SessionRef);

#[async_trait]
impl send::SendTarget for FakeSendTarget {
    fn reference(&self) -> &resolve::SessionRef {
        &self.1
    }
    async fn call(&self, method: &'static str, params: Value) -> Result<Value, send::RunnerError> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        if method == "session.head" {
            return Ok(
                json!({"history_version": 1, "incarnation": "inc-a", "first_ordinal": 1,
                "updated_at": 1, "last_run_state": {"run_id": "r1", "state": "active"}}),
            );
        }
        if method == "role.describe" {
            return Ok(serde_json::to_value(
                cortexkit_role_llm_runner::describe::RoleDescribe::default(),
            )
            .unwrap());
        }
        let starts = params["delivery"]
            .as_array()
            .is_some_and(|modes| modes.contains(&json!("idle_start")));
        if self.0.worker.load(Ordering::SeqCst) && starts {
            return Err(send::RunnerError::Refused {
                code: "invalid_params".into(),
                message: "the session's plan does not let module occipital start runs".into(),
                detail: Some(json!({"field": "delivery"})),
            });
        }
        if self.0.expired.load(Ordering::SeqCst) {
            return Ok(json!({"state": "expired"}));
        }
        Ok(if starts {
            json!({"state": "active", "run_id": "r1"})
        } else {
            json!({"state": "pending", "submission_id": "sub-1"})
        })
    }
}

struct SharedSends(Arc<FakeSends>);

#[async_trait]
impl send::SendTargets for SharedSends {
    async fn open(
        &self,
        session: &str,
        _: Option<&str>,
    ) -> Result<Arc<dyn send::SendTarget>, ReadError> {
        self.0.opened.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(FakeSendTarget(
            self.0.clone(),
            resolve::SessionRef {
                runner: "broca".into(),
                session: session.into(),
            },
        )))
    }
}

fn binding(harness: &str) -> RouteBinding {
    (
        Some(Principal::Reserved {
            module_id: "callosum".into(),
        }),
        RouteTarget::ManagementSurface {
            module_id: crate::MODULE_ID.into(),
        },
        None,
        Some(BindIdentity::new(harness, "caller")),
        None,
    )
}

fn frame(ty: FrameType, channel: u16, epoch: u32, corr: u64, body: Value) -> Frame {
    Frame::build_with_version(
        subc_protocol::PROTOCOL_VERSION,
        ty,
        Flags::new(false, Priority::Passive, false),
        channel,
        epoch,
        corr,
        serde_json::to_vec(&body).unwrap(),
    )
    .unwrap()
}

/// A reader served as the handler serves it, with fake runners behind it, and
/// the routes these tests bind.
struct Harness {
    connection: Connection,
    tx: mpsc::Sender<Frame>,
    output: mpsc::Receiver<Frame>,
    routes: std::collections::HashMap<(u16, u32), RouteBinding>,
    app: Arc<AppReader>,
    resolver: Arc<FakeResolver>,
    sends: Arc<FakeSends>,
}

impl Harness {
    fn new() -> Self {
        let resolver = Arc::new(FakeResolver::default());
        let sends = Arc::new(FakeSends::default());
        let sender = send::Sender::new(Arc::new(SharedSends(sends.clone())), None);
        let app = Arc::new(AppReader::with_sender(resolver.clone(), Arc::new(sender)));
        let (tx, output) = mpsc::channel(256);
        Self {
            connection: Connection::new(app.clone()),
            tx,
            output,
            routes: std::collections::HashMap::new(),
            app,
            resolver,
            sends,
        }
    }

    async fn next(&mut self) -> Frame {
        self.next_within(3).await
    }
    async fn next_within(&mut self, seconds: u64) -> Frame {
        tokio::time::timeout(std::time::Duration::from_secs(seconds), self.output.recv())
            .await
            .expect("the reader must reply or end the stream")
            .expect("egress remains open")
    }
    async fn bind(&mut self, channel: u16, epoch: u32, binding: RouteBinding) {
        self.routes.insert((channel, epoch), binding);
    }
    async fn request(&mut self, method: &str, corr: u64, params: Value) {
        self.request_on(7, 1, method, corr, params).await;
    }
    async fn request_on(
        &mut self,
        channel: u16,
        epoch: u32,
        method: &str,
        corr: u64,
        params: Value,
    ) {
        let request = frame(
            FrameType::Request,
            channel,
            epoch,
            corr,
            json!({"method": method, "params": params}),
        );
        let binding = self.routes.get(&(channel, epoch)).cloned();
        self.connection
            .dispatch(request, binding, method.into(), params, None, &self.tx)
            .await
            .unwrap();
    }
    fn cancel(&mut self, channel: u16, epoch: u32, corr: u64) {
        self.connection.cancel((channel, epoch, corr));
    }
    /// Let every request task run until it waits on something these tests drive.
    async fn barrier(&mut self) {
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
    }
    async fn finish(mut self) {
        self.connection.shutdown().await;
        assert_eq!(self.app.watches.open_count(), 0);
    }
}

fn params(method: &str, session: &str) -> Value {
    if method == SEND {
        json!({"session":session, "text":"hello", "clientMessageId":"m1"})
    } else if method == TRANSCRIPT_WATCH {
        json!({"session":session,"claim":{"heldFloorMessageId":"m1","sinceMessageId":"m1","heldVersion":"history:1"}})
    } else if method == MEDIA {
        json!({"session":session, "sha256":"ab".repeat(32), "size":"full"})
    } else {
        json!({"session":session})
    }
}

fn body(frame: &Frame) -> Value {
    serde_json::from_slice(&frame.body).unwrap()
}

#[derive(Default)]
struct FailingConsumerConnect(AtomicUsize);

#[async_trait]
impl broca::SourceConnector for FailingConsumerConnect {
    async fn connect(&self) -> Result<Arc<broca::BrocaSources>, ReadError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ReadError::new(
            "provider_unavailable",
            "injected daemon consumer connect failure",
        ))
    }
}

/// A daemon connection for reads that cannot be opened refuses each read, which
/// tries it again, and never touches the handler's health answer.
#[tokio::test]
async fn reads_retry_a_failed_consumer_connect_and_health_never_connects() {
    let connector = Arc::new(FailingConsumerConnect::default());
    let resolver = Arc::new(broca::LazyBrocaSources::new(connector.clone()));
    let app = Arc::new(AppReader::new(resolver));
    let handler =
        crate::serve::OccipitalHandler::new(None, Box::new(|_| {})).with_app_reader(app.clone());
    use subc_client_rs::ModuleHandler;
    let initial = handler.health().await;
    assert_eq!(connector.0.load(Ordering::SeqCst), 0);
    let (tx, mut output) = mpsc::channel(16);
    let mut connection = Connection::new(app);
    for corr in [3, 4] {
        let params = json!({"session":"alfonso:s1", "harness":"broca"});
        let request = frame(
            FrameType::Request,
            7,
            1,
            corr,
            json!({"method":TRANSCRIPT_PAGE, "params":params}),
        );
        connection
            .dispatch(
                request,
                Some(binding("fed-phone:test")),
                TRANSCRIPT_PAGE.into(),
                params,
                None,
                &tx,
            )
            .await
            .unwrap();
        let refused = output.recv().await.unwrap();
        assert_eq!(refused.header.ty, FrameType::Error);
        assert_eq!(refused.header.corr, corr);
        assert_eq!(body(&refused)["code"], "provider_unavailable");
    }
    assert_eq!(connector.0.load(Ordering::SeqCst), 2);
    let after = handler.health().await;
    assert_eq!(after.status, initial.status);
    assert_eq!(connector.0.load(Ordering::SeqCst), 2);
    connection.shutdown().await;
}

#[test]
fn the_manifest_declares_every_watch_as_a_subscribable_stream() {
    let manifest = serde_json::to_value(crate::manifest::manifest().unwrap()).unwrap();
    let surface = manifest["provides"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["role"] == "management_surface")
        .expect("management surface");
    assert_eq!(
        surface["concurrency"],
        serde_json::to_value(Concurrency::ModuleManaged).unwrap()
    );
    for name in METHODS {
        let op = surface["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|op| op["name"] == name)
            .unwrap();
        assert_eq!(op["kind"], "query");
    }
    assert_eq!(manifest["operation_delivery"][TRANSCRIPT_WATCH], "watch");
    assert_eq!(manifest["operation_delivery"][DISPLAY_WATCH], "watch");
    assert!(manifest["operation_delivery"][TRANSCRIPT_PAGE].is_null());
    assert!(manifest["operation_delivery"][DISPLAY_READ].is_null());
    assert!(manifest["operation_delivery"][DETAILS].is_null());
}

/// The daemon sends the operator-app stamp only to a module whose manifest
/// asks for it.
#[test]
fn the_manifest_asks_for_operator_app_stamps() {
    let manifest = serde_json::to_value(crate::manifest::manifest().unwrap()).unwrap();
    assert!(manifest["capabilities"]["provides"]
        .as_array()
        .unwrap()
        .contains(&json!("operator-app/v1")));
}

fn desktop_binding() -> RouteBinding {
    let mut b = binding("desktop");
    b.0 = Some(Principal::Unverified);
    b.4 = Some(subc_protocol::peer::OperatorApp::new(
        "alfonso-desktop",
        123,
    ));
    b
}

#[test]
fn verified_desktop_admits_each_of_the_five_reads_as_operator() {
    let b = desktop_binding();
    for method in [
        "session.transcript_page",
        "session.transcript_watch",
        "session.display_read",
        "session.display_watch",
        "session.details",
    ] {
        assert_eq!(
            admit(Some(&b), method).unwrap(),
            Caller::Operator,
            "{method}"
        );
    }
}

/// The stamp alone admits only reads: a stamped route whose principal earns no admission
/// of its own is refused everything else.
fn assert_desktop_refuses(method: &str) {
    let b = desktop_binding();
    let refusal = admit(Some(&b), method).unwrap_err();
    assert_eq!(refusal.code, "carrier_op_not_permitted", "{method}");
    assert_eq!(
        refusal.message,
        format!(
            "principal {:?} is not permitted to call {method}",
            b.0.as_ref()
        )
    );
}

/// While direct callers are admitted for development, the stamp changes nothing for a
/// desktop that connects directly: its sends, Stop and uploads keep the development
/// admission they have today.
#[test]
fn the_stamp_keeps_the_development_admission_of_a_direct_desktop() {
    let mut b = desktop_binding();
    b.0 = Some(Principal::Direct);
    for method in [SEND, STOP].into_iter().chain(uploads::METHODS) {
        assert_eq!(
            admit(Some(&b), method).unwrap(),
            Caller::Device {
                class: "direct".into(),
                fingerprint: DEVELOPMENT_DIRECT_FINGERPRINT.into(),
            },
            "{method}"
        );
    }
}

#[test]
fn verified_desktop_refuses_send() {
    assert_desktop_refuses("session.enqueue_user_message");
}

#[test]
fn verified_desktop_refuses_stop() {
    assert_desktop_refuses("session.stop");
    assert_desktop_refuses("session.stop_all");
}

#[test]
fn verified_desktop_refuses_each_upload_operation() {
    for method in [
        "upload.begin",
        "upload.chunk",
        "upload.finish",
        "upload.abort",
        "upload.status",
        "upload.place",
    ] {
        assert_desktop_refuses(method);
    }
}

#[test]
fn verified_desktop_refuses_unknown_operations() {
    assert_desktop_refuses("session.future_read");
}

#[test]
fn another_operator_app_keeps_existing_admission_rules() {
    let mut b = desktop_binding();
    b.4 = Some(subc_protocol::peer::OperatorApp::new("another-app", 123));
    for method in METHODS
        .into_iter()
        .chain([SEND, STOP])
        .chain(uploads::METHODS)
    {
        assert_eq!(
            admit(Some(&b), method).unwrap_err().code,
            "carrier_op_not_permitted"
        );
        b.0 = Some(Principal::Direct);
        assert_eq!(
            admit(Some(&b), method).unwrap(),
            Caller::Device {
                class: "direct".into(),
                fingerprint: DEVELOPMENT_DIRECT_FINGERPRINT.into(),
            }
        );
        b.0 = Some(Principal::Unverified);
    }
}

#[test]
fn an_absent_operator_app_keeps_existing_admission_rules() {
    let mut b = desktop_binding();
    b.4 = None;
    for method in METHODS
        .into_iter()
        .chain([SEND, STOP])
        .chain(uploads::METHODS)
    {
        assert_eq!(
            admit(Some(&b), method).unwrap_err().code,
            "carrier_op_not_permitted"
        );
        b.0 = Some(Principal::Direct);
        assert_eq!(
            admit(Some(&b), method).unwrap(),
            Caller::Device {
                class: "direct".into(),
                fingerprint: DEVELOPMENT_DIRECT_FINGERPRINT.into(),
            }
        );
        b.0 = Some(Principal::Unverified);
    }
}

#[test]
fn verified_desktop_is_admitted_only_on_our_management_surface() {
    let mut b = desktop_binding();
    for target in [
        RouteTarget::ManagementSurface {
            module_id: "other".into(),
        },
        RouteTarget::InternalService {
            module_id: crate::MODULE_ID.into(),
            service_id: "other".into(),
        },
    ] {
        b.1 = target;
        for method in METHODS {
            assert_eq!(
                admit(Some(&b), method).unwrap_err().code,
                "carrier_op_not_permitted"
            );
        }
    }
}

/// The stamp admits the desktop to the reads and nothing else. The route here
/// is `unverified`, so the development admission for `direct` callers
/// (`DEVELOPMENT_DIRECT_ADMITTED`) doesn't apply, and sends, Stop and uploads
/// are refused.
#[tokio::test]
async fn an_operator_app_stamp_admits_the_desktop_to_reads_only() {
    for method in METHODS
        .into_iter()
        .chain([SEND, STOP])
        .chain(uploads::METHODS)
    {
        let mut h = Harness::new();
        h.app
            .display
            .ingest("s", DisplayInput::TurnEnd { run_id: None });
        h.app.display.ingest("s", DisplayInput::Reset);
        h.bind(7, 1, desktop_binding()).await;
        h.request(method, 1, params(method, "s")).await;
        let reply = h.next().await;
        if METHODS.contains(&method) {
            assert_eq!(
                reply.header.ty,
                if method.ends_with("watch") {
                    FrameType::StreamData
                } else {
                    FrameType::Response
                },
                "{method}: {}",
                body(&reply)
            );
        } else {
            assert_eq!(reply.header.ty, FrameType::Error, "{method}");
            assert_eq!(body(&reply)["code"], "carrier_op_not_permitted", "{method}");
            assert_eq!(h.resolver.resolutions.load(Ordering::SeqCst), 0);
            assert_eq!(h.sends.opened.load(Ordering::SeqCst), 0);
        }
        h.finish().await;
    }
}

#[tokio::test]
async fn rebind_without_operator_app_does_not_retain_desktop_admission() {
    let mut h = Harness::new();
    h.bind(7, 1, desktop_binding()).await;
    let mut unstamped = desktop_binding();
    unstamped.4 = None;
    h.bind(7, 2, unstamped).await;
    h.request_on(7, 2, TRANSCRIPT_PAGE, 1, params(TRANSCRIPT_PAGE, "s"))
        .await;
    let reply = h.next().await;
    assert_eq!(reply.header.ty, FrameType::Error);
    assert_eq!(body(&reply)["code"], "carrier_op_not_permitted");
    assert_eq!(h.resolver.resolutions.load(Ordering::SeqCst), 0);
    h.finish().await;
}

#[tokio::test]
async fn app_reads_admit_unscoped_callosum_routes_and_development_direct_callers() {
    let mut cases = vec![];
    for label in [
        "operator",
        "fed-phone:fp",
        "fed-sim:fp",
        "fed-desktop:fp",
        "fed-future-device:fp",
    ] {
        cases.push((label, binding(label), true));
    }
    // A plain local direct caller is admitted while apps are in development.
    let mut direct = binding("fed-phone:spoofed");
    direct.0 = Some(Principal::Direct);
    cases.push(("direct", direct, true));
    let mut unverified = binding("fed-phone:spoofed");
    unverified.0 = Some(Principal::Unverified);
    cases.push(("unverified", unverified, false));
    let mut absent = binding("fed-sim:spoofed");
    absent.0 = None;
    cases.push(("unattested", absent, false));
    for module in ["prefrontal-core", "thalamus-harness", "other"] {
        let mut b = binding("fed-phone:spoofed");
        b.0 = Some(Principal::Reserved {
            module_id: module.into(),
        });
        cases.push((module, b, false));
    }
    let mut scoped = binding("fed-phone:fp");
    scoped.2 = Some(ScopeStamp::new(
        Principal::Reserved {
            module_id: "prefrontal-core".into(),
        },
        "scope",
        1,
        ScopeKind::Worker,
    ));
    cases.push(("scoped", scoped, false));
    let mut wrong = binding("operator");
    wrong.1 = RouteTarget::ManagementSurface {
        module_id: "other".into(),
    };
    cases.push(("wrong module", wrong, false));
    let mut service = binding("operator");
    service.1 = RouteTarget::InternalService {
        module_id: crate::MODULE_ID.into(),
        service_id: "other".into(),
    };
    cases.push(("wrong surface", service, false));
    for (label, b, allowed) in cases {
        for method in METHODS.into_iter().chain([SEND]) {
            let mut h = Harness::new();
            h.app
                .display
                .ingest("s", DisplayInput::TurnEnd { run_id: None });
            h.app.display.ingest("s", DisplayInput::Reset);
            h.bind(7, 1, b.clone()).await;
            h.request(method, 1, params(method, "s")).await;
            let reply = h.next().await;
            if allowed {
                assert_eq!(
                    reply.header.ty,
                    if method.ends_with("watch") {
                        FrameType::StreamData
                    } else {
                        FrameType::Response
                    },
                    "{label} {method}: {}",
                    body(&reply)
                );
                if method.starts_with("session.transcript") || method == DETAILS {
                    assert!(h.resolver.source.reads.load(Ordering::SeqCst) > 0);
                }
                if method == SEND {
                    assert_eq!(body(&reply)["result"]["ok"], true, "{label}");
                    assert_eq!(h.sends.calls.load(Ordering::SeqCst), 3, "{label}");
                }
            } else {
                assert_eq!(reply.header.ty, FrameType::Error, "{label} {method}");
                assert_eq!(
                    body(&reply)["code"],
                    "carrier_op_not_permitted",
                    "{label} {method}"
                );
                assert_eq!(h.resolver.resolutions.load(Ordering::SeqCst), 0);
                assert_eq!(h.resolver.source.reads.load(Ordering::SeqCst), 0);
                // A refused send reaches neither the session lookup nor a runner.
                assert_eq!(h.sends.opened.load(Ordering::SeqCst), 0, "{label}");
                assert_eq!(h.sends.calls.load(Ordering::SeqCst), 0, "{label}");
            }
            h.finish().await;
        }
    }
}

#[tokio::test]
async fn refused_callers_cannot_probe_session_existence_or_request_validity() {
    for method in METHODS.into_iter().chain([SEND]) {
        let mut h = Harness::new();
        let mut b = binding("fed-phone:spoofed");
        b.0 = Some(Principal::Unverified);
        h.bind(7, 1, b).await;
        let mut refusals = vec![];
        for params in [
            params(method, "s"),
            params(method, "missing"),
            json!({"session":42,"principal":"callosum"}),
            Value::Null,
        ] {
            h.request(method, 1, params).await;
            let reply = h.next().await;
            assert_eq!(reply.header.ty, FrameType::Error);
            refusals.push(body(&reply));
        }
        assert!(refusals.iter().all(|reply| reply == &refusals[0]));
        assert_eq!(refusals[0]["code"], "carrier_op_not_permitted");
        assert_eq!(h.resolver.resolutions.load(Ordering::SeqCst), 0);
        assert_eq!(h.resolver.source.reads.load(Ordering::SeqCst), 0);
        assert_eq!(h.app.watches.open_count(), 0);
        assert_eq!(h.sends.opened.load(Ordering::SeqCst), 0);
        assert_eq!(h.sends.calls.load(Ordering::SeqCst), 0);
        h.finish().await;
    }
}

#[test]
fn a_development_direct_caller_is_a_device_only_on_this_modules_surface() {
    let mut direct = binding("anything");
    direct.0 = Some(Principal::Direct);
    // Uploads accept it: its fingerprint is the shared development one.
    assert_eq!(
        crate::app_reader::admit(Some(&direct), "upload.begin").unwrap(),
        crate::app_reader::Caller::Device {
            class: "direct".into(),
            fingerprint: crate::app_reader::DEVELOPMENT_DIRECT_FINGERPRINT.into(),
        }
    );
    let mut elsewhere = direct.clone();
    elsewhere.1 = RouteTarget::ManagementSurface {
        module_id: "other".into(),
    };
    assert_eq!(
        crate::app_reader::admit(Some(&elsewhere), "upload.begin")
            .unwrap_err()
            .code,
        "carrier_op_not_permitted"
    );
}

#[test]
fn the_send_is_advertised_as_a_mutation_beside_the_reads() {
    let manifest = serde_json::to_value(crate::manifest::manifest().unwrap()).unwrap();
    let surface = manifest["provides"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["role"] == "management_surface")
        .unwrap();
    let send = surface["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["name"] == SEND)
        .expect("the send is advertised");
    assert_eq!(send["kind"], "mutate");
    assert!(manifest["operation_delivery"][SEND].is_null());
}

/// A worker steer no run takes expires, and the app's transcript watch on that
/// session carries the late outcome as its own notice.
#[tokio::test(start_paused = true)]
async fn an_expired_worker_steer_is_reported_on_the_sessions_transcript_watch() {
    let mut h = Harness::new();
    h.sends.worker.store(true, Ordering::SeqCst);
    h.bind(7, 1, binding("fed-phone:fp")).await;
    h.request(TRANSCRIPT_WATCH, 1, params(TRANSCRIPT_WATCH, "s"))
        .await;
    assert_eq!(h.next().await.header.ty, FrameType::StreamData);
    h.request(SEND, 2, params(SEND, "s")).await;
    let receipt = h.next().await;
    assert_eq!(receipt.header.ty, FrameType::Response);
    assert_eq!(body(&receipt)["result"]["ok"], true);
    h.barrier().await;
    h.sends.expired.store(true, Ordering::SeqCst);
    // Wake the follow of the fake's expired steer without paying its real interval.
    tokio::time::advance(Duration::from_secs(60)).await;
    let mut notice = None;
    for _ in 0..4 {
        let frame = h.next_within(15).await;
        if frame.header.corr == 1 && body(&frame).get("sendOutcome").is_some() {
            notice = Some(body(&frame));
            break;
        }
    }
    let notice = notice.expect("the late outcome reaches the watch");
    assert_eq!(notice["sendOutcome"]["code"], "steer_expired");
    assert_eq!(notice["sendOutcome"]["clientMessageId"], "m1");
    assert_eq!(notice["sendOutcome"]["session"], "s");
    assert_eq!(notice["sendOutcome"]["terminal"], true);
    h.finish().await;
}

#[tokio::test]
async fn the_thirty_third_watch_is_refused_and_slots_free_when_watches_end() {
    let mut h = Harness::new();
    h.bind(7, 1, binding("fed-phone:fp")).await;
    h.bind(8, 1, binding("fed-phone:fp")).await;
    // One route holds at most 24, so the daemon always has requests in flight
    // left for its reads and sends.
    for corr in 1..=24 {
        h.request(DISPLAY_WATCH, corr, params(DISPLAY_WATCH, "s"))
            .await;
    }
    h.barrier().await;
    h.request(DISPLAY_WATCH, 25, params(DISPLAY_WATCH, "s"))
        .await;
    let refusal = h.next().await;
    assert_eq!(refusal.header.ty, FrameType::Error);
    assert_eq!(refusal.header.corr, 25);
    assert_eq!(body(&refusal)["code"], "watch_capacity");
    for corr in 26..=33 {
        h.request_on(8, 1, DISPLAY_WATCH, corr, params(DISPLAY_WATCH, "s"))
            .await;
    }
    h.barrier().await;
    assert_eq!(h.app.watches.open_count(), 32);
    h.request_on(8, 1, DISPLAY_WATCH, 34, params(DISPLAY_WATCH, "s"))
        .await;
    let refusal = h.next().await;
    assert_eq!(refusal.header.ty, FrameType::Error);
    assert_eq!(refusal.header.corr, 34);
    assert_eq!(body(&refusal)["code"], "watch_capacity");
    h.cancel(7, 1, 1);
    let ended = h.next().await;
    assert_eq!(ended.header.ty, FrameType::StreamEnd);
    assert_eq!(ended.header.corr, 1);
    h.request(DISPLAY_WATCH, 35, params(DISPLAY_WATCH, "s"))
        .await;
    h.barrier().await;
    assert_eq!(h.app.watches.open_count(), 32);
    h.finish().await;
}

async fn open_both(h: &mut Harness) {
    h.bind(7, 1, binding("fed-sim:fp")).await;
    h.request(TRANSCRIPT_WATCH, 1, params(TRANSCRIPT_WATCH, "s"))
        .await;
    assert_eq!(h.next().await.header.ty, FrameType::StreamData);
    h.request(DISPLAY_WATCH, 2, params(DISPLAY_WATCH, "s"))
        .await;
    h.barrier().await;
    assert_eq!(h.app.watches.open_count(), 2);
}

async fn ends(h: &mut Harness) {
    let mut corrs = vec![];
    for _ in 0..2 {
        let reply = h.next().await;
        assert_eq!(reply.header.ty, FrameType::StreamEnd);
        assert!(reply.body.is_empty());
        corrs.push(reply.header.corr);
    }
    corrs.sort();
    assert_eq!(corrs, [1, 2]);
}

#[tokio::test]
async fn cancel_ends_only_the_matching_watch() {
    let mut h = Harness::new();
    open_both(&mut h).await;
    h.cancel(7, 9, 1);
    h.barrier().await;
    assert_eq!(h.app.watches.open_count(), 2);
    for corr in [1, 2] {
        h.cancel(7, 1, corr);
        let ended = h.next().await;
        assert_eq!(ended.header.ty, FrameType::StreamEnd);
        assert_eq!(ended.header.corr, corr);
    }
    h.finish().await;
}

/// A route going ends its watches, and only its own.
#[tokio::test]
async fn a_gone_route_ends_its_watches() {
    let mut h = Harness::new();
    open_both(&mut h).await;
    h.bind(8, 1, binding("fed-sim:fp")).await;
    h.request_on(8, 1, DISPLAY_WATCH, 3, params(DISPLAY_WATCH, "s"))
        .await;
    h.barrier().await;
    assert_eq!(h.app.watches.open_count(), 3);
    h.connection.end_route(7, 2);
    h.barrier().await;
    assert_eq!(h.app.watches.open_count(), 3);
    h.connection.end_route(7, 1);
    ends(&mut h).await;
    h.barrier().await;
    assert_eq!(h.app.watches.open_count(), 1);
    h.finish().await;
}

#[tokio::test]
async fn module_shutdown_ends_every_open_watch_with_stream_end() {
    let mut h = Harness::new();
    open_both(&mut h).await;
    h.app.shutdown();
    ends(&mut h).await;
    h.connection.join().await;
    assert_eq!(h.app.watches.open_count(), 0);
    h.request(DISPLAY_WATCH, 3, params(DISPLAY_WATCH, "s"))
        .await;
    assert_eq!(body(&h.next().await)["code"], "module_stopping");
}

#[tokio::test]
async fn cancel_interrupts_a_stalled_transcript_source_read() {
    let mut h = Harness::new();
    h.resolver.source.stalled.store(true, Ordering::SeqCst);
    h.bind(7, 1, binding("operator")).await;
    h.request(TRANSCRIPT_WATCH, 1, params(TRANSCRIPT_WATCH, "s"))
        .await;
    h.barrier().await;
    h.cancel(7, 1, 1);
    assert_eq!(h.next().await.header.ty, FrameType::StreamEnd);
    h.finish().await;
}

#[tokio::test]
async fn display_watchers_share_one_live_projection_and_reads_do_not_create_it() {
    let mut h = Harness::new();
    h.bind(7, 1, binding("operator")).await;
    h.request(DISPLAY_READ, 9, params(DISPLAY_READ, "s")).await;
    assert_eq!(
        body(&h.next().await),
        json!({"result":{"absent":"no_ring_attached"}})
    );
    assert!(h.app.display.projection("s").is_none());
    for corr in [1, 2] {
        h.request(DISPLAY_WATCH, corr, params(DISPLAY_WATCH, "s"))
            .await;
    }
    h.barrier().await;
    assert!(h.app.display.projection("s").is_none());
    let a = h.app.display.attach("s");
    let b = h.app.display.attach("s");
    assert!(Arc::ptr_eq(&a, &b));
    h.app.display.ingest(
        "s",
        DisplayInput::Delta {
            mid: "m1".into(),
            text: "hello".into(),
            part_type: DisplayPartType::Text,
        },
    );
    let first = h.next().await;
    let second = h.next().await;
    assert_eq!(first.header.ty, FrameType::StreamData);
    assert_ne!(first.header.corr, second.header.corr);
    assert_eq!(body(&first), body(&second));
    assert_eq!(body(&first)["frame"]["chunk"], "hello");
    h.request(DISPLAY_READ, 9, params(DISPLAY_READ, "s")).await;
    assert_eq!(body(&h.next().await)["result"], body(&first));
    h.finish().await;
}

#[tokio::test(start_paused = true)]
async fn display_hub_reset_discards_pending_text_and_its_stale_timer() {
    let hub = Arc::new(display_hub::DisplayHub::default());
    let mut changes = hub.changes("s");
    let projection = hub.attach("s");
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    hub.ingest(
        "s",
        DisplayInput::Delta {
            mid: "m1".into(),
            text: "old".into(),
            part_type: DisplayPartType::Text,
        },
    );
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(100)).await;
    hub.ingest("s", DisplayInput::Reset);
    hub.ingest(
        "s",
        DisplayInput::Delta {
            mid: "m1".into(),
            text: "new".into(),
            part_type: DisplayPartType::Text,
        },
    );
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(200)).await;
    tokio::task::yield_now().await;
    let before_new_timer = serde_json::to_value(projection.display_read(None)).unwrap();
    assert_eq!(before_new_timer["frame"]["kind"], "display_gap");
    tokio::time::advance(std::time::Duration::from_millis(100)).await;
    tokio::task::yield_now().await;
    let after_new_timer = serde_json::to_value(projection.display_read(None)).unwrap();
    assert_eq!(after_new_timer["frame"]["chunk"], "new");
    hub.remove("s");
    assert!(hub.projection("s").is_none());
    assert!(changes.has_changed().unwrap());
}

/// A resolver that knows no session: every lookup is refused `session_unknown`, as for
/// a session no runner this reader serves hosts.
struct Unknown;

#[async_trait]
impl SourceResolver for Unknown {
    async fn resolve(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Option<Arc<dyn TranscriptSource>>, ReadError> {
        Err(
            ReadError::new("session_unknown", "no runner this reader serves hosts it")
                .with_detail(json!({"runner": null})),
        )
    }
}

/// A watch on a session the reader refuses ends at once with that refusal, both kinds,
/// and the opening is logged like a read, with its outcome.
#[tokio::test]
async fn a_watch_on_a_refused_session_ends_at_once_and_is_logged() {
    let app = Arc::new(AppReader::new(Arc::new(Unknown)));
    for method in [DISPLAY_WATCH, TRANSCRIPT_WATCH] {
        let (log, _capture) = super::observe::tests::capture();
        let mut request = params(method, "ses_gone");
        request["harness"] = json!("opencode2");
        let frame = serve_frame(&app, method, &request, 1, Duration::from_secs(5)).await;
        assert_eq!(
            frame.header.ty,
            FrameType::Error,
            "{method}: {}",
            String::from_utf8_lossy(&frame.body)
        );
        let error = String::from_utf8_lossy(&frame.body).into_owned();
        assert!(error.contains("session_unknown"), "{method}: {error}");
        let text = log.text();
        assert!(
            text.contains(&format!("method={method}"))
                && text.contains("outcome=\"session_unknown\"")
                && text.contains("level=WARN"),
            "{method}: {text}"
        );
    }
}
