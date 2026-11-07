use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use async_trait::async_trait;
use occipital_app_reader::claim::NewestPageFit;
use occipital_app_reader::source::{
    Anchor, Attachments, Head, HistoryPage, InputCapability, ReadError, Row, TranscriptReader,
    TranscriptSource,
};
use occipital_app_reader::transcript_watch::{
    self, advance_claim, message_ids, TranscriptEmitted, TranscriptWatchEnd, TranscriptWatchHost,
    NOTICE_BOUND,
};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

struct State {
    rows: Vec<Row>,
    history_version: Option<u64>,
    incarnation: Option<String>,
    activity: (&'static str, i64),
    error: Option<&'static str>,
    unstable: usize,
    rewrite_at: Option<usize>,
    wrong_page_version: bool,
    exact_unavailable: bool,
    last_run: Option<cortexkit_role_llm_runner::read::LastRunState>,
    service_tier: Option<String>,
    history_intake: bool,
    host: Option<String>,
}

struct FakeSource {
    harness: &'static str,
    state: Mutex<State>,
    changes: watch::Sender<u64>,
    newest_reads: AtomicUsize,
    chunk: AtomicUsize,
}

impl FakeSource {
    fn new(harness: &'static str, count: usize, bytes: usize, native: bool) -> Arc<Self> {
        let (changes, _) = watch::channel(0);
        Arc::new(Self {
            harness,
            state: Mutex::new(State {
                rows: (0..count).map(|ordinal| row(ordinal, bytes)).collect(),
                history_version: native.then_some(1),
                incarnation: None,
                activity: ("idle", 1),
                error: None,
                unstable: 0,
                rewrite_at: None,
                wrong_page_version: false,
                exact_unavailable: false,
                last_run: None,
                service_tier: None,
                history_intake: false,
                host: None,
            }),
            changes,
            newest_reads: AtomicUsize::new(0),
            chunk: AtomicUsize::new(100),
        })
    }
    fn notify(&self) {
        self.changes.send_modify(|seq| *seq += 1);
    }
    fn append(&self, count: usize, bytes: usize) {
        let mut state = self.state.lock().unwrap();
        let start = state.rows.len();
        state
            .rows
            .extend((start..start + count).map(|ordinal| row(ordinal, bytes)));
        drop(state);
        self.notify();
    }
    fn page(&self, start: usize, end: usize) -> Result<HistoryPage, ReadError> {
        let state = self.state.lock().unwrap();
        if let Some(code) = state.error {
            return Err(ReadError::new(code, "injected source failure"));
        }
        Ok(HistoryPage {
            rows: state.rows[start..end].to_vec(),
            older: (start > 0 && start < state.rows.len())
                .then(|| format!("older:{}", state.rows[start].mid)),
            newer: (end < state.rows.len()).then(|| format!("newer:{}", state.rows[end - 1].mid)),
            history_version: if state.wrong_page_version {
                Some(999)
            } else {
                state.history_version
            },
            incarnation: state.incarnation.clone(),
        })
    }
}

fn row(ordinal: usize, bytes: usize) -> Row {
    Row {
        mid: format!("m{ordinal}"),
        ordinal: Some(ordinal as u64),
        written_at: Some(1700000000000),
        display: json!({"role": if ordinal.is_multiple_of(2) {"user"} else {"assistant"}, "text": "x".repeat(bytes), "blockSummaries": [], "parts": [], "clientMessageId": format!("client-{ordinal}")}),
    }
}

#[async_trait]
impl TranscriptSource for FakeSource {
    fn harness(&self) -> &'static str {
        self.harness
    }
    async fn head(&self) -> Result<Head, ReadError> {
        let state = self.state.lock().unwrap();
        if let Some(code) = state.error {
            return Err(ReadError::new(code, "injected source failure"));
        }
        Ok(Head {
            first_ordinal: state.rows.iter().find_map(|row| row.ordinal),
            activity_token: None,
            history_version: state.history_version,
            incarnation: state.incarnation.clone(),
            last_run: state.last_run.clone(),
            service_tier: state.service_tier.clone(),
            history_intake: state.history_intake,
            host: state.host.clone(),
            input: InputCapability {
                attachments: Attachments::Yes,
                mime_types: Some(vec!["image/png".into()]),
                modalities: Some(vec!["image".into()]),
                ..Default::default()
            },
            ..Default::default()
        })
    }
    async fn newest(&self, limit: u32) -> Result<HistoryPage, ReadError> {
        let read = self.newest_reads.fetch_add(1, Ordering::SeqCst) + 1;
        let (len, changed) = {
            let mut state = self.state.lock().unwrap();
            let mut changed = false;
            if state.rewrite_at == Some(read) {
                state.rows[0].display["text"] = json!("rewritten mid-fallback");
                state.rewrite_at = None;
                changed = true;
            }
            if state.unstable > 0 {
                state.unstable -= 1;
                changed = true;
            }
            (state.rows.len(), changed)
        };
        if changed {
            self.notify();
        }
        self.page(len.saturating_sub(limit as usize), len)
    }
    async fn older(&self, anchor: &Anchor, limit: u32) -> Result<HistoryPage, ReadError> {
        let (start, end) = {
            let state = self.state.lock().unwrap();
            let len = state.rows.len();
            let index = |mid: &str| {
                state
                    .rows
                    .iter()
                    .position(|row| row.mid == mid)
                    .ok_or_else(|| ReadError::new("cursor_expired", "deleted anchor"))
            };
            match anchor {
                Anchor::Ordinal(ordinal) => {
                    let end = state
                        .rows
                        .iter()
                        .position(|row| row.ordinal.is_some_and(|n| n >= *ordinal))
                        .unwrap_or(len);
                    (end.saturating_sub(limit as usize), end)
                }
                Anchor::Mid(mid) => {
                    let end = index(mid)?;
                    (end.saturating_sub(limit as usize), end)
                }
                Anchor::Newest => (len.saturating_sub(limit as usize), len),
                Anchor::Cursor(cursor) => {
                    let (direction, mid) = cursor.split_once(':').unwrap();
                    let index = index(mid)?;
                    if direction == "older" {
                        (index.saturating_sub(limit as usize), index)
                    } else {
                        let start = index + 1;
                        (start, (start + limit as usize).min(len))
                    }
                }
            }
        };
        self.page(start, end)
    }
    async fn newer(&self, from_mid: &str, limit: u32) -> Result<HistoryPage, ReadError> {
        let mut page = self
            .older(
                &Anchor::Cursor(format!("newer:{from_mid}")),
                limit.min(self.chunk.load(Ordering::SeqCst) as u32),
            )
            .await?;
        if self.state.lock().unwrap().wrong_page_version {
            page.history_version = Some(999);
        }
        Ok(page)
    }
    async fn exact(&self, anchor: &Anchor) -> Result<Option<Row>, ReadError> {
        let state = self.state.lock().unwrap();
        if state.exact_unavailable {
            return Err(ReadError::new(
                "provider_unavailable",
                "full content was pruned",
            ));
        }
        Ok(state
            .rows
            .iter()
            .find(|row| match anchor {
                Anchor::Mid(mid) => row.mid == *mid,
                Anchor::Ordinal(ordinal) => row.ordinal == Some(*ordinal),
                _ => false,
            })
            .cloned())
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    fn activity(&self) -> Option<(&'static str, i64)> {
        Some(self.state.lock().unwrap().activity)
    }
    fn activity_feed_live(&self) -> bool {
        self.state.lock().unwrap().error.is_none()
    }
}

fn claim_for(page: &Value) -> Value {
    let ids = message_ids(page);
    json!({"heldFloorMessageId": ids.first().unwrap(), "sinceMessageId": ids.last().unwrap(), "heldVersion": page["version"]})
}

async fn read(reader: &TranscriptReader, source: &FakeSource, params: Value) -> Value {
    reader.read(Some(source), params).await.unwrap()
}

struct Host {
    source: Arc<FakeSource>,
    reader: Arc<TranscriptReader>,
    params: Value,
    tx: mpsc::UnboundedSender<Value>,
    stop: watch::Receiver<bool>,
    evaluations: AtomicUsize,
    refuse: std::sync::atomic::AtomicBool,
    force_outside_window: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl TranscriptWatchHost for Host {
    type Refusal = ();
    async fn read(&self, params: Value) -> Result<Value, ReadError> {
        let force =
            params.get("claim").is_some() && self.force_outside_window.load(Ordering::SeqCst);
        let mut reply = self
            .reader
            .read_for_watch(Some(&*self.source), params)
            .await?;
        if force && reply["claim"] == "cursor_expired" {
            reply["claim"] = json!("outside_window");
        }
        Ok(reply)
    }
    fn trimmed_newest(
        &self,
        session: &str,
        version: &str,
        newest: &Value,
        fits: &mut (dyn FnMut(&Value) -> bool + Send),
    ) -> Option<NewestPageFit> {
        self.reader.trimmed_newest(session, version, newest, fits)
    }
    fn subscribe_changes(&self, _: &str) -> watch::Receiver<u64> {
        self.source.changes()
    }
    fn readmit(&self) -> Result<Value, ()> {
        self.evaluations.fetch_add(1, Ordering::SeqCst);
        if self.refuse.load(Ordering::SeqCst) {
            Err(())
        } else {
            Ok(self.params.clone())
        }
    }
    async fn emit(&self, body: Vec<u8>) -> Result<(), ()> {
        self.tx
            .send(serde_json::from_slice(&body).unwrap())
            .map_err(|_| ())
    }
    async fn stopped(&self) {
        let mut stop = self.stop.clone();
        let _ = stop.wait_for(|stop| *stop).await;
    }
    fn is_stopped(&self) -> bool {
        *self.stop.borrow()
    }
}

struct Running {
    host: Arc<Host>,
    rx: mpsc::UnboundedReceiver<Value>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<TranscriptWatchEnd<()>>,
}
impl Running {
    fn start(source: Arc<FakeSource>, reader: Arc<TranscriptReader>, claim: Value) -> Self {
        let params = json!({"session": "s", "claim": claim});
        Self::with_params(source, reader, params)
    }
    fn start_accepting(
        source: Arc<FakeSource>,
        reader: Arc<TranscriptReader>,
        claim: Value,
    ) -> Self {
        let params = json!({"session":"s", "claim":claim, "acceptClaims":["outside_window"]});
        Self::with_params(source, reader, params)
    }
    fn with_params(source: Arc<FakeSource>, reader: Arc<TranscriptReader>, params: Value) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (stop, stopped) = watch::channel(false);
        let host = Arc::new(Host {
            source,
            reader,
            params: params.clone(),
            tx,
            stop: stopped,
            evaluations: AtomicUsize::new(0),
            refuse: std::sync::atomic::AtomicBool::new(false),
            force_outside_window: std::sync::atomic::AtomicBool::new(false),
        });
        let run_host = host.clone();
        let task = tokio::spawn(async move { transcript_watch::run(&*run_host, &params).await });
        Self {
            host,
            rx,
            stop,
            task,
        }
    }
    async fn next(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(2), self.rx.recv())
            .await
            .expect("notice before timeout")
            .expect("watch notice")
    }
    async fn quiet(&mut self, millis: u64) {
        assert!(
            tokio::time::timeout(Duration::from_millis(millis), self.rx.recv())
                .await
                .is_err(),
            "unexpected notice"
        );
    }
    async fn end(self) {
        self.stop.send_replace(true);
        assert!(matches!(
            self.task.await.unwrap(),
            TranscriptWatchEnd::Streamed
        ));
    }
}

#[tokio::test(start_paused = true)]
async fn transcript_watch_gaps_respect_accept_claims_including_oversize_fallback() {
    for bytes in [8, 60_000] {
        for force_outside_window in [false, true] {
            for (accepted, expected) in [
                (None, "cursor_expired"),
                (Some(json!([])), "cursor_expired"),
                (Some(json!(["future_verdict"])), "cursor_expired"),
                (
                    Some(json!(["future_verdict", "outside_window"])),
                    "outside_window",
                ),
            ] {
                let source = FakeSource::new("broca", 2, 4, true);
                let reader = Arc::new(TranscriptReader::default());
                let baseline = read(&reader, &source, json!({"session":"s"})).await;
                source.append(205, bytes);
                let mut params = json!({"session":"s", "claim":claim_for(&baseline), "limit":4});
                if let Some(accepted) = accepted {
                    params["acceptClaims"] = accepted;
                }
                let mut running = Running::with_params(source.clone(), reader, params);
                running
                    .host
                    .force_outside_window
                    .store(force_outside_window, Ordering::SeqCst);
                let reply = running.next().await;
                assert_eq!(
                    reply["claim"], expected,
                    "bytes={bytes}, forced host={force_outside_window}: {reply}"
                );
                assert!(serde_json::to_vec(&reply).unwrap().len() <= NOTICE_BOUND);
                let kept = message_ids(&reply["resync"]);
                assert!(!kept.is_empty());
                if bytes == 8 {
                    assert_eq!(kept.len(), 4);
                } else {
                    assert!(kept.len() < 4, "the oversize resync must have been fitted");
                }
                running
                    .host
                    .force_outside_window
                    .store(false, Ordering::SeqCst);
                source.append(1, 4);
                let matched = running.next().await;
                assert_eq!(matched["claim"], "matched");
                assert_eq!(matched["answers"]["heldFloorMessageId"], kept[0]);
                assert_eq!(matched["answers"]["sinceMessageId"], "m206");
                assert_eq!(matched["newTurns"][0]["messageId"], "m207");
                source.append(5, 4);
                assert_eq!(
                    running.next().await["claim"],
                    expected,
                    "acceptance must survive claim advancement"
                );
                running.end().await;
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn malformed_accept_claims_ends_a_watch_as_invalid_claim() {
    for accepted in [
        Value::Null,
        json!(true),
        json!(7),
        json!("outside_window"),
        json!({}),
        json!(["outside_window", false]),
    ] {
        let source = FakeSource::new("broca", 2, 4, true);
        let reader = Arc::new(TranscriptReader::default());
        let page = read(&reader, &source, json!({"session":"s"})).await;
        let reads = source.newest_reads.load(Ordering::SeqCst);
        let running = Running::with_params(
            source.clone(),
            reader,
            json!({"session":"s", "claim":claim_for(&page), "acceptClaims":accepted}),
        );
        let end = running.task.await.unwrap();
        assert!(matches!(end, TranscriptWatchEnd::Error {code, message}
            if code == "invalid_claim" && message == "acceptClaims must be an array of strings"));
        assert_eq!(source.newest_reads.load(Ordering::SeqCst), reads);
    }
}

#[tokio::test]
async fn history_pages_neither_repeat_nor_skip_across_next_before_ordinal() {
    for harness in ["opencode2", "broca", "pi"] {
        let source = FakeSource::new(harness, 350, 4, true);
        let reader = TranscriptReader::default();
        let mut params = json!({"session": "s", "limit": 100});
        let mut all = Vec::new();
        loop {
            let page = read(&reader, &source, params.clone()).await;
            assert_eq!(page["historyVersion"], 1);
            let mut ids = message_ids(&page);
            ids.extend(all);
            all = ids;
            if page["reachedStart"] == true {
                assert!(
                    page.get("nextBeforeOrdinal").is_none()
                        && page.get("nextBeforeCursor").is_none()
                );
                break;
            }
            assert!(page["nextBeforeOrdinal"].is_number(), "{page}");
            params["beforeOrdinal"] = page["nextBeforeOrdinal"].clone();
            assert!(all.len() <= 350, "repeated history window");
        }
        assert_eq!(all, (0..350).map(|n| format!("m{n}")).collect::<Vec<_>>());
    }
}

#[tokio::test]
async fn pre_adoption_history_uses_restart_safe_cursors_and_never_enters_claims() {
    let source = FakeSource::new("opencode2", 12, 4, true);
    for row in &mut source.state.lock().unwrap().rows[..8] {
        row.ordinal = None;
    }
    let reader = TranscriptReader::default();
    let page = read(&reader, &source, json!({"session":"s", "limit":6})).await;
    assert_eq!(message_ids(&page), ["m8", "m9", "m10", "m11"]);
    assert!(page.get("nextBeforeOrdinal").is_none());
    let mut cursor = page["nextBeforeCursor"].clone();
    assert!(cursor.as_str().unwrap().contains("m8"));
    let held = claim_for(&page);
    let mut ids = Vec::new();
    let restarted = TranscriptReader::default();
    loop {
        let older = read(
            &restarted,
            &source,
            json!({"session":"s", "beforeCursor":cursor, "limit":3}),
        )
        .await;
        assert!(older["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row.get("ordinal").is_none()));
        assert_eq!(older["historyVersion"], page["historyVersion"]);
        let mut next = message_ids(&older);
        next.extend(ids);
        ids = next;
        if older["reachedStart"] == true {
            break;
        }
        assert!(older.get("nextBeforeOrdinal").is_none());
        cursor = older["nextBeforeCursor"].clone();
    }
    assert_eq!(ids, (0..8).map(|n| format!("m{n}")).collect::<Vec<_>>());
    source.append(1, 4);
    let matched = read(&reader, &source, json!({"session":"s", "claim":held})).await;
    assert_eq!(matched["claim"], "matched");
    assert_eq!(matched["newTurns"][0]["messageId"], "m12");
    source.state.lock().unwrap().history_version = Some(2);
    let error = reader
        .read(
            Some(&*source),
            json!({"session":"s", "beforeCursor":cursor}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "cursor_expired");
}

#[tokio::test]
async fn a_numbered_boundary_uses_a_cursor_even_when_the_page_ends_exactly_there() {
    let source = FakeSource::new("pi", 6, 4, true);
    for row in &mut source.state.lock().unwrap().rows[..3] {
        row.ordinal = None;
    }
    let reader = TranscriptReader::default();
    let page = read(&reader, &source, json!({"session":"s", "limit":3})).await;
    assert!(page.get("nextBeforeOrdinal").is_none());
    let older = read(
        &reader,
        &source,
        json!({"session":"s", "beforeCursor":page["nextBeforeCursor"]}),
    )
    .await;
    assert_eq!(message_ids(&older), ["m0", "m1", "m2"]);
    assert_eq!(older["reachedStart"], true);
}

#[tokio::test]
async fn a_recreated_log_and_history_rewrites_each_expire_claims() {
    let source = FakeSource::new("broca", 3, 4, true);
    source.state.lock().unwrap().incarnation = Some("log-1".into());
    let reader = TranscriptReader::default();
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let held = claim_for(&page);
    source.state.lock().unwrap().incarnation = Some("log-2".into());
    source.notify();
    let cut = read(&reader, &source, json!({"session":"s", "claim":held})).await;
    assert_eq!(cut["claim"], "cursor_expired");
    assert_eq!(cut["historyVersion"], page["historyVersion"]);
    let mut held = held.as_object().unwrap().clone();
    advance_claim(&mut held, &cut);
    source.state.lock().unwrap().history_version = Some(2);
    source.notify();
    let expired = read(&reader, &source, json!({"session":"s", "claim":held})).await;
    assert_eq!(expired["claim"], "cursor_expired");
    assert_eq!(expired["historyVersion"], 2);
    let mut advanced = held;
    advance_claim(&mut advanced, &expired);
    let matched = read(&reader, &source, json!({"session":"s", "claim":advanced})).await;
    assert_eq!(matched["claim"], "matched");
}

#[tokio::test]
async fn a_native_gap_resync_pages_back_without_losing_turns() {
    for harness in ["opencode2", "broca", "pi"] {
        let source = FakeSource::new(harness, 2, 4, true);
        let reader = TranscriptReader::default();
        let page = read(&reader, &source, json!({"session":"s"})).await;
        source.append(205, 4);
        source.changes.send_replace(1000);
        let reply = read(
            &reader,
            &source,
            json!({"session":"s", "claim":claim_for(&page), "limit":3, "acceptClaims":["outside_window"]}),
        )
        .await;
        assert_eq!(reply["claim"], "outside_window");
        assert_eq!(reply["version"], page["version"]);
        let mut current = reply["resync"].clone();
        let mut collected = std::collections::BTreeMap::new();
        loop {
            let rows = current["messages"].as_array().unwrap();
            assert!(rows.len() <= 3);
            for row in rows {
                assert!(
                    collected
                        .insert(
                            row["ordinal"].as_u64().unwrap(),
                            row["messageId"].as_str().unwrap().to_string()
                        )
                        .is_none(),
                    "pages must not overlap"
                );
            }
            if rows.iter().any(|row| row["messageId"] == "m1") {
                break;
            }
            let cursor = current["nextBeforeOrdinal"].as_u64().unwrap();
            current = read(
                &reader,
                &source,
                json!({"session":"s", "beforeOrdinal":cursor, "limit":3}),
            )
            .await;
        }
        assert_eq!(
            collected
                .range(2..)
                .map(|(_, mid)| mid.clone())
                .collect::<Vec<_>>(),
            (2..207)
                .map(|ordinal| format!("m{ordinal}"))
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn deleted_or_unnumbered_claim_anchors_resync_instead_of_vouching() {
    let source = FakeSource::new("opencode2", 4, 4, true);
    let reader = TranscriptReader::default();
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let held = claim_for(&page);
    source.state.lock().unwrap().rows.remove(3);
    let reply = read(&reader, &source, json!({"session":"s", "claim":held})).await;
    assert_eq!(reply["claim"], "cursor_expired");
    assert_eq!(message_ids(&reply["resync"]), ["m0", "m1", "m2"]);
}

#[tokio::test]
async fn exact_messages_are_full_and_pruned_messages_are_named() {
    for harness in ["opencode2", "broca", "pi"] {
        let source = FakeSource::new(harness, 2, 200_000, true);
        let reader = TranscriptReader::default();
        let page = read(&reader, &source, json!({"session":"s", "messageOrdinal":1})).await;
        assert_eq!(page["messages"][0]["text"].as_str().unwrap().len(), 200_000);
        assert_eq!(page["messages"][0]["clientMessageId"], "client-1");
        let missing = read(
            &reader,
            &source,
            json!({"session":"s", "messageOrdinal":30}),
        )
        .await;
        assert!(missing["fullContentUnavailable"]
            .as_str()
            .unwrap()
            .contains("source no longer contains message [30]"));
        source.state.lock().unwrap().exact_unavailable = true;
        let summary = read(&reader, &source, json!({"session":"s", "messageOrdinal":1})).await;
        assert_eq!(summary["fullContentUnavailable"], "full content was pruned");
        assert_eq!(summary["messages"][0]["messageId"], "m1");
        assert!(summary.get("fullContent").is_none());
        assert_eq!(message_ids(&missing), Vec::<String>::new());
    }
}

#[tokio::test]
async fn stable_pages_offer_tokens_but_double_instability_never_vouches() {
    let source = FakeSource::new("opencode2", 3, 4, false);
    let reader = TranscriptReader::default();
    let first = read(&reader, &source, json!({"session":"s"})).await;
    let unchanged = read(
        &reader,
        &source,
        json!({"session":"s", "activityToken":first["activityToken"]}),
    )
    .await;
    assert_eq!(unchanged["unchanged"], true);
    assert_eq!(source.newest_reads.load(Ordering::SeqCst), 1);
    source.state.lock().unwrap().unstable = 2;
    source.notify();
    let unstable = read(&reader, &source, json!({"session":"s"})).await;
    assert!(unstable.get("version").is_none());
    assert_eq!(unstable["discardActivityToken"], true);
    assert_eq!(source.newest_reads.load(Ordering::SeqCst), 3);
    source.state.lock().unwrap().unstable = 2;
    let reply = read(
        &reader,
        &source,
        json!({"session":"s", "claim":claim_for(&first)}),
    )
    .await;
    assert_eq!(reply["error"]["code"], "observation_unstable");
}

#[tokio::test]
async fn source_errors_and_request_refusals_keep_their_codes() {
    let reader = TranscriptReader::default();
    let source = FakeSource::new("pi", 1, 4, true);
    let bad = reader
        .read(Some(&*source), json!({"session":"s", "harness":"broca"}))
        .await
        .unwrap_err();
    assert_eq!(bad.code, "invalid_request");
    let held = json!({"heldFloorMessageId":"m0", "sinceMessageId":"m0", "heldVersion":"v"});
    let mismatch = read(
        &reader,
        &source,
        json!({"session":"s", "harness":"broca", "claim":held}),
    )
    .await;
    assert_eq!(mismatch["error"]["code"], "harness_mismatch");
    let unknown = reader
        .read(None, json!({"session":"unknown", "claim":held}))
        .await
        .unwrap();
    assert_eq!(unknown["error"]["code"], "session_unknown");
    // With no source nothing can say whether older messages exist, so the page
    // names its reason and never claims the start.
    let empty = reader
        .read(None, json!({"session":"unknown"}))
        .await
        .unwrap();
    assert_eq!(empty["emptyReason"], "provider_unavailable");
    assert!(empty.get("reachedStart").is_none(), "{empty}");
    let zero = read(
        &reader,
        &source,
        json!({"session":"s", "limit":0, "claim":held}),
    )
    .await;
    assert_eq!(zero["error"]["code"], "claim_limit_zero");
}

#[tokio::test(start_paused = true)]
async fn session_facts_emit_even_when_idle_and_survive_oversize_resync() {
    let source = FakeSource::new("broca", 3, 10, true);
    let reader = Arc::new(TranscriptReader::default());
    let baseline = read(&reader, &source, json!({"session":"s"})).await;
    let mut running = Running::start(source.clone(), reader.clone(), claim_for(&baseline));
    let notice = running.next().await;
    assert!(notice.get("lastRun").is_none());
    let run = json!({"run_id":"r2", "state":"error", "reason":"provider_error"});
    source.state.lock().unwrap().last_run = Some(serde_json::from_value(run).unwrap());
    source.notify();
    let expected = json!({"runId":"r2", "state":"error", "reason":"provider_error"});
    let notice = running.next().await;
    assert_eq!(notice["lastRun"], expected);
    assert_eq!(notice["activity"], "idle");
    assert!(notice["newTurns"].as_array().unwrap().is_empty());
    // Every session fact rides an oversized reply at its top level, never in the
    // trimmed page it carries.
    {
        let mut state = source.state.lock().unwrap();
        state.service_tier = Some("fast".into());
        state.history_intake = true;
        state.host = Some("unknown".into());
    }
    source.append(3, 90_000);
    let oversized = running.next().await;
    assert_eq!(oversized["lastRun"], expected);
    assert!(oversized["resync"].is_object(), "{oversized}");
    for (fact, value) in [
        ("lastRun", expected.clone()),
        ("serviceTier", json!("fast")),
        ("historyIntake", json!("running")),
        ("host", json!("unknown")),
    ] {
        assert_eq!(oversized[fact], value, "{fact}");
        assert!(oversized["resync"].get(fact).is_none(), "{fact} in resync");
    }
    assert!(serde_json::to_vec(&oversized).unwrap().len() <= NOTICE_BOUND);
    running.end().await;
}

#[tokio::test(start_paused = true)]
async fn transcript_watch_emits_matched_turns_divergence_and_trimmed_pages() {
    for harness in ["opencode2", "broca", "pi"] {
        let source = FakeSource::new(harness, 1, 4, false);
        let reader = Arc::new(TranscriptReader::default());
        let page = read(&reader, &source, json!({"session":"s"})).await;
        let held = claim_for(&page);
        source.append(1, 4);
        let mut running = Running::start(source.clone(), reader.clone(), held.clone());
        let first = running.next().await;
        assert_eq!(first["claim"], "matched");
        assert_eq!(first["answers"], held);
        assert_eq!(first["newTurns"][0]["messageId"], "m1");
        let mut phone = held.as_object().unwrap().clone();
        advance_claim(&mut phone, &first);
        source.append(2, 4);
        let two = running.next().await;
        assert_eq!(two["answers"], json!(phone));
        assert_eq!(
            message_ids(&json!({"messages":two["newTurns"]})),
            ["m2", "m3"]
        );
        advance_claim(&mut phone, &two);
        source.state.lock().unwrap().rows[1].display["text"] = json!("rewritten");
        source.notify();
        let diverged = running.next().await;
        assert_eq!(diverged["claim"], "diverged");
        assert_eq!(diverged["answers"], json!(phone));
        assert_eq!(diverged["validThroughMessageId"], "m0");
        advance_claim(&mut phone, &diverged);
        source.append(1, 4);
        let next = running.next().await;
        assert_eq!(next["claim"], "matched");
        assert_eq!(next["answers"], json!(phone));
        running.quiet(1500).await;
        running.end().await;
    }
}

#[tokio::test(start_paused = true)]
async fn transcript_watch_suppressed_replies_preserve_the_last_emitted_claim() {
    let source = FakeSource::new("pi", 1, 4, false);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let held = claim_for(&page);
    source.append(1, 4);
    let mut running = Running::start(source.clone(), reader.clone(), held.clone());
    let first = running.next().await;
    let mut phone = held.as_object().unwrap().clone();
    advance_claim(&mut phone, &first);
    for _ in 0..2 {
        source.notify();
        running.quiet(1500).await;
    }
    source.append(2, 4);
    let next = running.next().await;
    assert_eq!(next["answers"], json!(phone));
    assert_eq!(
        message_ids(&json!({"messages":next["newTurns"]})),
        ["m2", "m3"]
    );
    running.end().await;
}

#[tokio::test(start_paused = true)]
async fn transcript_watch_orders_tied_created_at_turns_by_ordinal() {
    let source = FakeSource::new("pi", 1, 4, true);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    source.append(3, 4);
    let mut running = Running::start(source.clone(), reader, claim_for(&page));
    let reply = running.next().await;
    let turns = reply["newTurns"].as_array().unwrap();
    assert_eq!(
        turns
            .iter()
            .map(|row| row["ordinal"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        turns
            .iter()
            .map(|row| row["role"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["assistant", "user", "assistant"]
    );
    assert!(turns
        .iter()
        .all(|row| row["createdAtMs"] == 1700000000000_i64));
    running.end().await;
}

#[tokio::test(start_paused = true)]
async fn an_oversize_transcript_reply_is_sent_as_a_trimmed_page_that_later_claims_match() {
    for harness in ["opencode2", "broca", "pi"] {
        let source = FakeSource::new(harness, 100, 3 * 1024, harness != "opencode2");
        let reader = Arc::new(TranscriptReader::default());
        let held =
            json!({"heldFloorMessageId":"m0", "sinceMessageId":"m99", "heldVersion":"stale"});
        let mut running = Running::start(source.clone(), reader.clone(), held.clone());
        let trimmed = running.next().await;
        assert!(serde_json::to_vec(&trimmed).unwrap().len() <= NOTICE_BOUND);
        assert_eq!(trimmed["claim"], "cursor_expired");
        assert_eq!(trimmed["answers"], held);
        let kept = message_ids(&trimmed["resync"]);
        assert!(!kept.is_empty() && kept.len() < 100);
        assert!(trimmed["resync"].get("reachedStart").is_none());
        assert!(trimmed["resync"].get("nextBeforeCursor").is_none());
        let older = read(&reader, &source, json!({"session":"s", "beforeOrdinal":trimmed["resync"]["nextBeforeOrdinal"], "limit":100-kept.len()})).await;
        let mut all = message_ids(&older);
        all.extend(kept);
        assert_eq!(all, (0..100).map(|n| format!("m{n}")).collect::<Vec<_>>());
        let mut phone = held.as_object().unwrap().clone();
        advance_claim(&mut phone, &trimmed);
        source.append(1, 4);
        let matched = running.next().await;
        assert_eq!(matched["claim"], "matched");
        assert_eq!(matched["answers"], json!(phone));
        assert_eq!(matched["newTurns"][0]["messageId"], "m100");
        assert_eq!(matched["acceptedModalities"], json!(["image"]));
        if harness != "opencode2" {
            assert_eq!(trimmed["historyVersion"], 1);
        }
        running.end().await;
    }
}

#[tokio::test(start_paused = true)]
async fn transcript_watch_broca_large_delta_sends_one_trimmed_page_that_reads_back_and_then_matches(
) {
    let source = FakeSource::new("broca", 2, 4, true);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let mut running = Running::start_accepting(source.clone(), reader.clone(), claim_for(&page));
    assert_eq!(running.next().await["claim"], "matched");
    source.append(100, 3 * 1024);
    let trimmed = running.next().await;
    assert_eq!(trimmed["claim"], "outside_window");
    let kept = message_ids(&trimmed["resync"]);
    let cursor = trimmed["resync"]["nextBeforeOrdinal"].as_u64().unwrap();
    let older = read(
        &reader,
        &source,
        json!({"session":"s", "beforeOrdinal":cursor}),
    )
    .await;
    assert_eq!(
        message_ids(&older).last().unwrap(),
        &format!("m{}", cursor - 1)
    );
    assert_eq!(kept.first().unwrap(), &format!("m{cursor}"));
    source.append(1, 4);
    let matched = running.next().await;
    assert_eq!(matched["newTurns"][0]["messageId"], "m102");
    assert_eq!(matched["claim"], "matched");
    assert_eq!(matched["answers"]["heldFloorMessageId"], kept[0]);
    assert_eq!(matched["answers"]["sinceMessageId"], "m101");
    assert_eq!(matched["answers"]["heldVersion"], trimmed["version"]);
    running.end().await;
}

#[tokio::test(start_paused = true)]
async fn transcript_watch_a_rewrite_between_an_oversize_read_and_its_fallback_still_diverges_for_the_old_version(
) {
    let source = FakeSource::new("opencode2", 1, 4, false);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let held = claim_for(&page);
    let mut running = Running::start(source.clone(), reader.clone(), held.clone());
    assert_eq!(running.next().await["claim"], "matched");
    source.append(80, 3 * 1024);
    source.state.lock().unwrap().rewrite_at = Some(source.newest_reads.load(Ordering::SeqCst) + 2);
    let replaced = running.next().await;
    assert_eq!(replaced["claim"], "cursor_expired");
    assert_eq!(
        source.state.lock().unwrap().rows[0].display["text"],
        "rewritten mid-fallback"
    );
    let other = read(&reader, &source, json!({"session":"s", "claim":held})).await;
    assert_eq!(other["claim"], "diverged");
    running.end().await;
}

#[tokio::test(start_paused = true)]
async fn transcript_watch_deleted_turns_return_the_expired_page_then_a_turn_matches() {
    let source = FakeSource::new("opencode2", 1, 4, false);
    let reader = Arc::new(TranscriptReader::default());
    let held = json!({"heldFloorMessageId":"m0", "sinceMessageId":"m0", "heldVersion":"unknown"});
    let mut running = Running::start(source.clone(), reader, held);
    let first = running.next().await;
    assert_eq!(first["claim"], "cursor_expired");
    source.append(2, 4);
    assert_eq!(
        running.next().await["newTurns"].as_array().unwrap().len(),
        2
    );
    source.state.lock().unwrap().rows.truncate(1);
    source.notify();
    let expired = running.next().await;
    assert_eq!(expired["claim"], "cursor_expired");
    assert_eq!(
        message_ids(&expired["resync"]),
        message_ids(&first["resync"])
    );
    running.quiet(1500).await;
    source.append(1, 4);
    assert_eq!(running.next().await["newTurns"][0]["messageId"], "m1");
    running.end().await;
}

#[tokio::test(start_paused = true)]
async fn a_newest_turn_too_large_for_a_notice_expires_without_a_page_and_ends() {
    let source = FakeSource::new("pi", 1, 250 * 1024, true);
    let reader = Arc::new(TranscriptReader::default());
    let held = json!({"heldFloorMessageId":"m0", "sinceMessageId":"m0", "heldVersion":"stale"});
    let mut running = Running::start(source, reader, held.clone());
    let bare = running.next().await;
    assert_eq!(bare["claim"], "cursor_expired");
    assert_eq!(bare["answers"], held);
    assert_eq!(bare["historyVersion"], 1);
    assert!(bare.get("resync").is_none());
    assert!(matches!(
        running.task.await.unwrap(),
        TranscriptWatchEnd::Streamed
    ));
}

#[tokio::test(start_paused = true)]
async fn transcript_watch_wakes_on_host_events_and_otherwise_only_on_the_slow_sweep() {
    let source = FakeSource::new("opencode2", 1, 4, true);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let mut running = Running::start(source.clone(), reader, claim_for(&page));
    running.next().await;
    running.quiet(29_000).await;
    assert_eq!(running.host.evaluations.load(Ordering::SeqCst), 1);
    running.quiet(1100).await;
    assert_eq!(running.host.evaluations.load(Ordering::SeqCst), 2);
    source.append(1, 4);
    assert_eq!(running.next().await["newTurns"][0]["messageId"], "m1");
    running.end().await;
}

#[tokio::test(start_paused = true)]
async fn a_hundred_events_a_second_cause_at_most_three_evaluations_a_second() {
    let source = FakeSource::new("opencode2", 1, 4, true);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let mut running = Running::start(source.clone(), reader, claim_for(&page));
    running.next().await;
    for _ in 0..100 {
        source.notify();
        tokio::time::advance(Duration::from_millis(10)).await;
        tokio::task::yield_now().await;
    }
    let evaluations = running.host.evaluations.load(Ordering::SeqCst);
    assert!(
        (2..=3).contains(&evaluations),
        "{evaluations} evaluations in one second"
    );
    running.end().await;
}

#[tokio::test(start_paused = true)]
async fn transcript_watch_pi_stays_silent_while_its_source_is_away_and_matches_after_reconnect() {
    let source = FakeSource::new("pi", 2, 4, true);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    source.state.lock().unwrap().error = Some("provider_unavailable");
    let mut running = Running::start(source.clone(), reader, claim_for(&page));
    running.quiet(2000).await;
    source.state.lock().unwrap().error = None;
    source.append(1, 4);
    let matched = running.next().await;
    assert_eq!(matched["claim"], "matched");
    assert_eq!(matched["newTurns"][0]["messageId"], "m2");
    running.end().await;
}

#[tokio::test(start_paused = true)]
async fn transient_reads_retry_and_admission_refusals_end_the_watch() {
    let source = FakeSource::new("broca", 1, 4, true);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    source.state.lock().unwrap().error = Some("transient");
    let mut running = Running::start(source.clone(), reader, claim_for(&page));
    running.quiet(4900).await;
    assert_eq!(running.host.evaluations.load(Ordering::SeqCst), 1);
    source.state.lock().unwrap().error = None;
    assert_eq!(running.next().await["claim"], "matched");
    running.host.refuse.store(true, Ordering::SeqCst);
    source.notify();
    assert!(matches!(
        running.task.await.unwrap(),
        TranscriptWatchEnd::Refused(())
    ));
}

#[test]
fn transcript_suppression_follows_page_material_turns_and_activity() {
    let page = json!({"messages": [{"messageId": "a", "ordinal": 0, "role": "user", "text": "x", "blockSummaries": [], "parts": []}]});
    let expired =
        json!({"claim": "cursor_expired", "version": "v1", "resync": page, "activity": "busy"});
    let mut emitted = TranscriptEmitted::default();
    assert!(emitted.should_emit(&expired));
    emitted.record(&expired);
    // Same page, same activity, nothing between: suppressed.
    assert!(!emitted.should_emit(&expired));
    // Activity changed: emitted.
    let mut idle = expired.clone();
    idle["activity"] = json!("idle");
    assert!(emitted.should_emit(&idle));
    // A matched notice with turns since the page: emitted again.
    let turns = json!({"claim": "matched", "version": "v2", "newTurns": [{"messageId": "b"}], "activity": "busy"});
    assert!(emitted.should_emit(&turns));
    emitted.record(&turns);
    assert!(emitted.should_emit(&expired));
    // A matched reply without turns and the same activity is not emitted.
    let quiet = json!({"claim": "matched", "version": "v2", "newTurns": [], "activity": "busy"});
    assert!(!emitted.should_emit(&quiet));

    let mut claim = json!({"heldFloorMessageId": "a", "sinceMessageId": "a", "heldVersion": "v0"});
    advance_claim(claim.as_object_mut().unwrap(), &turns);
    assert_eq!(
        claim,
        json!({"heldFloorMessageId": "a", "sinceMessageId": "b", "heldVersion": "v2"})
    );
}

#[test]
fn a_history_counter_change_is_not_suppressed_even_when_page_material_is_equal() {
    let page = json!({"messages":[{"messageId":"m0", "ordinal":0, "role":"user", "text":"same", "blockSummaries":[], "parts":[]}], "reachedStart":true});
    let reply =
        json!({"claim":"cursor_expired", "version":"history:1", "historyVersion":1, "resync":page});
    let mut emitted = TranscriptEmitted::default();
    emitted.record(&reply);
    assert!(!emitted.should_emit(&reply));
    let mut rewrite = reply;
    rewrite["historyVersion"] = json!(2);
    rewrite["version"] = json!("history:2");
    assert!(
        emitted.should_emit(&rewrite),
        "the client must replace its invalidated claim"
    );
}

#[test]
fn display_sequence_numbers_never_replay_and_a_gap_requires_a_fresh_baseline() {
    use occipital_app_reader::display::{
        DisplayChunk, DisplayPartType, DisplayReadReply, DisplayRing,
    };
    let mut ring = DisplayRing::new(2);
    for n in 0..3 {
        ring.push(
            DisplayChunk {
                session_id: "s".into(),
                message_id: "m0".into(),
                chunk: format!("{n}"),
                part_type: DisplayPartType::Text,
            }
            .into(),
        );
    }
    let baseline = ring.read(None);
    assert!(matches!(
        baseline,
        DisplayReadReply::Baseline { seq: 3, .. }
    ));
    assert!(matches!(
        ring.read(Some(0)),
        DisplayReadReply::Expired { seq: 3, .. }
    ));
    match ring.read(Some(1)) {
        DisplayReadReply::Incremental { seq, frames } => {
            assert_eq!(seq, 3);
            assert_eq!(
                frames.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
                [2, 3]
            );
        }
        other => panic!("{other:?}"),
    }
    match ring.read(Some(3)) {
        DisplayReadReply::Incremental { frames, .. } => assert!(frames.is_empty()),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        ring.read(None),
        baseline,
        "gap recovery must not consume the baseline"
    );
}

#[tokio::test]
async fn a_native_page_from_another_history_generation_never_vouches() {
    let source = FakeSource::new("broca", 2, 4, true);
    let reader = TranscriptReader::default();
    let page = read(&reader, &source, json!({"session":"s"})).await;
    source.append(1, 4);
    source.state.lock().unwrap().wrong_page_version = true;
    let reply = read(
        &reader,
        &source,
        json!({"session":"s", "claim":claim_for(&page)}),
    )
    .await;
    assert_eq!(reply["error"]["code"], "observation_unstable");
    assert!(reply.get("newTurns").is_none());
}

#[tokio::test(start_paused = true)]
async fn a_delta_above_the_transport_limit_still_resyncs_within_the_notice_bound() {
    let source = FakeSource::new("broca", 2, 4, true);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let mut running = Running::start_accepting(source.clone(), reader.clone(), claim_for(&page));
    assert_eq!(running.next().await["claim"], "matched");
    source.append(100, 64 * 1024);
    let polling = reader
        .read(Some(&*source), json!({"session":"s"}))
        .await
        .unwrap_err();
    assert_eq!(polling.code, "reply_too_large");
    let trimmed = running.next().await;
    assert_eq!(trimmed["claim"], "outside_window");
    assert!(serde_json::to_vec(&trimmed).unwrap().len() <= NOTICE_BOUND);
    assert!(!message_ids(&trimmed["resync"]).is_empty());
    running.end().await;
}

/// A session with no messages names why its newest page is empty. A claim holding
/// no messages (`heldVersion` alone) is answered `unchanged` until the first message,
/// then `cursor_expired` with the newest page, the answer an app already applies to a
/// replaced held page. The claim advanced from that answer holds the page and matches.
#[tokio::test]
async fn an_empty_claim_is_unchanged_until_the_first_message_replaces_the_empty_page() {
    for native in [true, false] {
        let source = FakeSource::new("broca", 0, 4, native);
        let reader = TranscriptReader::default();
        let page = read(&reader, &source, json!({"session":"s"})).await;
        assert_eq!(page["messages"], json!([]), "native {native}");
        assert_eq!(page["reachedStart"], true);
        assert_eq!(page["emptyReason"], "empty");
        let held = json!({"heldVersion": page["version"]});
        let unchanged = read(&reader, &source, json!({"session":"s", "claim":held})).await;
        assert_eq!(unchanged["unchanged"], true, "native {native}: {unchanged}");
        assert_eq!(unchanged["version"], page["version"]);
        assert!(unchanged.get("claim").is_none());
        source.append(2, 4);
        let replaced = read(&reader, &source, json!({"session":"s", "claim":held})).await;
        assert_eq!(
            replaced["claim"], "cursor_expired",
            "native {native}: {replaced}"
        );
        assert_eq!(message_ids(&replaced["resync"]), ["m0", "m1"]);
        // The phone refuses either member on this verdict.
        assert!(replaced.get("validThroughMessageId").is_none());
        assert!(replaced.get("vouchedFromMessageId").is_none());
        let mut advanced = held.as_object().unwrap().clone();
        advance_claim(&mut advanced, &replaced);
        source.append(1, 4);
        let matched = read(&reader, &source, json!({"session":"s", "claim":advanced})).await;
        assert_eq!(matched["claim"], "matched", "native {native}: {matched}");
        assert_eq!(
            message_ids(&json!({"messages": matched["newTurns"]})),
            ["m2"]
        );
    }
}

/// An empty page whose history was rewritten while it stayed empty is replaced by the
/// empty page at the new version, and the claim advanced from it is unchanged again.
#[tokio::test]
async fn an_empty_claim_at_an_older_version_gets_the_empty_page_again() {
    let source = FakeSource::new("broca", 0, 4, true);
    let reader = TranscriptReader::default();
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let held = json!({"heldVersion": page["version"]});
    source.state.lock().unwrap().history_version = Some(2);
    source.notify();
    let replaced = read(&reader, &source, json!({"session":"s", "claim":held})).await;
    assert_eq!(replaced["claim"], "cursor_expired", "{replaced}");
    assert_ne!(replaced["version"], page["version"]);
    assert_eq!(replaced["resync"]["messages"], json!([]));
    assert_eq!(replaced["resync"]["emptyReason"], "empty");
    let mut advanced = held.as_object().unwrap().clone();
    advance_claim(&mut advanced, &replaced);
    assert_eq!(
        Value::Object(advanced.clone()),
        json!({"heldVersion": replaced["version"]})
    );
    let unchanged = read(&reader, &source, json!({"session":"s", "claim":advanced})).await;
    assert_eq!(unchanged["unchanged"], true, "{unchanged}");
}

/// A watch holding no messages sends nothing while the session stays empty, even past
/// the slow sweep, then answers the first message as a replaced page and follows the
/// session from there.
#[tokio::test(start_paused = true)]
async fn an_empty_claim_watch_is_silent_until_the_first_message() {
    let source = FakeSource::new("broca", 0, 4, true);
    let reader = Arc::new(TranscriptReader::default());
    let page = read(&reader, &source, json!({"session":"s"})).await;
    let mut running = Running::start(
        source.clone(),
        reader.clone(),
        json!({"heldVersion": page["version"]}),
    );
    running
        .quiet(transcript_watch::TRANSCRIPT_SWEEP.as_millis() as u64 * 2)
        .await;
    assert!(running.host.evaluations.load(Ordering::SeqCst) >= 2);
    source.append(1, 4);
    let first = running.next().await;
    assert_eq!(first["claim"], "cursor_expired", "{first}");
    assert_eq!(message_ids(&first["resync"]), ["m0"]);
    source.append(1, 4);
    let next = running.next().await;
    assert_eq!(next["claim"], "matched", "{next}");
    assert_eq!(message_ids(&json!({"messages": next["newTurns"]})), ["m1"]);
    running.end().await;
}
