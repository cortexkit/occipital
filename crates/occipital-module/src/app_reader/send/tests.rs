//! The send's routing, receipts and follow-up against a runner that keeps Broca's
//! rules: a plan names this module as a sender, a head's entry grants `idle_start`
//! and `interrupt`, a worker's grants neither, and a list without `idle_start`
//! waits for a running turn until it expires.

use super::*;
use occipital_app_reader::source::ReadError;

mod mode_samples;
mod prompt;
mod stop_all_samples;
use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicBool, AtomicI64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Head,
    Worker,
}

struct FakeRunner {
    kind: Kind,
    /// Whether the session's frozen plan names this module at all.
    named: AtomicBool,
    sender: Mutex<Option<Value>>,
    head_sender: AtomicBool,
    /// Broca validates a new run's model before checking sender permissions.
    require_model: AtomicBool,
    running: AtomicBool,
    run_id: Mutex<String>,
    retractable: AtomicBool,
    /// A retract racing with a boundary: reply and resulting durable send state.
    retract_race: Mutex<Option<(Result<Value, RunnerError>, String)>>,
    /// Whether a steer joins the running turn at once, instead of waiting.
    lands: AtomicBool,
    unreachable: AtomicBool,
    calls: Mutex<Vec<(String, Value)>>,
    sends: Mutex<HashMap<String, (Value, String)>>,
    scripted: Mutex<VecDeque<Result<Value, RunnerError>>>,
    captured: Mutex<HashMap<&'static str, Value>>,
}

impl FakeRunner {
    fn new(kind: Kind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            named: AtomicBool::new(true),
            sender: Mutex::default(),
            head_sender: AtomicBool::new(false),
            require_model: AtomicBool::new(false),
            running: AtomicBool::new(true),
            run_id: Mutex::new("r1".into()),
            retractable: AtomicBool::new(true),
            retract_race: Mutex::default(),
            lands: AtomicBool::new(false),
            unreachable: AtomicBool::new(false),
            calls: Mutex::default(),
            sends: Mutex::default(),
            scripted: Mutex::default(),
            captured: Mutex::default(),
        })
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    /// Bodies the runner recorded: sends it accepted, not ones it refused.
    fn written(&self) -> Vec<Value> {
        self.sends
            .lock()
            .unwrap()
            .values()
            .map(|(body, _)| body.clone())
            .collect()
    }

    /// The run ended before any step took the held steers: each expires.
    fn end_run(&self) {
        self.running.store(false, Ordering::SeqCst);
        for (_, state) in self.sends.lock().unwrap().values_mut() {
            if state == "pending" {
                *state = "expired".into();
            }
        }
    }

    fn refuse(code: &str, field: Option<&str>) -> Result<Value, RunnerError> {
        Err(RunnerError::Refused {
            code: code.into(),
            message: format!("refused {code}"),
            detail: field.map(|field| json!({"field": field})),
        })
    }

    /// `session.send_status`: the send's durable fate, read without acting.
    fn status(&self, params: &Value) -> Result<Value, RunnerError> {
        let sends = self.sends.lock().unwrap();
        let Some((_, state)) = sends.get(params["send_id"].as_str().unwrap()) else {
            return Self::refuse("unknown_send", None);
        };
        Ok(match state.as_str() {
            "delivered" => {
                json!({"state": "active", "run_id": "r1", "delivered": {"as": "step", "ref": "row-1"}})
            }
            state => json!({"state": state, "submission_id": "sub-1"}),
        })
    }

    fn set_state(&self, send_id: &Value, state: &str) {
        self.sends
            .lock()
            .unwrap()
            .get_mut(send_id.as_str().unwrap())
            .unwrap()
            .1 = state.into();
    }

    fn retract(&self, params: &Value) -> Result<Value, RunnerError> {
        assert_eq!(params, &json!({"submission_id": "sub-1"}));
        let (reply, state) = self
            .retract_race
            .lock()
            .unwrap()
            .take()
            .unwrap_or((Ok(json!("retracted")), "retracted".into()));
        for (_, pending) in self.sends.lock().unwrap().values_mut() {
            if pending == "pending" {
                *pending = state.clone();
            }
        }
        reply
    }

    fn send(&self, body: &Value) -> Result<Value, RunnerError> {
        let delivery = &body["delivery"];
        let starts = delivery
            .as_array()
            .is_some_and(|modes| modes.iter().any(|mode| mode == "idle_start"));
        if self.require_model.load(Ordering::SeqCst) && starts && body.get("model").is_none() {
            return Err(RunnerError::Refused {
                code: "invalid_params".into(),
                message: "model is required unless a joining delivery can inherit it from an existing episode".into(),
                detail: Some(json!({"field": "model"})),
            });
        }
        if !self.named.load(Ordering::SeqCst) {
            return Self::refuse("scope_owner_mismatch", None);
        }
        if self.kind == Kind::Worker && (starts || delivery == "interrupt") {
            return Self::refuse("invalid_params", Some("delivery"));
        }
        let send_id = body["send_id"].as_str().unwrap().to_owned();
        let mut sends = self.sends.lock().unwrap();
        if let Some((recorded, state)) = sends.get(&send_id) {
            if recorded != body {
                return Self::refuse("send_id_reuse", Some("delivery"));
            }
            return Ok(match state.as_str() {
                "delivered" => {
                    json!({"state": "active", "run_id": "r1", "delivered": {"as": "step", "ref": "row-1"}})
                }
                state => json!({"state": state, "submission_id": "sub-1"}),
            });
        }
        let steer_only = delivery
            .as_array()
            .is_some_and(|modes| modes == &[json!("steer")]);
        let (state, reply) = if steer_only && !self.lands.load(Ordering::SeqCst) {
            (
                "pending",
                json!({"state": "pending", "submission_id": "sub-1"}),
            )
        } else if steer_only {
            (
                "delivered",
                json!({"state": "active", "run_id": "r1", "delivered": {"as": "step", "ref": "row-1"}}),
            )
        } else {
            ("active", json!({"state": "active", "run_id": "r1"}))
        };
        sends.insert(send_id, (body.clone(), state.into()));
        Ok(reply)
    }
}

struct Target(Arc<FakeRunner>, SessionRef);

#[async_trait]
impl SendTarget for Target {
    fn reference(&self) -> &SessionRef {
        &self.1
    }
    async fn call(&self, method: &'static str, params: Value) -> Result<Value, RunnerError> {
        self.0
            .calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        if method == "session.send" {
            if let Some(scripted) = self.0.scripted.lock().unwrap().pop_front() {
                return scripted;
            }
        }
        if self.0.unreachable.load(Ordering::SeqCst) {
            return Err(RunnerError::Unreachable("route lost".into()));
        }
        if let Some(reply) = self.0.captured.lock().unwrap().get(method) {
            return crate::app_reader::broca::runner_result(reply.clone());
        }
        match method {
            "session.head" => {
                let mut head = json!({
                "incarnation": "inc-a", "first_ordinal": 1, "history_version": 1, "updated_at": 1,
                "last_run_state": {"run_id": *self.0.run_id.lock().unwrap(), "state": if self.0.running.load(Ordering::SeqCst) { "active" } else { "completed" }},
                });
                if let Some(sender) = self.0.sender.lock().unwrap().as_ref() {
                    head["sender"] = sender.clone();
                }
                Ok(head)
            }
            "role.describe" => {
                let mut groups = Vec::new();
                if self.0.head_sender.load(Ordering::SeqCst) {
                    groups.push("head_sender".into());
                }
                if self.0.retractable.load(Ordering::SeqCst) {
                    groups.push("retract".into());
                }
                Ok(
                    serde_json::to_value(RoleDescribe::default().with_capabilities(groups))
                        .unwrap(),
                )
            }
            "session.send" => self.0.send(&params),
            "session.send_status" => self.0.status(&params),
            "session.retract" => self.0.retract(&params),
            other => panic!("the sender called {other}"),
        }
    }
}

struct Targets {
    runner: Arc<FakeRunner>,
    opened: AtomicI64,
}

#[async_trait]
impl SendTargets for Targets {
    async fn open(&self, session: &str, _: Option<&str>) -> Result<Arc<dyn SendTarget>, ReadError> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        match session {
            "missing" => Err(ReadError::new(
                "session_unknown",
                "no session missing on runner broca",
            )),
            "pi-session" => Err(ReadError::new(
                "invalid_provider_reply",
                "session pi-session is not a Broca session",
            )),
            "offline" => Err(ReadError::new(
                "provider_unavailable",
                "entorhinal: connection refused",
            )),
            _ => Ok(Arc::new(Target(
                self.runner.clone(),
                SessionRef {
                    runner: "broca".into(),
                    session: session.into(),
                },
            ))),
        }
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    runner: Arc<FakeRunner>,
    targets: Arc<Targets>,
    receipts: Arc<Receipts>,
    now: Arc<AtomicI64>,
    sender: Arc<Sender>,
}

impl Fixture {
    /// The module restarting: a new sender over the same store and runner. The old
    /// one is dropped, and its follow-up tasks with it.
    fn restart(&mut self) {
        let clock = self.now.clone();
        let restarted = Arc::new(Sender::with_clock(
            self.targets.clone(),
            Some(self.receipts.clone()),
            Arc::new(move || clock.load(Ordering::SeqCst)),
        ));
        drop(std::mem::replace(&mut self.sender, restarted));
    }
}

const START: i64 = 1_767_225_600_000;

fn fixture(kind: Kind) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::tests::shared(&dir.path().join("store.db"));
    let receipts = Receipts::open(store).unwrap();
    let runner = FakeRunner::new(kind);
    let targets = Arc::new(Targets {
        runner: runner.clone(),
        opened: AtomicI64::new(0),
    });
    let now = Arc::new(AtomicI64::new(START));
    let clock = now.clone();
    let sender = Arc::new(Sender::with_clock(
        targets.clone(),
        Some(receipts.clone()),
        Arc::new(move || clock.load(Ordering::SeqCst)),
    ));
    Fixture {
        _dir: dir,
        runner,
        targets,
        receipts,
        now,
        sender,
    }
}

fn message(id: &str, text: &str) -> Value {
    json!({"session": "s", "text": text, "clientMessageId": id})
}

fn granted_fixture(kind: Kind) -> Fixture {
    let f = fixture(kind);
    *f.runner.sender.lock().unwrap() = Some(match kind {
        Kind::Head => json!({"modes": ["steer", "idle_start"], "interrupt": true}),
        Kind::Worker => json!({"modes": ["tool_result", "steer"], "interrupt": false}),
    });
    // Head fixtures have a default model; workers must inherit one by joining.
    f.runner
        .require_model
        .store(kind == Kind::Worker, Ordering::SeqCst);
    f
}

fn call_counts(runner: &FakeRunner) -> (usize, usize, usize) {
    let calls = runner.calls();
    let count = |name| calls.iter().filter(|(method, _)| method == name).count();
    (
        count("session.head"),
        count("session.send"),
        count("role.describe"),
    )
}

#[tokio::test]
async fn a_granted_worker_steers_even_when_model_validation_precedes_the_grant() {
    let f = granted_fixture(Kind::Worker);
    f.runner.lands.store(true, Ordering::SeqCst);
    let receipt = f
        .sender
        .send(json!({
            "session": "s", "harness": "broca", "clientMessageId": "phone-1", "text": "hello",
            "ambient": {"page": "phone"}
        }))
        .await
        .unwrap();
    assert_eq!(receipt["ok"], true);
    assert_eq!(sends(&f.runner)[0]["delivery"], json!(["steer"]));
    assert_eq!(call_counts(&f.runner), (1, 1, 0));
}

#[tokio::test]
async fn a_granted_worker_reuses_the_head_read_and_a_repeat_calls_nothing() {
    let f = granted_fixture(Kind::Worker);
    f.runner.lands.store(true, Ordering::SeqCst);
    let first = f.sender.send(message("w1", "hello")).await.unwrap();
    assert_eq!(first["ok"], true);
    assert_eq!(sends(&f.runner)[0]["delivery"], json!(["steer"]));
    assert_eq!(call_counts(&f.runner), (1, 1, 0));
    let before = f.runner.calls();
    let repeat = f.sender.send(message("w1", "hello")).await.unwrap();
    assert_eq!(repeat["effectId"], first["effectId"]);
    assert_eq!(repeat["duplicate"], true);
    assert_eq!(f.runner.calls(), before);
    assert_eq!(f.targets.opened.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_granted_idle_worker_is_paused_without_a_send() {
    let f = granted_fixture(Kind::Worker);
    f.runner.running.store(false, Ordering::SeqCst);
    assert_eq!(
        f.sender
            .send(message("w1", "hello"))
            .await
            .unwrap_err()
            .code,
        "task_paused"
    );
    assert_eq!(call_counts(&f.runner), (1, 0, 0));
}

/// A held worker steer is followed across restarts from a row that names no harness,
/// so a worker on another harness is refused before anything is sent or recorded.
#[tokio::test]
async fn a_worker_on_another_harness_is_refused_before_a_send() {
    let f = granted_fixture(Kind::Worker);
    let mut request = message("w1", "hello");
    request["harness"] = json!("opencode2");
    let refusal = f.sender.send(request).await.unwrap_err();
    assert_eq!(refusal.code, "unsupported");
    assert_eq!(call_counts(&f.runner).1, 0, "nothing was sent");
    assert!(f.receipts.get("worker", "w1", START).unwrap().is_none());
}

#[tokio::test]
async fn granted_interrupts_require_a_head_and_its_interrupt_permission() {
    for kind in [Kind::Head, Kind::Worker] {
        let f = granted_fixture(kind);
        f.runner.sender.lock().unwrap().as_mut().unwrap()["interrupt"] =
            json!(kind == Kind::Worker);
        assert_eq!(
            f.sender
                .send(json!({
                    "session": "s", "text": "stop", "mode": "interrupt"
                }))
                .await
                .unwrap_err()
                .code,
            "enqueue_not_authorized"
        );
        assert_eq!(call_counts(&f.runner), (1, 0, 0));
    }
}

#[tokio::test]
async fn a_declared_missing_sender_grant_refuses_without_a_send() {
    let f = fixture(Kind::Worker);
    f.runner.head_sender.store(true, Ordering::SeqCst);
    f.runner.require_model.store(true, Ordering::SeqCst);
    assert_eq!(
        f.sender
            .send(message("w1", "hello"))
            .await
            .unwrap_err()
            .code,
        "enqueue_not_authorized"
    );
    assert_eq!(call_counts(&f.runner), (1, 0, 1));
}

#[tokio::test]
async fn an_unknown_grant_keeps_the_trial_path_including_model_first_refusals() {
    let f = fixture(Kind::Worker);
    f.runner.lands.store(true, Ordering::SeqCst);
    f.runner.require_model.store(true, Ordering::SeqCst);
    let refusal = f.sender.send(message("w1", "hello")).await.unwrap_err();
    assert_eq!(refusal.code, "runner_refused");
    assert!(refusal.message.contains("model is required"));
    assert_eq!(call_counts(&f.runner), (1, 1, 1));
    f.runner.require_model.store(false, Ordering::SeqCst);
    let sent = f.sender.send(message("w1", "hello")).await.unwrap();
    assert_eq!(sent["ok"], true);
    assert_eq!(
        sends(&f.runner).last().unwrap()["delivery"],
        json!(["steer"])
    );
    assert_eq!(call_counts(&f.runner), (2, 3, 2));
}

fn sends(runner: &FakeRunner) -> Vec<Value> {
    runner
        .calls()
        .into_iter()
        .filter(|(method, _)| method == "session.send")
        .map(|(_, body)| body)
        .collect()
}

/// Wait for a follow-up task, whose store writes run on the blocking pool.
async fn eventually(mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the follow-up never settled");
}

#[tokio::test]
async fn malformed_and_attachment_sends_are_refused_before_anything_is_resolved() {
    let f = fixture(Kind::Head);
    for (params, code) in [
        (json!({"session": "s"}), "invalid_request"),
        (
            json!({"session": "s", "text": "t", "clientMessageId": "bad id"}),
            "invalid_client_message_id",
        ),
        (
            json!({"session": "s", "text": "t", "send_id": "a/b"}),
            "invalid_send_id",
        ),
        (
            json!({"session": "s", "text": "t", "mode": "now"}),
            "invalid_request",
        ),
        // This sender has no upload storage, so no file can be sent.
        (
            json!({"session": "s", "text": "look", "clientMessageId": "m1",
                "parts": [{"type": "file", "mime": "image/png", "filename": "a.png", "attachment_id": "up-1"}]}),
            "provider_unavailable",
        ),
    ] {
        assert_eq!(
            f.sender.send(params.clone()).await.unwrap_err().code,
            code,
            "{params}"
        );
    }
    assert_eq!(f.targets.opened.load(Ordering::SeqCst), 0);
    assert!(f.runner.calls().is_empty());
}

#[tokio::test]
async fn unknown_and_unserved_sessions_are_refused_and_an_unreachable_lookup_is_safe_to_resend() {
    let f = fixture(Kind::Head);
    let missing = f
        .sender
        .send(json!({"session": "missing", "text": "t", "clientMessageId": "m1"}))
        .await
        .unwrap_err();
    assert_eq!(missing.code, "not_found");
    let other = f
        .sender
        .send(json!({"session": "pi-session", "text": "t", "clientMessageId": "m2"}))
        .await
        .unwrap_err();
    assert_eq!(other.code, "provider_unavailable");
    let offline = f
        .sender
        .send(json!({"session": "offline", "text": "t", "clientMessageId": "m3"}))
        .await
        .unwrap();
    assert_eq!(
        (
            offline["ok"].clone(),
            offline["code"].clone(),
            offline["terminal"].clone()
        ),
        (json!(false), json!("not_sent"), json!(false))
    );
    assert!(f.runner.calls().is_empty());
    // Nothing was recorded, so the same id may be sent again once the lookup works.
    assert!(f.receipts.get("offline", "m3", START).unwrap().is_none());
}

#[tokio::test]
async fn head_modes_map_onto_the_plan_granted_deliveries() {
    let f = granted_fixture(Kind::Head);
    for (id, mode, delivery) in [
        ("q", None, json!(["idle_start"])),
        ("q2", Some("queue"), json!(["idle_start"])),
        ("st", Some("steer"), json!(["steer", "idle_start"])),
        ("int", Some("interrupt"), json!("interrupt")),
    ] {
        let mut params = message(id, "hello");
        if let Some(mode) = mode {
            params["mode"] = json!(mode);
        }
        let receipt = f.sender.send(params).await.unwrap();
        assert_eq!(receipt["ok"], true, "{id}");
        assert_eq!(receipt["duplicate"], false);
        let body = sends(&f.runner).pop().unwrap();
        assert_eq!(body["delivery"], delivery, "{id}");
        assert_eq!(body["prompt"], "hello");
        assert_eq!(body["client_message_id"], id);
        assert_eq!(body["send_id"], receipt["effectId"]);
        assert!(
            body.get("expires_at_ms").is_none(),
            "a head send never expires"
        );
    }
    assert_eq!(call_counts(&f.runner), (4, 4, 0));
    assert_eq!(f.runner.written().len(), 4);
}

#[tokio::test]
async fn a_worker_takes_only_a_steer_into_its_running_turn_with_a_backstop_expiry() {
    let f = fixture(Kind::Worker);
    let receipt = f
        .sender
        .send(message("w1", "also check the logs"))
        .await
        .unwrap();
    assert_eq!(receipt["ok"], true);
    let calls = f.runner.calls();
    let methods: Vec<_> = calls.iter().map(|(method, _)| method.as_str()).collect();
    assert_eq!(
        methods,
        [
            "session.head",
            "role.describe",
            "session.send",
            "session.send"
        ]
    );
    // The head delivery is refused for a worker and writes nothing.
    assert_eq!(calls[2].1["delivery"], json!(["idle_start"]));
    let steer = &calls[3].1;
    assert_eq!(steer["delivery"], json!(["steer"]));
    assert_eq!(steer["expires_at_ms"], START + 30 * 60_000);
    assert_eq!(steer["send_id"], receipt["effectId"]);
    assert_ne!(steer["send_id"], calls[2].1["send_id"]);
    assert_eq!(f.runner.written(), vec![steer.clone()]);
    // An explicit steer is the same delivery; an interrupt stays core's.
    let steered = f
        .sender
        .send(json!({"session": "s", "text": "t", "clientMessageId": "w2", "mode": "steer"}))
        .await
        .unwrap();
    assert_eq!(steered["ok"], true);
    assert_eq!(
        sends(&f.runner).pop().unwrap()["delivery"],
        json!(["steer"])
    );
    let before = f.runner.written().len();
    let interrupt = f
        .sender
        .send(json!({"session": "s", "text": "t", "clientMessageId": "w3", "mode": "interrupt"}))
        .await
        .unwrap_err();
    assert_eq!(interrupt.code, "enqueue_not_authorized");
    assert!(interrupt.message.contains("invalid_params"));
    assert_eq!(f.runner.written().len(), before);
}

#[tokio::test]
async fn a_worker_without_a_running_turn_is_refused_at_once_and_nothing_is_written() {
    let f = fixture(Kind::Worker);
    f.runner.running.store(false, Ordering::SeqCst);
    let refusal = f.sender.send(message("w1", "hello")).await.unwrap_err();
    assert_eq!(refusal.code, "task_paused");
    assert!(f.runner.written().is_empty());
    let methods: Vec<_> = f
        .runner
        .calls()
        .into_iter()
        .map(|(method, _)| method)
        .collect();
    assert_eq!(methods, ["session.head", "role.describe", "session.send"]);
    // A refusal is not a receipt: the same id may be sent once the worker runs.
    assert!(f.receipts.get("s", "w1", START).unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_steer_the_run_never_takes_expires_and_the_app_is_told_late() {
    let f = fixture(Kind::Worker);
    let mut late = f.sender.late_outcomes();
    let receipt = f.sender.send(message("w1", "hello")).await.unwrap();
    assert_eq!(receipt["ok"], true);
    assert_eq!(
        f.receipts
            .get("s", "w1", START)
            .unwrap()
            .unwrap()
            .follow_until,
        Some(START + 30 * 60_000)
    );
    // No run step takes the steer before the run ends.
    f.runner.end_run();
    f.now.store(START + 61_000, Ordering::SeqCst);
    let outcome = tokio::time::timeout(Duration::from_secs(120), late.recv())
        .await
        .expect("the expiry is reported")
        .unwrap();
    assert_eq!(outcome.session, "s");
    let notice = &outcome.notice["sendOutcome"];
    assert_eq!(notice["ok"], false);
    assert_eq!(notice["code"], "steer_expired");
    assert_eq!(notice["terminal"], true);
    assert_eq!(notice["clientMessageId"], "w1");
    assert_eq!(notice["effectId"], receipt["effectId"]);
    // The follow-up only asked: no send went out after the steer itself.
    assert_eq!(sends(&f.runner).len(), 2);
    assert!(f
        .runner
        .calls()
        .iter()
        .any(|(method, _)| method == "session.send_status"));
    assert_eq!(f.runner.written().len(), 1);
    // A repeat now answers the final outcome, marked as a duplicate, sending nothing.
    let calls = f.runner.calls().len();
    let repeat = f.sender.send(message("w1", "hello")).await.unwrap();
    assert_eq!(repeat["code"], "steer_expired");
    assert_eq!(repeat["duplicate"], true);
    assert_eq!(f.runner.calls().len(), calls);
    assert_eq!(
        f.receipts
            .get("s", "w1", START + 61_000)
            .unwrap()
            .unwrap()
            .follow_until,
        None
    );
}

/// A head send the runner accepted is listed as queued until its message is stored,
/// and one the runner ended undelivered stays listed as unsent, with the reason,
/// so the phone never shows an undelivered message as delivered. Each change is
/// broadcast for the session; a transcript watch passes it on as a notice only when it
/// opted in with `acceptQueuedSends`.
#[tokio::test(start_paused = true)]
async fn a_head_send_is_listed_until_delivered_or_listed_unsent() {
    let f = fixture(Kind::Head);
    let mut changes = f.sender.queue_changes();
    let list = || f.receipts.queue().list("s", START).unwrap();
    let first = f.sender.send(message("h1", "first")).await.unwrap();
    assert_eq!(first["ok"], true);
    let (queued, truncated) = list().expect("the accepted send is listed");
    assert!(!truncated);
    assert_eq!(
        queued,
        json!([{"clientMessageId":"h1", "effectId": first["effectId"],
            "acceptedAtMs": START, "state":"queued"}])
    );
    assert_eq!(changes.recv().await.unwrap(), "s");
    // Its message is stored: it leaves the list.
    f.runner.set_state(&first["effectId"], "delivered");
    eventually(|| list().is_none()).await;
    assert_eq!(changes.recv().await.unwrap(), "s");
    // The run that would have carried the next one ends without it.
    let second = f.sender.send(message("h2", "second")).await.unwrap();
    assert_eq!(changes.recv().await.unwrap(), "s");
    f.runner.set_state(&second["effectId"], "finished");
    eventually(|| list().is_some_and(|(entries, _)| entries[0]["state"] == "unsent")).await;
    let (unsent, _) = list().unwrap();
    assert_eq!(unsent[0]["clientMessageId"], "h2");
    assert_eq!(unsent[0]["reason"], "not_sent");
    assert!(unsent[0]["endedAtMs"].is_i64());
    assert_eq!(changes.recv().await.unwrap(), "s");
    // A page reply carries the list.
    let mut reply = json!({"messages": []});
    f.sender.attach_queued(&mut reply, "s").await;
    assert_eq!(reply["queuedSends"], unsent);
}

#[tokio::test(start_paused = true)]
async fn a_steer_a_step_takes_is_settled_without_a_late_signal() {
    let f = fixture(Kind::Worker);
    let mut late = f.sender.late_outcomes();
    f.sender.send(message("w1", "hello")).await.unwrap();
    // The next step boundary renders it.
    for (_, state) in f.runner.sends.lock().unwrap().values_mut() {
        *state = "delivered".into();
    }
    eventually(|| {
        f.receipts
            .get("s", "w1", START)
            .unwrap()
            .unwrap()
            .follow_until
            .is_none()
    })
    .await;
    let repeat = f.sender.send(message("w1", "hello")).await.unwrap();
    assert_eq!(
        (repeat["ok"].clone(), repeat["duplicate"].clone()),
        (json!(true), json!(true))
    );
    assert!(late.try_recv().is_err());
    // A steer that lands at once is never followed.
    f.runner.lands.store(true, Ordering::SeqCst);
    f.sender.send(message("w2", "now")).await.unwrap();
    assert_eq!(
        f.receipts
            .get("s", "w2", START)
            .unwrap()
            .unwrap()
            .follow_until,
        None
    );
}

#[tokio::test]
async fn a_repeat_within_48_hours_answers_the_first_receipt_and_never_reaches_the_runner() {
    let f = fixture(Kind::Head);
    let first = f.sender.send(message("m1", "hello")).await.unwrap();
    let calls = f.runner.calls().len();
    f.now.store(
        START + super::super::send_store::RETENTION_MS,
        Ordering::SeqCst,
    );
    let repeat = f.sender.send(message("m1", "hello")).await.unwrap();
    let mut expected = first.clone();
    expected["duplicate"] = json!(true);
    assert_eq!(repeat, expected);
    // The same id for other text, or in another mode, is a conflict.
    let changed = f.sender.send(message("m1", "changed")).await.unwrap_err();
    assert_eq!(
        (changed.code, changed.message.as_str()),
        (
            "client_message_id_conflict",
            "clientMessageId was already used for different text or attachments"
        )
    );
    let mut steer = message("m1", "hello");
    steer["mode"] = json!("steer");
    assert_eq!(
        f.sender.send(steer).await.unwrap_err().code,
        "client_message_id_conflict"
    );
    // The same id in another session is another message.
    let elsewhere = f
        .sender
        .send(json!({"session": "s2", "text": "hello", "clientMessageId": "m1"}))
        .await
        .unwrap();
    assert_eq!(elsewhere["duplicate"], false);
    assert_ne!(elsewhere["effectId"], first["effectId"]);
    assert_eq!(f.runner.calls().len(), calls + 3);
    assert_eq!(f.targets.opened.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn after_48_hours_the_id_is_sent_again_under_the_same_runner_key() {
    let f = fixture(Kind::Head);
    let first = f.sender.send(message("m1", "hello")).await.unwrap();
    f.now.store(
        START + super::super::send_store::RETENTION_MS + 1,
        Ordering::SeqCst,
    );
    let again = f.sender.send(message("m1", "hello")).await.unwrap();
    // A new send as far as the receipts know, but the runner key is the same, so
    // the runner answers its existing state instead of delivering twice.
    assert_eq!(again["duplicate"], false);
    assert_eq!(again["effectId"], first["effectId"]);
    assert_eq!(sends(&f.runner).len(), 2);
    assert_eq!(f.runner.written().len(), 1);
    // Different text past retention reaches the runner, which refuses the reused key.
    f.now.store(
        START + 2 * super::super::send_store::RETENTION_MS + 2,
        Ordering::SeqCst,
    );
    let changed = f.sender.send(message("m1", "changed")).await.unwrap_err();
    assert_eq!(changed.code, "client_message_id_conflict");
    assert_eq!(f.runner.written().len(), 1);
}

#[tokio::test]
async fn pruning_removes_only_receipts_past_retention_in_bounded_chunks() {
    let f = fixture(Kind::Head);
    for index in 0..(super::super::send_store::PRUNE_CHUNK + 2) {
        f.receipts
            .record(
                "s",
                &format!("old-{index}"),
                "d",
                "queue",
                "k",
                &json!({"ok": true}),
                None,
                START,
            )
            .unwrap();
    }
    f.receipts
        .record(
            "s",
            "fresh",
            "d",
            "queue",
            "k",
            &json!({"ok": true}),
            None,
            START + 10,
        )
        .unwrap();
    let now = START + super::super::send_store::RETENTION_MS + 5;
    assert_eq!(
        f.receipts.prune(now).unwrap(),
        super::super::send_store::PRUNE_CHUNK
    );
    assert_eq!(f.receipts.prune(now).unwrap(), 2);
    assert_eq!(f.receipts.prune(now).unwrap(), 0);
    assert!(f.receipts.get("s", "fresh", now).unwrap().is_some());
}

#[tokio::test]
async fn runner_refusals_reach_the_app_by_code_and_nothing_is_recorded() {
    let f = fixture(Kind::Head);
    // A session whose frozen plan predates core naming this module as a sender.
    f.runner.named.store(false, Ordering::SeqCst);
    let refusal = f.sender.send(message("m1", "hello")).await.unwrap_err();
    assert_eq!(refusal.code, "enqueue_not_authorized");
    assert_eq!(
        refusal.message,
        "the runner answered scope_owner_mismatch: refused scope_owner_mismatch"
    );
    f.runner.named.store(true, Ordering::SeqCst);
    for (scripted, code) in [
        (FakeRunner::refuse("session_unknown", None), "not_found"),
        (
            FakeRunner::refuse("delivery_unsupported", Some("delivery")),
            "unsupported",
        ),
        (
            FakeRunner::refuse("context_ceiling", None),
            "context_ceiling",
        ),
        (FakeRunner::refuse("run_paused", None), "run_paused"),
        (
            FakeRunner::refuse("pre_user_unavailable", None),
            "runner_refused",
        ),
    ] {
        f.runner.scripted.lock().unwrap().push_back(scripted);
        assert_eq!(
            f.sender
                .send(message("m1", "hello"))
                .await
                .unwrap_err()
                .code,
            code
        );
    }
    assert!(f.receipts.get("s", "m1", START).unwrap().is_none());
    assert!(f.runner.written().is_empty());
    // `runner_refused` says whether the same send may succeed later, with the runner's
    // own code and any wait it named, so the app resends only a transient refusal.
    for (scripted, detail) in [
        (
            FakeRunner::refuse("pre_user_unavailable", None),
            json!({"class": "final", "runnerCode": "pre_user_unavailable"}),
        ),
        (
            FakeRunner::refuse("transient", None),
            json!({"class": "transient", "runnerCode": "transient"}),
        ),
        (
            Err(RunnerError::Refused {
                code: "fetch_unavailable".into(),
                message: "refused".into(),
                detail: Some(json!({"provider": "aft", "retry_after_secs": 1.5})),
            }),
            json!({"class": "transient", "runnerCode": "fetch_unavailable", "retryAfterMs": 1500}),
        ),
    ] {
        f.runner.scripted.lock().unwrap().push_back(scripted);
        let refusal = f.sender.send(message("m1", "hello")).await.unwrap_err();
        assert_eq!(
            (refusal.code, refusal.detail),
            ("runner_refused", Some(detail))
        );
    }
    // A lost route is safe to resend: the key is derived from the app's id.
    f.runner.unreachable.store(true, Ordering::SeqCst);
    let lost = f.sender.send(message("m1", "hello")).await.unwrap();
    assert_eq!(
        (lost["code"].clone(), lost["terminal"].clone()),
        (json!("not_sent"), json!(false))
    );
    f.runner.unreachable.store(false, Ordering::SeqCst);
    let sent = f.sender.send(message("m1", "hello")).await.unwrap();
    assert_eq!(sent["effectId"], lost["effectId"]);
}

/// Core's vectors, replayed through the send: the first send's receipt, the
/// repeat's, and the conflict's refusal, byte for byte except the opaque effect id.
#[tokio::test]
async fn cores_idempotency_vectors_replay_through_the_send() {
    let f = fixture(Kind::Head);
    let vector = |name: &str| -> Value {
        serde_json::from_str(match name {
            "first" => include_str!("../../../../occipital-app-reader/test-vectors/phone-message-idempotency-v1/first-enqueue.json"),
            "repeat" => include_str!("../../../../occipital-app-reader/test-vectors/phone-message-idempotency-v1/repeat.json"),
            _ => include_str!("../../../../occipital-app-reader/test-vectors/phone-message-idempotency-v1/conflict.json"),
        })
        .unwrap()
    };
    let normalized = |mut reply: Value| {
        assert!(reply["effectId"].as_str().unwrap().starts_with("app-"));
        reply["effectId"] = json!("<effectId>");
        reply
    };
    for (name, session) in [("first", "phone-target"), ("repeat", "phone-target")] {
        let record = vector(name);
        assert_eq!(record["request"]["session"], session);
        let reply = f.sender.send(record["request"].clone()).await.unwrap();
        assert_eq!(normalized(reply), record["reply"], "{name}");
    }
    let record = vector("conflict");
    let refusal = f.sender.send(record["request"].clone()).await.unwrap_err();
    assert_eq!(
        json!({"code": refusal.code, "message": refusal.message}),
        record["refusal"]
    );
    assert_eq!(sends(&f.runner).len(), 1);
}

#[tokio::test]
async fn concurrent_attempts_for_one_message_send_once() {
    let f = Arc::new(fixture(Kind::Head));
    let attempts: Vec<_> = (0..4)
        .map(|_| {
            let f = f.clone();
            tokio::spawn(async move { f.sender.send(message("m1", "hello")).await.unwrap() })
        })
        .collect();
    let mut duplicates = 0;
    for attempt in attempts {
        duplicates += usize::from(attempt.await.unwrap()["duplicate"] == true);
    }
    assert_eq!(duplicates, 3);
    assert_eq!(sends(&f.runner).len(), 1);
}

fn statuses(runner: &FakeRunner) -> usize {
    runner
        .calls()
        .iter()
        .filter(|(method, _)| method == "session.send_status")
        .count()
}

#[tokio::test(start_paused = true)]
async fn a_worker_steer_survives_sixty_seconds_in_the_same_run_then_lands() {
    let f = fixture(Kind::Worker);
    let mut late = f.sender.late_outcomes();
    let receipt = f
        .sender
        .send(message("long-tool", "check the logs"))
        .await
        .unwrap();
    assert_eq!(
        f.receipts.followed(1).unwrap()[0].run_id.as_deref(),
        Some("r1")
    );
    // The tool call lasts ninety seconds with no step boundary to take the steer.
    f.now.store(START + 90_000, Ordering::SeqCst);
    eventually(|| {
        statuses(&f.runner) > 0
            && (f.receipts.followed(1).unwrap().is_empty() || call_counts(&f.runner).0 >= 2)
    })
    .await;
    assert_eq!(
        f.receipts.followed(1).unwrap().len(),
        1,
        "an active run's steer must not expire during a long tool call"
    );
    let (queued, _) = f.sender.queued("s").await.unwrap();
    assert_eq!(queued[0]["state"], "queued");
    assert!(late.try_recv().is_err());
    f.runner.set_state(&receipt["effectId"], "delivered");
    eventually(|| f.receipts.queue().list("s", START).unwrap().is_none()).await;
    assert!(f.receipts.followed(1).unwrap().is_empty());
    let repeat = f
        .sender
        .send(message("long-tool", "check the logs"))
        .await
        .unwrap();
    assert_eq!(repeat["ok"], true);
    assert!(late.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn a_pending_steer_is_retracted_when_its_run_ends_or_is_replaced() {
    for replaced in [false, true] {
        let f = fixture(Kind::Worker);
        let receipt = f.sender.send(message("w1", "hello")).await.unwrap();
        if replaced {
            *f.runner.run_id.lock().unwrap() = "r2".into();
        } else {
            // Broca keeps a steer pending after its run ends, until retracted.
            f.runner.running.store(false, Ordering::SeqCst);
        }
        eventually(|| {
            f.receipts
                .queue()
                .list("s", START)
                .unwrap()
                .is_some_and(|(rows, _)| rows[0]["reason"] == "run_ended")
        })
        .await;
        let (unsent, _) = f.sender.queued("s").await.unwrap();
        assert_eq!(
            unsent,
            json!([{
                "clientMessageId": "w1", "effectId": receipt["effectId"],
                "acceptedAtMs": START, "state": "unsent", "reason": "run_ended",
                "endedAtMs": START,
            }])
        );
        assert!(f.receipts.followed(1).unwrap().is_empty());
        assert_eq!(
            f.runner
                .calls()
                .iter()
                .filter(|(method, _)| method == "session.retract")
                .count(),
            1
        );
        assert_eq!(f.runner.written().len(), 1);
        let repeat = f.sender.send(message("w1", "hello")).await.unwrap();
        assert_eq!(
            repeat["ok"], true,
            "the accepted receipt's shape is unchanged"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_restart_retracts_a_steer_whose_run_ended_while_the_module_was_down() {
    let mut f = fixture(Kind::Worker);
    let receipt = f.sender.send(message("w1", "hello")).await.unwrap();
    f.restart();
    f.runner.running.store(false, Ordering::SeqCst);
    f.now.store(START + 90_000, Ordering::SeqCst);
    let _background = f.sender.start();
    eventually(|| {
        f.receipts
            .queue()
            .list("s", START)
            .unwrap()
            .is_some_and(|(rows, _)| rows[0]["reason"] == "run_ended")
    })
    .await;
    let (unsent, _) = f.sender.queued("s").await.unwrap();
    assert_eq!(unsent[0]["state"], "unsent");
    assert_eq!(unsent[0]["effectId"], receipt["effectId"]);
    assert!(f.receipts.followed(1).unwrap().is_empty());
    assert!(f
        .runner
        .calls()
        .iter()
        .any(|(method, _)| method == "session.retract"));
    assert_eq!(f.runner.written().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_retract_that_loses_to_delivery_uses_the_durable_send_status() {
    for reply in ["not_pending", "already_started", "refused"] {
        let f = fixture(Kind::Worker);
        let mut late = f.sender.late_outcomes();
        let receipt = f.sender.send(message("w1", "hello")).await.unwrap();
        f.runner.running.store(false, Ordering::SeqCst);
        let answer = if reply == "refused" {
            Err(RunnerError::Refused {
                code: "not_pending".into(),
                message: "already landed".into(),
                detail: None,
            })
        } else {
            Ok(json!(reply))
        };
        *f.runner.retract_race.lock().unwrap() = Some((answer, "delivered".into()));
        eventually(|| f.receipts.queue().list("s", START).unwrap().is_none()).await;
        assert!(
            statuses(&f.runner) >= 2,
            "retract alone does not prove delivery"
        );
        assert_eq!(
            f.receipts.get("s", "w1", START).unwrap().unwrap().receipt,
            receipt
        );
        assert!(f.receipts.followed(1).unwrap().is_empty());
        assert!(late.try_recv().is_err());
    }
}

#[tokio::test(start_paused = true)]
async fn a_not_pending_retract_is_not_delivery_when_status_says_expired() {
    let f = fixture(Kind::Worker);
    let mut late = f.sender.late_outcomes();
    f.sender.send(message("w1", "hello")).await.unwrap();
    f.runner.running.store(false, Ordering::SeqCst);
    *f.runner.retract_race.lock().unwrap() = Some((Ok(json!("not_pending")), "expired".into()));
    eventually(|| {
        f.receipts
            .queue()
            .list("s", START)
            .unwrap()
            .is_some_and(|(rows, _)| rows[0]["reason"] == "steer_expired")
    })
    .await;
    assert_eq!(
        late.try_recv().unwrap().notice["sendOutcome"]["code"],
        "steer_expired"
    );
}

#[tokio::test(start_paused = true)]
async fn a_not_pending_retract_preserves_an_undelivered_terminal_status() {
    let f = fixture(Kind::Worker);
    f.sender.send(message("w1", "hello")).await.unwrap();
    f.runner.running.store(false, Ordering::SeqCst);
    *f.runner.retract_race.lock().unwrap() = Some((Ok(json!("not_pending")), "superseded".into()));
    eventually(|| {
        f.receipts
            .queue()
            .list("s", START)
            .unwrap()
            .is_some_and(|(rows, _)| rows[0]["reason"] == "superseded")
    })
    .await;
    assert!(f.receipts.followed(1).unwrap().is_empty());
}

/// Retract is owner-only, and a worker's owner is core, not the phone's sender. A
/// runner that refuses the sender's withdraw once refuses it every time, so the
/// follower stops asking and the steer settles at its deadline.
#[tokio::test(start_paused = true)]
async fn an_owner_only_retract_refusal_leaves_the_steer_for_its_deadline() {
    let f = fixture(Kind::Worker);
    let mut late = f.sender.late_outcomes();
    f.sender.send(message("w1", "hello")).await.unwrap();
    f.runner.running.store(false, Ordering::SeqCst);
    let refused = Err(RunnerError::Refused {
        code: "scope_owner_mismatch".into(),
        message: "not the owner".into(),
        detail: None,
    });
    // Only the first retract is refused: a follower that asked again would see
    // `retracted` and report `run_ended`, which a real owner-only runner never says.
    *f.runner.retract_race.lock().unwrap() = Some((refused, "pending".into()));
    let count = |name: &str| {
        f.runner
            .calls()
            .iter()
            .filter(|(method, _)| method == name)
            .count()
    };
    let retracts = || count("session.retract");
    // Several follow rounds after the refusal, all before the deadline.
    eventually(|| retracts() == 1 && count("session.send_status") >= 6).await;
    assert_eq!(retracts(), 1);
    let (queued, _) = f.sender.queued("s").await.unwrap();
    assert_eq!(queued[0]["state"], "queued");
    f.now.store(START + 31 * 60_000, Ordering::SeqCst);
    eventually(|| f.receipts.followed(1).unwrap().is_empty()).await;
    assert_eq!(retracts(), 1);
    assert_eq!(
        late.try_recv().unwrap().notice["sendOutcome"]["code"],
        "steer_expired"
    );
}

#[tokio::test(start_paused = true)]
async fn a_runner_without_retract_leaves_the_steer_for_backstop_expiry() {
    let f = fixture(Kind::Worker);
    f.runner.retractable.store(false, Ordering::SeqCst);
    let mut late = f.sender.late_outcomes();
    f.sender.send(message("w1", "hello")).await.unwrap();
    f.runner.running.store(false, Ordering::SeqCst);
    eventually(|| call_counts(&f.runner).2 >= 2).await;
    assert_eq!(f.receipts.followed(1).unwrap().len(), 1);
    let (queued, _) = f.sender.queued("s").await.unwrap();
    assert_eq!(queued[0]["state"], "queued");
    assert!(!f
        .runner
        .calls()
        .iter()
        .any(|(method, _)| method == "session.retract"));
    f.now.store(START + 31 * 60_000, Ordering::SeqCst);
    eventually(|| f.receipts.followed(1).unwrap().is_empty()).await;
    assert_eq!(
        late.try_recv().unwrap().notice["sendOutcome"]["code"],
        "steer_expired"
    );
}

#[tokio::test(start_paused = true)]
async fn a_restart_during_the_follow_still_reports_the_expiry() {
    let mut f = fixture(Kind::Worker);
    let receipt = f.sender.send(message("w1", "hello")).await.unwrap();
    assert_eq!(
        f.receipts
            .get("s", "w1", START)
            .unwrap()
            .unwrap()
            .follow_until,
        Some(START + 30 * 60_000)
    );
    // The module stops ten seconds in, before its follow-up settled the steer.
    f.now.store(START + 10_000, Ordering::SeqCst);
    f.restart();
    let mut late = f.sender.late_outcomes();
    let _background = f.sender.start();
    // The run then ends without taking the steer.
    f.runner.end_run();
    f.now.store(START + 61_000, Ordering::SeqCst);
    let outcome = tokio::time::timeout(Duration::from_secs(120), late.recv())
        .await
        .expect("the restarted module reports the expiry")
        .unwrap();
    assert_eq!(outcome.session, "s");
    assert_eq!(outcome.notice["sendOutcome"]["code"], "steer_expired");
    assert_eq!(
        outcome.notice["sendOutcome"]["effectId"],
        receipt["effectId"]
    );
    assert_eq!(outcome.notice["sendOutcome"]["clientMessageId"], "w1");
    let stored = f.receipts.get("s", "w1", START + 61_000).unwrap().unwrap();
    assert_eq!(stored.follow_until, None);
    assert_eq!(stored.receipt["code"], "steer_expired");
    // Recovery only asked the runner; it never sent again.
    assert_eq!(sends(&f.runner).len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_restart_after_the_deadline_settles_each_held_steer_from_send_status() {
    let mut f = fixture(Kind::Worker);
    let mut ids = HashMap::new();
    for id in ["landed", "expired", "never-landed", "unknown"] {
        let receipt = f.sender.send(message(id, id)).await.unwrap();
        ids.insert(id, receipt["effectId"].clone());
    }
    let sent = sends(&f.runner).len();
    f.restart();
    // While the module was down: one steer landed, one expired, one is still
    // listed as pending past its deadline, and one the runner has no record of.
    f.runner.set_state(&ids["landed"], "delivered");
    f.runner.set_state(&ids["expired"], "expired");
    f.runner
        .sends
        .lock()
        .unwrap()
        .remove(ids["unknown"].as_str().unwrap());
    f.now.store(START + 31 * 60_000, Ordering::SeqCst);
    // The runner is unreachable at first: nothing is settled and nothing invented.
    f.runner.unreachable.store(true, Ordering::SeqCst);
    let mut late = f.sender.late_outcomes();
    let _background = f.sender.start();
    eventually(|| statuses(&f.runner) >= 4).await;
    assert_eq!(f.receipts.followed(10).unwrap().len(), 4);
    assert!(late.try_recv().is_err());
    for id in ids.keys() {
        let stored = f.receipts.get("s", id, START).unwrap().unwrap();
        assert_eq!(stored.receipt["ok"], true, "{id}");
    }
    // The next sweep reaches it and settles each one from a single answer.
    f.runner.unreachable.store(false, Ordering::SeqCst);
    eventually(|| f.receipts.followed(10).unwrap().is_empty()).await;
    let receipt = |id: &str| f.receipts.get("s", id, START).unwrap().unwrap().receipt;
    assert_eq!(receipt("landed")["ok"], true);
    assert_eq!(receipt("landed")["effectId"], ids["landed"]);
    for id in ["expired", "never-landed", "unknown"] {
        assert_eq!(receipt(id)["code"], "steer_expired", "{id}");
        assert_eq!(receipt(id)["terminal"], true, "{id}");
    }
    let mut reported = Vec::new();
    while let Ok(outcome) = late.try_recv() {
        reported.push(
            outcome.notice["sendOutcome"]["clientMessageId"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    reported.sort();
    assert_eq!(reported, ["expired", "never-landed", "unknown"]);
    // Recovery asked with session.send_status only.
    assert_eq!(sends(&f.runner).len(), sent);
    // A repeat answers the settled outcome without reaching the runner.
    let calls = f.runner.calls().len();
    let repeat = f
        .sender
        .send(message("never-landed", "never-landed"))
        .await
        .unwrap();
    assert_eq!(
        (repeat["code"].clone(), repeat["duplicate"].clone()),
        (json!("steer_expired"), json!(true))
    );
    assert_eq!(f.runner.calls().len(), calls);
}

mod samples_0_7 {
    include!("../../../../occipital-app-reader/tests/fixtures/broca_0_7.rs");
}

#[tokio::test]
async fn broca_0_7_heads_and_describes_keep_enqueue_permissions() {
    use crate::app_reader::broca::tests::current_contract;
    use crate::app_reader::{AppReader, UnwiredResolver, SEND};
    for head_file in [
        "session.head.completed.json",
        "session.head.error.fractional.json",
        "session.head.error.missing-delay.json",
    ] {
        for (name, head) in samples_0_7::samples(head_file) {
            let f = fixture(Kind::Head);
            f.runner
                .captured
                .lock()
                .unwrap()
                .insert("session.head", current_contract(head));
            let app = AppReader::with_sender(Arc::new(UnwiredResolver), f.sender.clone());
            let reply = app
                .read(SEND, message("phone-granted", "hello"))
                .await
                .unwrap();
            assert_eq!(reply["ok"], true, "{name}: {reply}");
            assert_eq!(f.runner.written().len(), 1, "{name}");
            assert_eq!(call_counts(&f.runner), (1, 1, 0), "{name}");
        }
    }
    for file in ["role.describe.before.json", "role.describe.after.json"] {
        for (name, describe) in samples_0_7::samples(file) {
            let f = fixture(Kind::Head);
            f.runner
                .captured
                .lock()
                .unwrap()
                .insert("role.describe", current_contract(describe));
            let app = AppReader::with_sender(Arc::new(UnwiredResolver), f.sender.clone());
            let error = app
                .read(SEND, message("phone-refused", "hello"))
                .await
                .unwrap_err();
            assert_eq!(error.code, "enqueue_not_authorized", "{name}");
            assert_eq!(call_counts(&f.runner), (1, 0, 1), "{name}");
            assert!(f.runner.written().is_empty(), "{name}");
        }
    }
}

/// Stop cancels the named run through `run.cancel`, as a sender the plan grants
/// `interrupt`, once the runner declares `run_cancel_by_interrupt`. A repeat gets the first
/// answer without calling the runner again; a run that already ended another way is
/// `not_active`; a runner refusal keeps the send's mapping.
#[tokio::test]
async fn stop_cancels_the_named_run_and_answers_repeats_the_same() {
    let f = fixture(Kind::Head);
    f.runner.captured.lock().unwrap().insert(
        "role.describe",
        json!({"result": {"capabilities": ["run_cancel", "delivery_order", "run_cancel_by_interrupt"]}}),
    );
    f.runner
        .captured
        .lock()
        .unwrap()
        .insert("run.cancel", json!({"result": "ack"}));
    let stop = json!({"session": "s", "runId": "r1", "clientRequestId": "stop-1"});
    let answer = f.sender.stop(stop.clone()).await.unwrap();
    assert_eq!(answer, json!({"state": "stopped", "runId": "r1"}));
    let cancels = |runner: &FakeRunner| {
        runner
            .calls()
            .into_iter()
            .filter(|(method, _)| method == "run.cancel")
            .collect::<Vec<_>>()
    };
    assert_eq!(
        cancels(&f.runner),
        [(
            "run.cancel".to_string(),
            json!({"run_id": "r1", "at": "now"})
        )]
    );
    assert_eq!(f.sender.stop(stop).await.unwrap(), answer);
    assert_eq!(cancels(&f.runner).len(), 1, "a repeat calls nothing");

    // Already ended, and not by a cancel.
    f.runner
        .captured
        .lock()
        .unwrap()
        .insert("run.cancel", json!({"result": "not_active"}));
    f.runner
        .captured
        .lock()
        .unwrap()
        .insert("run.status", json!({"result": {"state": "completed"}}));
    let ended = json!({"session": "s", "runId": "r0", "clientRequestId": "stop-2"});
    assert_eq!(
        f.sender.stop(ended).await.unwrap(),
        json!({"state": "not_active", "runId": "r0"})
    );
    // Ended cancelled (a first stop this module no longer remembers): stopped.
    f.runner
        .captured
        .lock()
        .unwrap()
        .insert("run.status", json!({"result": {"state": "cancelled"}}));
    let forgotten = json!({"session": "s", "runId": "r1", "clientRequestId": "stop-3"});
    assert_eq!(f.sender.stop(forgotten).await.unwrap()["state"], "stopped");

    // A grant without `interrupt`: the runner refuses, as it would a send.
    f.runner.captured.lock().unwrap().insert(
        "run.cancel",
        json!({"code": "scope_owner_mismatch", "message": "not the owner"}),
    );
    let refused = json!({"session": "s", "runId": "r2", "clientRequestId": "stop-4"});
    assert_eq!(
        f.sender.stop(refused).await.unwrap_err().code,
        codes::NOT_AUTHORIZED
    );
}

/// A Stop's log line names the runner calls it made and what it answered, so the
/// log alone shows whether `run.cancel` went out, for which run, and how it came
/// back. A repeat answered from the kept first answer is marked `(repeat)`.
#[tokio::test]
async fn stop_logs_its_runner_calls_and_answer() {
    let f = fixture(Kind::Head);
    f.runner.captured.lock().unwrap().insert(
        "role.describe",
        json!({"result": {"capabilities": ["run_cancel", "delivery_order", "run_cancel_by_interrupt"]}}),
    );
    f.runner
        .captured
        .lock()
        .unwrap()
        .insert("run.cancel", json!({"result": "not_active"}));
    f.runner
        .captured
        .lock()
        .unwrap()
        .insert("run.status", json!({"result": {"state": "completed"}}));
    let stop = json!({"session": "s", "runId": "run_old", "clientRequestId": "stop-1"});
    for (correlation, answer) in [
        ("1:1:1", "answer=\"not_active run=run_old\""),
        ("1:1:2", "answer=\"not_active run=run_old (repeat)\""),
    ] {
        let (log, _guard) = crate::app_reader::observe::tests::capture();
        crate::app_reader::observe::request("session.stop", "s", correlation.into(), async {
            f.sender.stop(stop.clone()).await.unwrap();
        })
        .await;
        let text = log.text();
        assert!(text.contains(answer), "{text}");
        if correlation == "1:1:1" {
            assert!(
                text.contains(
                    "remote_calls={\"broca.role.describe\": 1, \"broca.run.cancel\": 1, \"broca.run.status\": 1}"
                ),
                "{text}"
            );
        }
    }
}

/// A runner that doesn't declare `run_cancel_by_interrupt` can't be stopped by a sender,
/// so Stop is refused `unsupported` without calling `run.cancel`; a malformed request is
/// refused naming its field.
#[tokio::test]
async fn stop_is_unsupported_without_the_runner_group_and_checks_its_request() {
    let f = fixture(Kind::Head);
    f.runner.captured.lock().unwrap().insert(
        "role.describe",
        json!({"result": {"capabilities": ["run_cancel", "delivery_order"]}}),
    );
    let refusal = f
        .sender
        .stop(json!({"session": "s", "runId": "r1", "clientRequestId": "stop-1"}))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, codes::UNSUPPORTED);
    assert_eq!(refusal.detail, Some(json!({"field": "session.stop"})));
    assert!(f
        .runner
        .calls()
        .iter()
        .all(|(method, _)| method != "run.cancel"));
    for (params, field) in [
        (json!({"session": "s", "clientRequestId": "c"}), "runId"),
        (json!({"session": "s", "runId": "r1"}), "clientRequestId"),
        (
            json!({"session": "s", "runId": "r1", "clientRequestId": "c", "at": "now"}),
            "at",
        ),
    ] {
        let refusal = f.sender.stop(params).await.unwrap_err();
        assert_eq!(refusal.code, "invalid_request");
        assert_eq!(refusal.detail, Some(json!({"field": field})));
    }
}

/// Stop everything calls the runner's `session.stop_all` under the app's request id,
/// naming the run when the app does, and passes the runner's reply on unchanged, item
/// members it doesn't know included. A repeat goes to the runner under the same id,
/// which answers it with its first reply.
#[tokio::test]
async fn stop_all_asks_the_runner_under_the_apps_id_and_passes_its_reply_on() {
    let f = fixture(Kind::Head);
    f.runner.captured.lock().unwrap().insert(
        "role.describe",
        json!({"result": {"capabilities": ["run_cancel", "delivery_order",
            "run_cancel_by_interrupt", "stop_all"]}}),
    );
    let reply = json!({"providers": [
        {"provider": "aft", "outcome": "answered", "items": [
            {"kind": "shell", "id": "bash-1", "label": "cargo test", "outcome": "stopped",
                "future_member": 1}]},
        {"provider": "basal", "outcome": "no_answer"}],
        "runner": {"items": []}});
    f.runner
        .captured
        .lock()
        .unwrap()
        .insert("session.stop_all", json!({"result": reply.clone()}));
    let stop = json!({"session": "s", "runId": "r1", "clientRequestId": "stop-all-1"});
    assert_eq!(f.sender.stop_all(stop.clone()).await.unwrap(), reply);
    assert_eq!(f.sender.stop_all(stop).await.unwrap(), reply);
    let anyway = json!({"session": "s", "clientRequestId": "stop-all-2"});
    f.sender.stop_all(anyway).await.unwrap();
    let asked: Vec<_> = f
        .runner
        .calls()
        .into_iter()
        .filter(|(method, _)| method == "session.stop_all")
        .map(|(_, params)| params)
        .collect();
    assert_eq!(
        asked,
        [
            json!({"request_id": "stop-all-1", "run_id": "r1"}),
            json!({"request_id": "stop-all-1", "run_id": "r1"}),
            json!({"request_id": "stop-all-2"}),
        ]
    );

    // A sender without `interrupt`: the runner refuses, as it does Stop.
    f.runner.captured.lock().unwrap().insert(
        "session.stop_all",
        json!({"code": "scope_owner_mismatch", "message": "not the owner"}),
    );
    let refused = json!({"session": "s", "clientRequestId": "stop-all-3"});
    assert_eq!(
        f.sender.stop_all(refused).await.unwrap_err().code,
        codes::NOT_AUTHORIZED
    );
}

/// A runner that doesn't declare `stop_all` is never sent it, which it would refuse;
/// the app is told `unsupported` and can fall back to Stop. A malformed request,
/// including an id the runner couldn't take, is refused naming its field.
#[tokio::test]
async fn stop_all_is_unsupported_without_the_runner_group_and_checks_its_request() {
    let f = fixture(Kind::Head);
    f.runner.captured.lock().unwrap().insert(
        "role.describe",
        json!({"result": {"capabilities": ["run_cancel", "delivery_order",
            "run_cancel_by_interrupt"]}}),
    );
    let refusal = f
        .sender
        .stop_all(json!({"session": "s", "clientRequestId": "stop-all-1"}))
        .await
        .unwrap_err();
    assert_eq!(refusal.code, codes::UNSUPPORTED);
    assert_eq!(refusal.detail, Some(json!({"field": "session.stop_all"})));
    assert!(f
        .runner
        .calls()
        .iter()
        .all(|(method, _)| method != "session.stop_all"));
    for (params, field) in [
        (json!({"session": "s"}), "clientRequestId"),
        (
            json!({"session": "s", "clientRequestId": "x".repeat(129)}),
            "clientRequestId",
        ),
        (
            json!({"session": "s", "clientRequestId": "tab\tin"}),
            "clientRequestId",
        ),
        (
            json!({"session": "s", "clientRequestId": "c", "runId": ""}),
            "runId",
        ),
        (
            json!({"session": "s", "clientRequestId": "c", "at": "now"}),
            "at",
        ),
        (json!({"clientRequestId": "c"}), "session"),
    ] {
        let refusal = f.sender.stop_all(params).await.unwrap_err();
        assert_eq!(refusal.code, "invalid_request");
        assert_eq!(refusal.detail, Some(json!({"field": field})));
    }
}

/// The published synthetic refusals in `phone-reader-runner-refused-v1` are exactly what
/// this mapping answers for those runner refusals.
#[test]
fn the_published_runner_refused_samples_match_the_mapping() {
    let published = |name: &str| -> Value {
        let path = format!(
            "{}/../occipital-app-reader/test-vectors/phone-reader-runner-refused-v1/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    };
    let unavailable = "a required plan item could not be fetched";
    for (name, code, message, detail) in [
        (
            "06-runner-refused-transient.synthetic.json",
            "fetch_unavailable",
            unavailable,
            json!({"provider": "aft", "reason": "timeout"}),
        ),
        (
            "07-runner-refused-transient-wait.synthetic.json",
            "fetch_unavailable",
            unavailable,
            json!({"provider": "aft", "reason": "timeout", "retry_after_secs": 1.5}),
        ),
        (
            "08-runner-refused-final.synthetic.json",
            "session_ended",
            "plan_stale",
            Value::Null,
        ),
    ] {
        let detail = (!detail.is_null()).then_some(detail);
        let refusal = super::refused(code, message, detail.as_ref());
        let wire = json!({"kind": "moduleError", "code": refusal.code,
            "message": refusal.message, "detail": refusal.detail});
        assert_eq!(wire, published(name), "{name}");
    }
}
