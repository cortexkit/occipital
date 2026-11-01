use super::*;
use cortexkit_role_llm_runner::display::{DisplayEntry, DisplayRow};

struct StrictRoute {
    entries: Vec<Value>,
    calls: Mutex<Vec<Value>>,
    describe_started: tokio::sync::Notify,
    describe_release: tokio::sync::Notify,
    hold_describe: AtomicBool,
    read_delay_ms: AtomicU64,
    events: Mutex<Vec<mpsc::Sender<Result<Value, ReadError>>>>,
}

impl StrictRoute {
    fn new(count: u64) -> Arc<Self> {
        let output = "checked source line with compiler diagnostics\n".repeat(12_000);
        let entries = (0..count)
            .map(|ordinal| {
                let call = format!("call-{}", ordinal / 3);
                let message = match ordinal % 3 {
                    0 => json!({"role":"user", "content":[{"type":"text", "text":"Please inspect the worktree and fix the failing test."}]}),
                    1 => json!({"role":"assistant", "content":[
                        {"type":"reasoning", "text":"Read the compiler output before choosing the next edit."},
                        {"type":"tool_call", "tool_call_id":call, "tool_name":"bash", "input":{"command":"cargo test"}}]}),
                    _ => json!({"role":"tool", "content":[{"type":"tool_result", "tool_call_id":call, "output":output}]}),
                };
                json!({"mid":format!("m{ordinal}"), "ordinal":ordinal,
                    "written_at":1_780_000_000_000_u64 + ordinal * 9_000, "message":message})
            })
            .collect();
        Arc::new(Self {
            entries,
            calls: Mutex::new(Vec::new()),
            describe_started: tokio::sync::Notify::new(),
            describe_release: tokio::sync::Notify::new(),
            hold_describe: AtomicBool::new(false),
            read_delay_ms: AtomicU64::new(0),
            events: Mutex::new(Vec::new()),
        })
    }

    fn locate(&self, params: &Value, key: &str, mid: bool) -> Result<Option<usize>, ReadError> {
        let Some(value) = params.get(key) else {
            return Ok(None);
        };
        self.entries
            .iter()
            .position(|entry| entry[if mid { "mid" } else { "ordinal" }] == *value)
            .map(Some)
            .ok_or_else(|| {
                ReadError::new(
                    if mid {
                        "unknown_mid"
                    } else {
                        "unknown_ordinal"
                    },
                    "anchor not assigned",
                )
            })
    }

    fn page(&self, op: ReadOp, params: &Value) -> Result<Value, ReadError> {
        let anchors = [
            "cursor",
            "from_mid",
            "before_mid",
            "from_ordinal",
            "before_ordinal",
        ];
        if anchors
            .iter()
            .filter(|key| params.get(**key).is_some())
            .count()
            > 1
        {
            return Err(ReadError::new("invalid_params", "exclusive anchors"));
        }
        if params.get("cursor").is_some() {
            return Err(ReadError::new("cursor_expired", "cursor not assigned"));
        }
        let limit = params["limit"].as_u64().unwrap_or(100) as usize;
        let from = self.locate(params, "from_mid", true)?.or(self.locate(
            params,
            "from_ordinal",
            false,
        )?);
        let before = self.locate(params, "before_mid", true)?.or(self.locate(
            params,
            "before_ordinal",
            false,
        )?);
        let (start, end) = if let Some(from) = from {
            (from, (from + limit).min(self.entries.len()))
        } else {
            let end = before.unwrap_or(self.entries.len());
            (end.saturating_sub(limit), end)
        };
        let mut selected = Vec::new();
        let mut bytes = 0;
        let max = params["max_bytes"].as_u64().unwrap_or(2_097_152) as usize;
        for entry in &self.entries[start..end] {
            let size = serde_json::to_vec(entry).unwrap().len();
            if !selected.is_empty() && bytes + size > max {
                break;
            }
            bytes += size;
            selected.push(entry.clone());
        }
        let next = start + selected.len();
        let messages = if params["form"] == "display" {
            selected.iter().map(|entry| json!({"mid":entry["mid"], "ordinal":entry["ordinal"],
                "written_at":entry["written_at"], "row":{"role":entry["message"]["role"], "text":"", "parts":[]}})).collect::<Vec<_>>()
        } else {
            selected
        };
        let mut page = json!({"messages":messages, "lineage_id":"cut-a", "history_version":7});
        if matches!(op, ReadOp::Read) {
            page["next_from_ordinal"] = json!((next < self.entries.len()).then_some(next));
        } else {
            page["older"] = json!((start > 0).then_some("older"));
            if next < self.entries.len() {
                page["newer"] = json!("newer");
            }
        }
        Ok(page)
    }
}

#[test]
fn strict_route_matches_broca_cursor_and_end_semantics() {
    let route = StrictRoute::new(151);
    let tail = route
        .page(
            ReadOp::Read,
            &json!({"from_ordinal":145,"limit":6,"max_bytes":2_097_152}),
        )
        .unwrap();
    assert_eq!(tail["messages"].as_array().unwrap().len(), 6);
    assert!(tail["next_from_ordinal"].is_null());
    let trimmed = route
        .page(
            ReadOp::Read,
            &json!({"from_ordinal":51,"limit":100,"max_bytes":2_097_152}),
        )
        .unwrap();
    assert!(trimmed["messages"].as_array().unwrap().len() < 100);
    assert!(trimmed["next_from_ordinal"].as_u64().unwrap() > 51);
    let history = route
        .page(ReadOp::History, &json!({"from_mid":"m145","limit":6}))
        .unwrap();
    assert!(history.get("newer").is_none());
    let display = route
        .page(
            ReadOp::History,
            &json!({"form":"display","from_ordinal":145,"limit":6}),
        )
        .unwrap();
    let display: DisplayPage = serde_json::from_value(display).unwrap();
    assert_eq!(display.messages.len(), 6);
    let older = route
        .page(ReadOp::History, &json!({"before_ordinal":145,"limit":6}))
        .unwrap();
    assert_eq!(older["messages"][0]["ordinal"], 139);
    assert_eq!(older["messages"][5]["ordinal"], 144);
    for (params, code) in [
        (json!({"from_mid":"m151"}), "unknown_mid"),
        (json!({"before_mid":"missing"}), "unknown_mid"),
        (json!({"from_ordinal":151}), "unknown_ordinal"),
        (json!({"before_ordinal":151}), "unknown_ordinal"),
        (
            json!({"from_ordinal":145,"before_mid":"m150"}),
            "invalid_params",
        ),
    ] {
        assert_eq!(route.page(ReadOp::History, &params).unwrap_err().code, code);
    }
}

#[async_trait]
impl BrocaRoute for StrictRoute {
    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        self.calls
            .lock()
            .unwrap()
            .push(json!({"method":op.method(), "params":params}));
        tokio::task::yield_now().await;
        let delay = self.read_delay_ms.load(Ordering::Acquire);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        match op {
            ReadOp::Head => Ok(
                json!({"first_ordinal":0, "last_ordinal":self.entries.len() - 1,
                "history_version":7, "lineage_id":"cut-a", "updated_at":1_780_001_350_000_u64,
                "last_run_state":{"run_id":"run-1", "state":"finished"}}),
            ),
            ReadOp::Describe => {
                if self.hold_describe.load(Ordering::Acquire) {
                    self.describe_started.notify_one();
                    self.describe_release.notified().await;
                }
                Ok(describe_with(&["history_display"]))
            }
            ReadOp::Read | ReadOp::History => self.page(op, &params),
            _ => panic!("unexpected read: {op:?}"),
        }
    }

    async fn subscribe(&self) -> Result<Box<dyn EventStream>, ReadError> {
        self.calls
            .lock()
            .unwrap()
            .push(json!({"method":"session.subscribe", "params":{"from":"live"}}));
        let (sender, receiver) = mpsc::channel(16);
        self.events.lock().unwrap().push(sender);
        Ok(Box::new(FakeEvents {
            receiver,
            detaches: Arc::new(AtomicUsize::new(0)),
        }))
    }
}

struct StrictRoutes(Arc<StrictRoute>);

#[async_trait]
impl BrocaRoutes for StrictRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

struct ReplayRoute {
    head: Value,
    describe: Value,
    raw: Value,
    display: Value,
}

#[async_trait]
impl BrocaRoute for ReplayRoute {
    async fn read(&self, op: ReadOp, _: Value) -> Result<Value, ReadError> {
        result(match op {
            ReadOp::Head => self.head.clone(),
            ReadOp::Describe => self.describe.clone(),
            ReadOp::Read => self.raw.clone(),
            ReadOp::History => self.display.clone(),
            _ => panic!("unexpected replay op"),
        })
    }
    async fn subscribe(&self) -> Result<Box<dyn EventStream>, ReadError> {
        panic!("plain page subscribed")
    }
}

struct ReplayRoutes(Arc<ReplayRoute>);

#[async_trait]
impl BrocaRoutes for ReplayRoutes {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn private_replay_completes_through_the_phone_handler() {
    let Some(root) = std::env::var_os("THALAMUS_BROCA_REPLAY_DIR") else {
        eprintln!("SKIP private_replay_completes_through_the_phone_handler: THALAMUS_BROCA_REPLAY_DIR is unset");
        return;
    };
    let root = std::path::PathBuf::from(root);
    let load = |file: &str| -> Value {
        serde_json::from_slice(&std::fs::read(root.join(file)).unwrap()).unwrap()
    };
    for (case, limit) in [("A", 6), ("B", 100)] {
        let route = Arc::new(ReplayRoute {
            head: load("A/01-session.head.json"),
            describe: load("A/02-role.describe.json"),
            raw: load(&format!("{case}/04-session.read.json")),
            display: load(&format!("{case}/05-session.history.json")),
        });
        let sources = Arc::new(BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(Lookup))),
            Arc::new(ReplayRoutes(route)),
        ));
        let app = Arc::new(crate::app_reader::AppReader::new(sources));
        let params = json!({"session":"alfonso:s1", "harness":"broca", "limit":limit});
        let bytes = tokio::time::timeout(
            Duration::from_secs(4),
            crate::app_reader::tests::serve(&app, crate::app_reader::TRANSCRIPT_PAGE, &params),
        )
        .await
        .expect("private replay timed out");
        let reply: Value = serde_json::from_slice(&bytes).unwrap();
        let messages = reply["result"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), limit as usize);
        assert_eq!(messages[0]["ordinal"], 151 - limit);
        assert_eq!(messages.last().unwrap()["ordinal"], 150);
        eprintln!("private replay {case}: {limit} rows served");
    }
}

#[tokio::test]
async fn large_tool_pages_complete_through_the_phone_handler() {
    for (count, limit) in [(151, 6), (151, 100), (8, 100)] {
        let route = StrictRoute::new(count);
        let sources = Arc::new(BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(Lookup))),
            Arc::new(StrictRoutes(route.clone())),
        ));
        let app = Arc::new(crate::app_reader::AppReader::new(sources));
        let params = json!({"session":"alfonso:s1", "harness":"broca", "limit":limit});
        let bytes = tokio::time::timeout(
            Duration::from_secs(4),
            crate::app_reader::tests::serve(&app, crate::app_reader::TRANSCRIPT_PAGE, &params),
        )
        .await
        .unwrap_or_else(|_| {
            panic!(
                "phone page timed out: count={count}, limit={limit}, calls={:?}",
                route.calls.lock().unwrap()
            )
        });
        let reply: Value = serde_json::from_slice(&bytes).unwrap();
        let messages = reply["result"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), count.min(limit) as usize, "{reply}");
        assert_eq!(messages[0]["ordinal"], count.saturating_sub(limit));
        assert_eq!(messages.last().unwrap()["ordinal"], count - 1);
        assert!(
            route.events.lock().unwrap().is_empty(),
            "a plain page must not subscribe"
        );
    }
}

#[tokio::test]
async fn reader_log_counts_every_remote_call_and_the_reply_encoding() {
    let (log, _guard) = crate::app_reader::observe::tests::capture();
    let route = StrictRoute::new(151);
    let sources = Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(Lookup))),
        Arc::new(StrictRoutes(route)),
    ));
    let app = Arc::new(crate::app_reader::AppReader::new(sources));
    crate::app_reader::tests::serve(
        &app,
        crate::app_reader::TRANSCRIPT_PAGE,
        &json!({"session":"alfonso:s1", "harness":"broca", "limit":6}),
    )
    .await;
    let text = log.text();
    assert_eq!(
        text.matches("app reader request finished").count(),
        1,
        "{text}"
    );
    for count in [
        "\"broca.session.head\": 4",
        "\"broca.session.read\": 1",
        "\"broca.session.history\": 1",
        "\"broca.role.describe\": 1",
        "\"entorhinal.agent.session_for_runner\": 4",
    ] {
        assert!(text.contains(count), "missing {count}: {text}");
    }
    for step in [
        "resolution",
        "broca.session.head",
        "broca.role.describe",
        "broca.session.read",
        "broca.session.history",
        "projection",
        "reply_encode",
    ] {
        assert!(
            text.contains(&format!("(\"{step}\",")),
            "missing step {step}: {text}"
        );
    }
    assert!(text.contains("outcome=\"ok\""), "{text}");
}

#[tokio::test(start_paused = true)]
async fn concurrent_page_and_watch_finish_after_a_slow_describe() {
    // These routes are in-memory futures. CPU spent projecting large fake rows
    // must not consume the budget that detects an unreleased notification gate.
    let route = StrictRoute::new(151);
    route.hold_describe.store(true, Ordering::Release);
    let sources = Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(Lookup))),
        Arc::new(StrictRoutes(route.clone())),
    ));
    let app = Arc::new(crate::app_reader::AppReader::new(sources.clone()));
    let params = json!({"session":"alfonso:s1", "harness":"broca", "limit":6});
    let page_app = app.clone();
    let page_params = params.clone();
    let page = tokio::spawn(async move {
        crate::app_reader::tests::serve(&page_app, crate::app_reader::TRANSCRIPT_PAGE, &page_params)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), route.describe_started.notified())
        .await
        .unwrap();
    let mut params = params;
    params["claim"] = json!({"heldFloorMessageId":"m145", "sinceMessageId":"m149", "heldVersion":"history:7:lineage:cut-a"});
    let second = tokio::spawn(async move {
        crate::app_reader::tests::serve_on_channel(
            &app,
            crate::app_reader::TRANSCRIPT_WATCH,
            &params,
            2,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), route.describe_started.notified())
        .await
        .unwrap();
    assert!(!page.is_finished());
    assert!(!second.is_finished());
    route.hold_describe.store(false, Ordering::Release);
    route.describe_release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(4), async {
        page.await.unwrap();
        second.await.unwrap();
    })
    .await
    .expect("released describes must not deadlock shared source state");
}

struct SlowLookup {
    hold: AtomicBool,
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[tokio::test(start_paused = true)]
async fn reader_deadline_covers_resolution_and_accumulated_remote_work() {
    use crate::app_reader::{observe, tests::serve_frame, TRANSCRIPT_PAGE};
    use subc_protocol::FrameType;

    for hold_lookup in [true, false] {
        let (log, _guard) = observe::tests::capture();
        let route = StrictRoute::new(151);
        // Each call can finish within the transport's 30 second budget, while
        // their sum still exceeds the budget for one phone request.
        if !hold_lookup {
            route.read_delay_ms.store(20_000, Ordering::Release);
        }
        let lookup = Arc::new(SlowLookup {
            hold: AtomicBool::new(hold_lookup),
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        let sources = Arc::new(BrocaSources::new(
            Arc::new(Resolver::new(lookup.clone())),
            Arc::new(StrictRoutes(route.clone())),
        ));
        let app = Arc::new(crate::app_reader::AppReader::new(sources));
        let params = json!({"session":"alfonso:s1", "harness":"broca", "limit":6});
        let started = tokio::time::Instant::now();
        let frame = serve_frame(
            &app,
            TRANSCRIPT_PAGE,
            &params,
            1,
            observe::READ_DEADLINE + Duration::from_secs(1),
        )
        .await;
        assert_eq!(started.elapsed(), observe::READ_DEADLINE);
        assert_eq!(frame.header.ty, FrameType::Error);
        let refusal: Value = serde_json::from_slice(&frame.body).unwrap();
        assert_eq!(refusal["code"], "provider_unavailable");
        assert_eq!(refusal["detail"]["retryable"], true);
        let text = log.text();
        assert_eq!(
            text.matches("app reader request finished").count(),
            1,
            "{text}"
        );
        assert!(text.contains("outcome=\"deadline\""), "{text}");
        assert!(text.contains("duration_ms=45000"), "{text}");
        // Without the held lookup, the two head calls take 40 of the 45 seconds, so
        // the deadline falls during the next call: the initial `describe` request,
        // which the read uses to check whether the runner stamps rich UI.
        let step = if hold_lookup {
            "entorhinal.agent.session_for_runner"
        } else {
            "broca.role.describe"
        };
        assert!(text.contains(&format!("pending_step=\"{step}\"")), "{text}");

        // Cancelling a read must release the session slot and binding locks, so
        // a later retry can use the same module after the provider recovers.
        lookup.hold.store(false, Ordering::Release);
        route.read_delay_ms.store(0, Ordering::Release);
        let retry = serve_frame(&app, TRANSCRIPT_PAGE, &params, 1, Duration::from_secs(1)).await;
        assert_eq!(retry.header.ty, FrameType::Response);
        let reply: Value = serde_json::from_slice(&retry.body).unwrap();
        assert_eq!(reply["result"]["messages"].as_array().unwrap().len(), 6);
    }
}

#[async_trait]
impl Entorhinal for SlowLookup {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        if self.hold.load(Ordering::Acquire) {
            self.started.notify_one();
            self.release.notified().await;
        }
        Lookup.request(method, params).await
    }
}

#[tokio::test(start_paused = true)]
async fn concurrent_page_and_watch_finish_after_a_slow_binding_lookup() {
    // The resolver and source use only Tokio gates, so wall-clock load must not
    // decide whether the shared binding lock was released.
    let route = StrictRoute::new(151);
    let lookup = Arc::new(SlowLookup {
        hold: AtomicBool::new(true),
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let sources = Arc::new(BrocaSources::new(
        Arc::new(Resolver::new(lookup.clone())),
        Arc::new(StrictRoutes(route)),
    ));
    let app = Arc::new(crate::app_reader::AppReader::new(sources));
    let params = json!({"session":"alfonso:s1", "harness":"broca", "limit":6});
    let page_app = app.clone();
    let page_params = params.clone();
    let page = tokio::spawn(async move {
        crate::app_reader::tests::serve(&page_app, crate::app_reader::TRANSCRIPT_PAGE, &page_params)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), lookup.started.notified())
        .await
        .unwrap();
    let mut params = params;
    params["claim"] = json!({"heldFloorMessageId":"m145", "sinceMessageId":"m149", "heldVersion":"history:7:lineage:cut-a"});
    let watch = tokio::spawn(async move {
        crate::app_reader::tests::serve_on_channel(
            &app,
            crate::app_reader::TRANSCRIPT_WATCH,
            &params,
            2,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(!page.is_finished());
    assert!(!watch.is_finished());
    lookup.hold.store(false, Ordering::Release);
    lookup.release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(4), async {
        page.await.unwrap();
        watch.await.unwrap();
    })
    .await
    .expect("released binding lookup must not deadlock shared source state");
}

pub(super) fn populated(count: u64) -> Arc<FakeRoute> {
    let route = FakeRoute::new();
    route
        .head_extra
        .lock()
        .unwrap()
        .insert("last_ordinal".into(), json!(count - 1));
    *route.describe.lock().unwrap() = Some(describe_with(&["history_display"]));
    *route.entries.lock().unwrap() = (0..count)
        .map(|ordinal| {
            json!({"mid":format!("m{ordinal}"), "ordinal":ordinal, "written_at":705,
            "message":{"role":"user", "content":[{"type":"text", "text":"hello"}]}})
        })
        .collect();
    for ordinal in 0..count {
        let messages: Vec<_> = (ordinal..(ordinal + 2).min(count))
            .map(|n| {
                DisplayEntry::new(
                    format!("m{n}"),
                    705,
                    DisplayRow::new("user", vec![]).with_client_message_id(format!("phone-{n}")),
                )
                .with_ordinal(n)
            })
            .collect();
        route.history_replies.lock().unwrap().insert(
            ordinal,
            Ok(json!({"messages":messages, "lineage_id":"cut-a", "history_version":7})),
        );
    }
    route
}

#[tokio::test]
async fn numbered_history_before_a_cut_keeps_ordinal_and_mid_cursors() {
    let route = populated(8);
    route
        .head_extra
        .lock()
        .unwrap()
        .insert("first_ordinal".into(), json!(4));
    let source = BrocaSource::new(resolved(), route);
    let page = source.newest(4).await.unwrap();
    assert_eq!(page.rows[0].mid, "m4");
    let older = source
        .older(&Anchor::Cursor(page.older.unwrap()), 2)
        .await
        .unwrap();
    assert_eq!(
        older
            .rows
            .iter()
            .map(|row| row.mid.as_str())
            .collect::<Vec<_>>(),
        ["m2", "m3"]
    );
    let oldest = source.older(&Anchor::Ordinal(2), 2).await.unwrap();
    assert_eq!(
        oldest
            .rows
            .iter()
            .map(|row| row.mid.as_str())
            .collect::<Vec<_>>(),
        ["m0", "m1"]
    );
    assert!(oldest.older.is_none());
    let exact = source.exact(&Anchor::Ordinal(1)).await.unwrap().unwrap();
    assert_eq!(exact.mid, "m1");
    assert_eq!(exact.ordinal, Some(1));
    let after = source.newer("m3", 2).await.unwrap();
    assert_eq!(
        after
            .rows
            .iter()
            .map(|row| row.mid.as_str())
            .collect::<Vec<_>>(),
        ["m4", "m5"]
    );
    assert!(source.exact(&Anchor::Ordinal(99)).await.unwrap().is_none());
}

#[tokio::test]
async fn unavailable_host_history_does_not_hide_the_numbered_window() {
    let route = populated(4);
    route.anchor_replies.lock().unwrap().insert(
        "m0".into(),
        Err(ReadError::new(
            "history_unrepresentable",
            "old rows have no recorded time",
        )),
    );
    let source = BrocaSource::new(resolved(), route);
    let page = source.newest(4).await.unwrap();
    assert_eq!(page.rows.len(), 4);
    assert_eq!(page.rows[0].mid, "m0");
    assert_eq!(page.rows[3].mid, "m3");
    assert!(
        page.older.is_some(),
        "unavailable history must not imply reachedStart"
    );
}

#[tokio::test]
async fn reader_window_cost_does_not_grow_with_the_transcript() {
    let mut costs = Vec::new();
    for count in [100, 20_000] {
        let route = populated(count);
        let (_, app) = serving(route.clone());
        let request = json!({"session":"alfonso:s1", "harness":"broca", "limit":4});
        let newest = app
            .read(crate::app_reader::TRANSCRIPT_PAGE, request.clone())
            .await
            .unwrap();
        assert_eq!(
            newest["messages"][0]["messageId"],
            format!("m{}", count - 4)
        );
        assert_eq!(
            newest["messages"][3]["clientMessageId"],
            format!("phone-{}", count - 1)
        );
        let mut claim = request.clone();
        claim["claim"] = json!({"heldFloorMessageId":format!("m{}", count - 4),
            "sinceMessageId":format!("m{}", count - 2), "heldVersion":newest["version"]});
        let matched = app
            .read(crate::app_reader::TRANSCRIPT_PAGE, claim)
            .await
            .unwrap();
        assert_eq!(matched["claim"], "matched");
        assert_eq!(
            matched["newTurns"][0]["messageId"],
            format!("m{}", count - 1)
        );
        let anchors = |route: &FakeRoute| {
            route
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| {
                    call["method"] == "session.history" && call["params"]["form"] == "raw"
                })
                .count()
        };
        assert_eq!(
            anchors(&route),
            0,
            "an anchor on the newest page needs no lookup"
        );
        let mut gap = request.clone();
        gap["claim"] = json!({"heldFloorMessageId":format!("m{}", count - 21),
            "sinceMessageId":format!("m{}", count - 20), "heldVersion":newest["version"]});
        gap["acceptClaims"] = json!(["outside_window"]);
        let gap = app
            .read(crate::app_reader::TRANSCRIPT_PAGE, gap)
            .await
            .unwrap();
        assert_eq!(gap["claim"], "outside_window");
        assert_eq!(anchors(&route), 1, "a gap gets one bounded anchor lookup");
        let mut older = request;
        older["beforeOrdinal"] = json!(count - 4);
        let older = app
            .read(crate::app_reader::TRANSCRIPT_PAGE, older)
            .await
            .unwrap();
        assert_eq!(older["messages"][0]["messageId"], format!("m{}", count - 8));
        let calls = route.calls.lock().unwrap();
        costs.push((
            calls
                .iter()
                .filter(|call| call["method"] == "session.read")
                .count(),
            route.read_entries.load(Ordering::Relaxed),
            calls
                .iter()
                .filter(|call| call["method"] == "session.history")
                .count(),
        ));
    }
    assert_eq!(
        costs[0], costs[1],
        "serving the same windows must not scan old turns"
    );
    assert_eq!(
        costs[0],
        (8, 16, 9),
        "eight raw reads, sixteen raw entries, eight display echoes and one anchor lookup"
    );
}
