use super::*;
use crate::app_reader::{self, send, send_store::Receipts, tests::Connection, AppReader};
use subc_protocol::{BindIdentity, Flags, Frame, FrameType, Principal, Priority};

const HELD: &str =
    include_str!("../../../../tests/fixtures/entorhinal-agent-sessions-v1/sessions-held-on.json");
const OPEN: &str =
    include_str!("../../../../tests/fixtures/entorhinal-agent-sessions-v1/sessions-open.json");

/// Wall-clock milliseconds for the send receipts these tests record and look up.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn sample(held: bool) -> Value {
    let records: Value = serde_json::from_str(if held { HELD } else { OPEN }).unwrap();
    records[0]["reply"]["result"]["sessions"][usize::from(held)].clone()
}

struct PlacementLookup {
    binding: Value,
    legacy: bool,
}

#[async_trait]
impl Entorhinal for PlacementLookup {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        match method {
            "agent.session_for_runner" => {
                assert_eq!(params["session"], self.binding["session"]);
                assert_eq!(params["runner"], self.binding["runner"]);
                if self.legacy {
                    Ok(json!({"code":"not_found", "message":"not registered"}))
                } else {
                    let mut binding = self.binding.clone();
                    binding["agent_id"] = json!("agent_0000000000000101");
                    binding["incarnation"] = json!("entorhinal-test");
                    binding["network_history"] = json!([]);
                    Ok(json!({"result":binding}))
                }
            }
            "agent.for_session" if self.legacy => {
                assert_eq!(params["session_id"], self.binding["session"]);
                Ok(json!({"result":{
                    "agent_id":"agent_0000000000000101",
                    "ref_kind":self.binding["ref_kind"], "ref":self.binding["ref"],
                }}))
            }
            "agent.session_for_ref" if self.legacy => {
                assert_eq!(params["ref"], self.binding["ref"]);
                let mut binding = self.binding.clone();
                binding["agent_id"] = json!("agent_0000000000000101");
                binding["incarnation"] = json!("entorhinal-test");
                binding["session_id"] = binding["session"].clone();
                Ok(json!({"result":binding}))
            }
            _ => panic!("unexpected lookup {method}"),
        }
    }
}

struct PlacementRoutes {
    route: Arc<FakeRoute>,
    opened: AtomicUsize,
    senders: AtomicUsize,
    sent: Arc<AtomicUsize>,
}

struct PlacementSender(SessionRef, Arc<AtomicUsize>);

#[async_trait]
impl SendTarget for PlacementSender {
    fn reference(&self) -> &SessionRef {
        &self.0
    }

    async fn call(&self, method: &'static str, _: Value) -> Result<Value, RunnerError> {
        if method == "session.head" {
            return Ok(json!({
                "sender": {"modes": ["idle_start", "steer"], "interrupt": true},
                "last_run_state": {"run_id": "r1", "state": "active"},
            }));
        }
        assert_eq!(method, "session.send");
        self.1.fetch_add(1, Ordering::Relaxed);
        Ok(json!({"state":"active", "run_id":"r1"}))
    }
}

#[async_trait]
impl BrocaRoutes for PlacementRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        self.opened.fetch_add(1, Ordering::Relaxed);
        Ok(self.route.clone())
    }

    fn sender(&self, resolved: &Resolved) -> Option<Arc<dyn SendTarget>> {
        self.senders.fetch_add(1, Ordering::Relaxed);
        Some(Arc::new(PlacementSender(
            resolved.reference.clone(),
            self.sent.clone(),
        )))
    }
}

fn serving_placement(
    binding: Value,
    legacy: bool,
) -> (Arc<LazyBrocaSources>, Arc<PlacementRoutes>) {
    let routes = Arc::new(PlacementRoutes {
        route: FakeRoute::new(),
        opened: AtomicUsize::new(0),
        senders: AtomicUsize::new(0),
        sent: Arc::new(AtomicUsize::new(0)),
    });
    let sources = Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(PlacementLookup { binding, legacy }))),
        routes.clone(),
    ));
    (
        Arc::new(LazyBrocaSources::new(Arc::new(Ready(sources)))),
        routes,
    )
}

struct Ready(Arc<BrocaSources>);

#[async_trait]
impl SourceConnector for Ready {
    async fn connect(&self) -> Result<Arc<BrocaSources>, ReadError> {
        Ok(self.0.clone())
    }
}

async fn operation(app: Arc<AppReader>, method: &str, session: &str) -> Frame {
    let mut params = json!({"session":session});
    if method == app_reader::SEND {
        params["text"] = json!("hello");
        params["clientMessageId"] = json!("phone-1");
    } else if method == app_reader::TRANSCRIPT_WATCH {
        params["claim"] =
            json!({"heldFloorMessageId":"m20", "sinceMessageId":"m22", "heldVersion":"history:7"});
    }
    let request = Frame::build(
        FrameType::Request,
        Flags::new(false, Priority::Interactive, false),
        1,
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
        .dispatch(request, Some(binding), method.into(), params, None, &tx)
        .await
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    connection.end_route(1, 1);
    connection.join().await;
    assert_eq!(app.watches.open_count(), 0);
    reply
}

fn assert_held(reply: &Frame) {
    assert_eq!(reply.header.ty, FrameType::Error);
    let records: Value = serde_json::from_str(HELD).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&reply.body).unwrap(),
        records[1]["reply"],
    );
}

fn assert_unopened(routes: &PlacementRoutes) {
    assert_eq!(routes.opened.load(Ordering::Relaxed), 0);
    assert_eq!(routes.senders.load(Ordering::Relaxed), 0);
    assert_eq!(routes.sent.load(Ordering::Relaxed), 0);
    assert_eq!(routes.route.attaches.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn held_session_refuses_reads_and_watches_before_opening_routes() {
    for legacy in [false, true] {
        let binding = sample(true);
        let session = binding["session"].as_str().unwrap().to_owned();
        let (sources, routes) = serving_placement(binding, legacy);
        let app = Arc::new(AppReader::new(sources));
        for method in [
            app_reader::TRANSCRIPT_PAGE,
            app_reader::DISPLAY_READ,
            app_reader::TRANSCRIPT_WATCH,
            app_reader::DISPLAY_WATCH,
        ] {
            assert_held(&operation(app.clone(), method, &session).await);
            assert_unopened(&routes);
        }
    }
}

#[tokio::test]
async fn held_session_refuses_send_before_opening_routes_or_recording_receipts() {
    for legacy in [false, true] {
        let binding = sample(true);
        let session = binding["session"].as_str().unwrap().to_owned();
        let (sources, routes) = serving_placement(binding, legacy);
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::shared(&dir.path().join("store.db"));
        let receipts = Receipts::open(store).unwrap();
        let sender = Arc::new(send::Sender::new(sources.clone(), Some(receipts.clone())));
        let app = Arc::new(AppReader::with_sender(sources, sender));
        assert_held(&operation(app.clone(), app_reader::SEND, &session).await);
        assert_unopened(&routes);
        assert!(receipts
            .get(&session, "phone-1", now_ms())
            .unwrap()
            .is_none());
        assert!(receipts.followed(10).unwrap().is_empty());

        // A cached send receipt must not bypass the current refusal for a session held on another machine.
        receipts
            .record(
                &session,
                "phone-1",
                "digest",
                "queue",
                "send-1",
                &json!({"ok":true}),
                None,
                now_ms(),
            )
            .unwrap();
        assert_held(&operation(app, app_reader::SEND, &session).await);
        assert_unopened(&routes);
    }
}

#[tokio::test]
async fn open_session_without_held_on_keeps_reads_watches_and_send_working() {
    for legacy in [false, true] {
        let binding = sample(false);
        assert!(binding.get("heldOn").is_none());
        let session = binding["session"].as_str().unwrap().to_owned();
        let (sources, routes) = serving_placement(binding, legacy);
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::shared(&dir.path().join("store.db"));
        let receipts = Receipts::open(store).unwrap();
        let sender = Arc::new(send::Sender::new(sources.clone(), Some(receipts.clone())));
        let app = Arc::new(AppReader::with_sender(sources, sender));
        for method in [
            app_reader::TRANSCRIPT_PAGE,
            app_reader::DISPLAY_READ,
            app_reader::SEND,
        ] {
            let reply = operation(app.clone(), method, &session).await;
            assert_eq!(
                reply.header.ty,
                FrameType::Response,
                "{method}: {:?}",
                String::from_utf8_lossy(&reply.body)
            );
        }
        assert_eq!(routes.sent.load(Ordering::Relaxed), 1);
        assert!(receipts
            .get(&session, "phone-1", now_ms())
            .unwrap()
            .is_some());
        for method in [app_reader::TRANSCRIPT_WATCH, app_reader::DISPLAY_WATCH] {
            let reply = operation(app.clone(), method, &session).await;
            assert_eq!(
                reply.header.ty,
                FrameType::StreamData,
                "{method}: {:?}",
                String::from_utf8_lossy(&reply.body)
            );
        }
        assert!(routes.opened.load(Ordering::Relaxed) > 0);
        assert!(routes.route.attaches.load(Ordering::Relaxed) > 0);
    }
}

#[tokio::test]
async fn malformed_held_on_is_an_invalid_reply_in_both_lookup_paths() {
    for legacy in [false, true] {
        for held_on in [
            json!(false),
            json!(7),
            json!([]),
            json!({"machine_id":"source-mac"}),
            json!(""),
            json!("   "),
        ] {
            let mut binding = sample(true);
            binding["heldOn"] = held_on;
            let reference = SessionRef {
                runner: "broca".into(),
                session: binding["session"].as_str().unwrap().into(),
            };
            let resolver = Resolver::new(Arc::new(PlacementLookup {
                binding: binding.clone(),
                legacy,
            }));
            assert_eq!(
                resolver.resolve(&reference).await.unwrap_err().code,
                "invalid_provider_reply"
            );
            let (sources, routes) = serving_placement(binding, legacy);
            let app = Arc::new(AppReader::with_sender(
                sources.clone(),
                Arc::new(send::Sender::new(sources, None)),
            ));
            for method in [
                app_reader::TRANSCRIPT_PAGE,
                app_reader::DISPLAY_READ,
                app_reader::TRANSCRIPT_WATCH,
                app_reader::DISPLAY_WATCH,
                app_reader::SEND,
            ] {
                let reply = operation(app.clone(), method, &reference.session).await;
                assert_eq!(reply.header.ty, FrameType::Error);
                assert_eq!(
                    serde_json::from_slice::<Value>(&reply.body).unwrap()["code"],
                    "invalid_provider_reply",
                    "{method}",
                );
                assert_unopened(&routes);
            }
        }
        let mut binding = sample(false);
        binding["heldOn"] = Value::Null;
        let reference = SessionRef {
            runner: "broca".into(),
            session: binding["session"].as_str().unwrap().into(),
        };
        let resolver = Resolver::new(Arc::new(PlacementLookup { binding, legacy }));
        assert_eq!(
            resolver.resolve(&reference).await.unwrap().reference,
            reference
        );
    }
}
