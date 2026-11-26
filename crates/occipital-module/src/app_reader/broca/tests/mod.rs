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
    Entorhinal, Network, NetworkChange, PI_RUNNER_MODULE_ID, THALAMUS_RUNNER_MODULE_ID,
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
    /// A runner refusal every read gets, as the runner's daemon call returns
    /// it, mapped the way a live read maps it ([`super::read_call_error`]).
    refusal: Mutex<Option<subc_protocol::ErrorBody>>,
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
    /// The files `session.media_read` serves, by digest: type and bytes.
    media: Mutex<HashMap<String, (String, Vec<u8>)>>,
    /// The `session.plan_view` answer, or its refusal; none refuses `no_plan`.
    plan_view: Mutex<Option<Result<Value, ReadError>>>,
    /// The runner build the catalog names, or `None` for unknown.
    build: Mutex<Option<String>>,
    /// The modules the catalog lists as fronting voice calls.
    voice_fronts: Mutex<HashSet<String>>,
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
    // A runner taking files serves the upload operations and the media read.
    if groups.contains(&"attachments") {
        for op in [
            "session.upload_begin",
            "session.upload_append",
            "session.upload_status",
            "session.upload_commit",
            "session.media_read",
            ops::SESSION_SEND,
            ops::SESSION_READ,
        ] {
            if !served.iter().any(|served| served == op) {
                served.push(op.into());
            }
        }
    }
    if groups.contains(&"plan_view") {
        served.push(ops::SESSION_PLAN_VIEW.into());
    }
    if groups.contains(&"send_promote") {
        served.push(ops::SESSION_PROMOTE.into());
    }
    if groups.contains(&"steer") && !served.iter().any(|op| op == ops::SESSION_SEND) {
        served.push(ops::SESSION_SEND.into());
    }
    let mut describe = serde_json::to_value(
        RoleDescribe::new(vec![Major::new(PROVIDES, served, "alpha")], "fake-broca")
            .with_capabilities(groups.iter().map(|group| group.to_string()).collect())
            .with_session_capabilities_from("baseline"),
    )
    .unwrap();
    // A runner declaring `transcript_reads` states its read size limits.
    if groups.contains(&"transcript_reads") {
        describe["max_bytes"] = json!({"default": 1_048_576, "maximum": 4_194_304});
    }
    describe
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
            refusal: Mutex::new(None),
            stuck: AtomicBool::new(false),
            updated_at: AtomicU64::new(730),
            run_state: Mutex::new(json!({"run_id":"r1", "state":"active"})),
            head_extra: Mutex::new(serde_json::Map::new()),
            describe: Mutex::new(Some(describe_with(&[]))),
            history_replies: Mutex::new(HashMap::new()),
            anchor_replies: Mutex::new(HashMap::new()),
            read_entries: AtomicUsize::new(0),
            media: Mutex::new(HashMap::new()),
            plan_view: Mutex::new(None),
            build: Mutex::new(None),
            voice_fronts: Mutex::new(HashSet::new()),
        })
    }

    /// Serve `bytes` of type `mime` from `session.media_read`, under their digest.
    fn hold_media(&self, mime: &str, bytes: &[u8]) -> String {
        let sha256 = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes));
        self.media
            .lock()
            .unwrap()
            .insert(sha256.clone(), (mime.into(), bytes.to_vec()));
        sha256
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
        ReadOp::MediaRead => serde_json::from_value::<
            cortexkit_role_llm_runner::attachments::MediaReadRequest,
        >(params.clone())
        .map(drop),
        ReadOp::PlanView => serde_json::from_value::<
            cortexkit_role_llm_runner::plan_view::PlanViewRequest,
        >(params.clone())
        .map(drop),
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
    async fn runner_build(&self) -> Option<String> {
        self.build.lock().unwrap().clone()
    }

    async fn voice_fronts(&self) -> HashSet<String> {
        self.voice_fronts.lock().unwrap().clone()
    }

    async fn read(&self, op: ReadOp, params: Value) -> Result<Value, ReadError> {
        let request: Value = serde_json::from_slice(&body(op, params.clone())).unwrap();
        runner_accepts(op, &params);
        self.calls.lock().unwrap().push(request.clone());
        if self.unavailable.load(Ordering::Relaxed) {
            return Err(ReadError::new("provider_unavailable", "Broca route lost"));
        }
        if let Some(body) = self.refusal.lock().unwrap().clone() {
            return Err(super::read_call_error(
                op,
                BROCA_RUNNER_MODULE_ID,
                CallError::Module(body),
            ));
        }
        match request["method"].as_str().unwrap() {
            "session.head" => {
                let (first, last) = {
                    let entries = self.entries.lock().unwrap();
                    let mut ordinals = entries.iter().filter_map(|entry| entry["ordinal"].as_u64());
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
            "role.describe" => self
                .describe
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| ReadError::new("provider_unavailable", "role.describe failed")),
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
            "session.media_read" => {
                let media = self.media.lock().unwrap();
                let Some((mime, bytes)) = params["sha256"]
                    .as_str()
                    .and_then(|sha256| media.get(sha256))
                else {
                    return Err(ReadError::new("unknown_media", "no such media"));
                };
                let offset = params["offset"].as_u64().unwrap() as usize;
                let length = params["length"].as_u64().unwrap() as usize;
                let mut reply = json!({"size": bytes.len(), "mime": mime});
                if length > 0 {
                    let end = (offset + length).min(bytes.len());
                    reply["data"] = json!(crate::base64::encode(&bytes[offset..end]));
                }
                Ok(reply)
            }
            "session.plan_view" => self
                .plan_view
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| Err(ReadError::new("no_plan", "no plan is applied"))),
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
    assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Restart);
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
    assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Restart);
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
    assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Restart);
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
    assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Restart);
    // The seed is sent before the feed task starts, so it is already queued.
    assert_eq!(
        lease.events.try_recv().ok(),
        Some(DisplayInput::RunStart {
            rich_ui: Some(json!({"dialect":"1.0", "catalogDigest":"a".repeat(64)})),
            run_id: Some("r1".into())
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
    assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Restart);
    assert!(lease.events.try_recv().is_err());
}

/// Thalamus's recorded `run_finished` for a send it never handed to its
/// host (`tests/data/thalamus-send-not-delivered-1116/06-…`, copied
/// unchanged) ends that run's live view and wakes the chat's watches, like
/// any run's end.
#[tokio::test]
async fn a_recorded_end_of_a_run_never_handed_over_ends_its_live_view() {
    let route = FakeRoute::new();
    let source = BrocaSource::new(resolved(), route.clone());
    let mut lease = source.acquire().await.unwrap();
    lease.events.recv().await.unwrap();
    let mut changes = source.changes();
    let event: Value = serde_json::from_slice(
        &std::fs::read(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "tests/data/thalamus-send-not-delivered-1116/06-run-ended-event-handover-failed.json",
        ))
        .unwrap(),
    )
    .unwrap();
    route.emit(event).await;
    assert_eq!(
        lease.events.recv().await.unwrap(),
        DisplayInput::TurnEnd {
            run_id: Some("run_ab7fd675779cf1c22289bc02".into())
        }
    );
    changes.changed().await.unwrap();
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
    assert_eq!(
        lease.events.recv().await.unwrap(),
        DisplayInput::TurnEnd {
            run_id: Some("r1".into())
        }
    );
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
        DisplayInput::RunStart {
            rich_ui: None,
            run_id: Some("r2".into())
        }
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
    assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Restart);
    route.emit(json!({"kind":"display", "event":[]})).await;
    assert_eq!(lease.events.recv().await.unwrap(), DisplayInput::Restart);
    assert!(!source.activity_feed_live());
    assert_eq!(route.detaches.load(Ordering::Relaxed), 1);
}

struct Lookup;
#[async_trait]
impl Entorhinal for Lookup {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        // A session named under another harness: Entorhinal knows none here.
        if method == "agent.session_for_harness" {
            return Ok(json!({"code":"not_found", "detail":null, "message":"no agent session"}));
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
        let read = match SourceResolver::resolve(&sources, "ses_head", Some("opencode2")).await {
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
        (THALAMUS_RUNNER_MODULE_ID, "opencode2"),
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

/// Entorhinal recording session `ses_head` as a session of `harness` on `runner`.
struct Hosts(&'static str, &'static str);
#[async_trait]
impl Entorhinal for Hosts {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ReadError> {
        let Self(runner, harness) = *self;
        match method {
            "agent.session_for_harness" => {
                assert_eq!(params, json!({"harness": harness, "session":"ses_head"}))
            }
            "agent.session_for_runner" => {
                assert_eq!(params, json!({"runner": runner, "session":"ses_head"}))
            }
            other => panic!("unexpected Entorhinal call {other}"),
        }
        Ok(
            json!({"result":{"agent_id":"agent-test", "ref_kind":"alfonso", "ref":"test-ref",
                "incarnation":"entorhinal-test", "ended_at_ms":null, "runner":runner, "session":"ses_head",
                "harness":harness, "project_root":"/workspace/head", "network":"unknown", "network_history":[]}}),
        )
    }
}

/// Thalamus's runner hosts OpenCode 2, Claude Code and Pi sessions, so each
/// harness on it is read, as Pi is on the Pi runner until Pi sessions move; a
/// harness recorded on a runner that doesn't host it is refused, rather than
/// read in another harness's message shape.
#[tokio::test]
async fn a_runner_hosting_several_harnesses_serves_each_and_only_those() {
    for (runner, harness) in [
        (THALAMUS_RUNNER_MODULE_ID, "opencode2"),
        (THALAMUS_RUNNER_MODULE_ID, "claude-code"),
        (THALAMUS_RUNNER_MODULE_ID, "pi"),
        (PI_RUNNER_MODULE_ID, "pi"),
    ] {
        let sources = BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(Hosts(runner, harness)))),
            Arc::new(Routes(FakeRoute::new())),
        );
        sources
            .validate_session("ses_head", Some(harness))
            .await
            .unwrap_or_else(|error| panic!("{harness} on {runner}: {error:?}"));
    }
    let sources = BrocaSources::new(
        Arc::new(Resolver::new(Arc::new(Hosts(
            PI_RUNNER_MODULE_ID,
            "claude-code",
        )))),
        Arc::new(Routes(FakeRoute::new())),
    );
    let refused = sources
        .validate_session("ses_head", Some("claude-code"))
        .await
        .unwrap_err();
    assert_eq!(refused.code, "invalid_provider_reply", "{refused:?}");
    assert!(refused.message.contains("not a pi session"), "{refused:?}");
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
            THALAMUS_RUNNER_MODULE_ID,
        ))))),
        routes.clone(),
    );
    let target = ours
        .send_target("ses_head", Some("opencode2"))
        .await
        .unwrap();
    assert_eq!(target.reference().runner, THALAMUS_RUNNER_MODULE_ID);
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
    assert!(app
        .display
        .runner("broca")
        .projection("alfonso:s1")
        .is_none());
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
    request["claim"] =
        json!({"heldFloorMessageId":"m20", "sinceMessageId":"m22", "heldVersion":page["version"]});
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
        params["claim"] =
            json!({"heldFloorMessageId":"m20", "sinceMessageId":"m22", "heldVersion":"history:7"});
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
            if frame.header.corr == corr && frame.header.ty == subc_protocol::FrameType::StreamEnd {
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
    assert!(app
        .display
        .runner("broca")
        .projection("alfonso:s1")
        .is_some());
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
    assert!(app
        .display
        .runner("broca")
        .projection("alfonso:s1")
        .is_none());

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
    assert!(hub.runner("broca").projection("alfonso:s1").is_some());
    assert_eq!(
        *routes.1.lock().unwrap(),
        vec![
            std::path::PathBuf::from("/workspace/resolved-project"),
            std::path::PathBuf::from("/workspace/moved")
        ]
    );
}

/// A runner listing the turn-scoped system form of runner contract 0.8.29
/// (`turn_system_cleared`), or a delivery mode no contract names yet, still
/// passes the describe check. Before 0.8.30 the check refused any mode it
/// didn't know, and the reader then treated the runner as describing
/// nothing: no media references, no notices and an unknown send grant for
/// all its chats.
#[test]
fn a_runner_listing_a_newer_delivery_mode_is_still_described() {
    let mut value = describe_with(&[
        "attachments",
        "transcript_reads",
        "media_refs",
        "delivery_order",
        "steer",
        "queue",
    ]);
    for modes in [
        json!(["turn_system_cleared", "tool_result", "steer", "idle_start"]),
        json!(["a_mode_from_a_later_contract", "steer", "idle_start"]),
    ] {
        value["delivery_order_modes"] = modes;
        let describe = check_describe(&value).unwrap();
        assert!(stored_media(Some(&describe)).0.is_some());
    }
}

mod aft_notice_1120;
mod app_readers_1112;
mod bounded_reads;
mod cache_write_split_1113;
mod call_pins;
mod claude_code_pages;
mod claude_code_samples;
mod client_message_ids;
mod contract_0085_samples;
mod contract_0748_samples;
mod contract_0811_samples;
mod contract_0821_writer_samples;
mod contract_096_samples;
mod contract_097_samples;
mod contract_goldens;
mod contract_heads;
mod credit_window_1111;
mod describe_samples;
mod describe_step_events_samples;
mod empty_claim_samples;
mod exact_read_samples;
mod files_samples;
mod fork_answer_1112;
mod head_provider_code_1112;
mod head_selection_samples;
mod host_refused_samples;
mod imported_heads_1110;
mod input_cap_withdrawn_1116;
mod model_fallback_1113;
mod moved_calls;
mod not_delivered_samples;
pub(crate) mod not_delivered_step2_1116;
mod notice_records_samples;
mod notice_writer_1107;
mod open_transient_1113;
mod queued_sends_samples;
mod read_moved_samples;
mod read_role_samples;
mod request_context;
mod request_context_samples;
mod run_awaits_1112;
mod run_provider_code_1113;
mod runner_0842_samples;
mod send_held_samples;
mod send_ids_1110;
mod send_moved_samples;
mod send_promote_1116;
mod send_released_samples;
mod session_converting_1115;
mod signed_reasoning_cause_1112;
mod speaks_for_name_1109;
mod temperature_0844_1112;
mod text_upload_1108;
mod thalamus_096_samples;
mod tool_output_samples;
mod unreadable_samples;
mod wake_records_samples;

/// The chat's activity follows the llm-runner role crate's `RunState`: a
/// running or paused run keeps it busy, every terminal state leaves it idle,
/// and a state the crate doesn't know says nothing.
#[test]
fn activity_follows_the_contracts_run_states() {
    for (state, activity) in [
        ("active", Some("busy")),
        ("paused", Some("busy")),
        ("completed", Some("idle")),
        ("max_steps", Some("idle")),
        ("budget_exhausted", Some("idle")),
        ("context_ceiling", Some("idle")),
        ("cancelled", Some("idle")),
        ("error", Some("idle")),
        ("interrupted", Some("idle")),
        ("future_state", None),
    ] {
        assert_eq!(activity_for(Some(state)), activity, "{state}");
    }
    assert_eq!(activity_for(None), None);
}
mod capacity_refusal_1125b;
mod capacity_wait_1122;
mod history_intake_samples;
mod host_parity_wave1;
mod host_parity_wave3;
mod host_parts;
mod host_text_1123;
mod host_unknown_samples;
mod in_place_rows;
mod model_names_samples;
mod notice_keep_1122;
mod notices_samples;
mod opencode2_live_samples;
mod opencode2_pages;
mod outside_window;
mod part_keys;
mod pi_display;
mod pi_live_steps_samples;
mod pi_pages;
mod pi_photo_1118;
mod pi_samples;
mod plan_notes_1124;
mod relayed_samples;
mod reply_tags_1112;
mod run_endings_samples;
#[path = "../../../../../occipital-app-reader/tests/fixtures/broca_0_7.rs"]
pub(crate) mod samples_0_7;
mod say_live_samples;
mod say_samples;
pub(crate) mod served_samples;
mod session_facts;
mod session_retention_1123;
mod simple_samples;
mod tag_rows;
mod tool_display_samples;
mod tool_times_samples;
mod tool_versions_samples;

#[tokio::test]
async fn losing_one_runner_does_not_remove_another_runners_local_session() {
    let hub = Arc::new(DisplayHub::default());
    let a = BrocaSource::new(resolved(), FakeRoute::new());
    let mut other = resolved();
    other.reference.runner = PI_RUNNER_MODULE_ID.into();
    other.harness = "pi".into();
    let b = BrocaSource::new(other, FakeRoute::new());
    let _a = a.acquire_inner(Some(("same", &hub))).await.unwrap();
    let _b = b.acquire_inner(Some(("same", &hub))).await.unwrap();
    let first = hub.runner("broca").projection("same").unwrap();
    let second = hub.runner(PI_RUNNER_MODULE_ID).projection("same").unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    a.lost();
    assert!(hub.runner("broca").projection("same").is_none());
    assert!(hub.runner(PI_RUNNER_MODULE_ID).projection("same").is_some());
}

/// Serve capability refusals against the same chat and rows as the message-tool samples.
pub(crate) fn recovery_chat(
    oversized_plan: bool,
) -> (Arc<crate::app_reader::AppReader>, &'static str) {
    let (app, route) = say_samples::chat(true);
    if oversized_plan {
        *route.plan_view.guard() = Some(Err(ReadError::new(
            "too_large",
            "plan exceeds the runner's reply limit",
        )));
    } else {
        route.describe.guard().take();
    }
    (app, say_samples::SESSION)
}

#[tokio::test]
async fn an_entirely_hidden_numbered_page_continues_at_its_raw_boundary() {
    use crate::app_reader::{tests::serve, TRANSCRIPT_PAGE};
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = vec![
        json!({"mid":"m1","ordinal":1,"message":{"role":"user","content":["visible"]}}),
        json!({"mid":"m2","ordinal":2,"delivered_as":{"mode":"tool_result"},"message":{"role":"user","content":["ride"]}}),
    ];
    let (_, app) = serving(route);
    let mut params = json!({"session":"alfonso:s1","limit":1});
    let first: Value =
        serde_json::from_slice(&serve(&app, TRANSCRIPT_PAGE, &params).await).unwrap();
    let first = &first["result"];
    assert_eq!(first["messages"], json!([]));
    assert!(first.get("reachedStart").is_none(), "{first}");
    if let Some(cursor) = first.get("nextBeforeCursor") {
        params["beforeCursor"] = cursor.clone();
    } else {
        params["beforeOrdinal"] = first["nextBeforeOrdinal"].clone();
    }
    let next: Value = serde_json::from_slice(&serve(&app, TRANSCRIPT_PAGE, &params).await).unwrap();
    assert_eq!(next["result"]["messages"][0]["messageId"], "m1", "{next}");
    assert_eq!(next["result"]["reachedStart"], true);
}
