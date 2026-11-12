//! The apps' send: `session.enqueue_user_message`, from an app through this module to
//! the session's runner.
//!
//! The request, its refusals and its duplicate handling are core's (see
//! `occipital_app_reader::send` and `send_store`). What changes is the path: core put
//! the message on its own delivery outbox, while this module hands it to the runner
//! that hosts the session, through the runner contract's `session.send`, as a sender
//! the session's plan names. The runner's reply is final for the app, except for a
//! worker steer, which this module follows until it lands or expires.
//!
//! Receipts are decoded by code (`occipital_app_reader::send::codes`):
//! - `{ok: true, effectId, clientMessageId, duplicate}`: the runner holds the message;
//! - `{ok: false, code: "not_sent", terminal: false}`: nothing reached the runner, or a
//!   resend is harmless because the runner key is derived from the app's id;
//! - an error with `enqueue_not_authorized`: the session's plan does not let this
//!   path send what was asked;
//! - any other code, as an error or as `{ok: false, terminal: true}`, is final.

use crate::app_reader::Guard as _;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use async_trait::async_trait;
use cortexkit_role_llm_runner::{
    attachments::{ContentPart, CHUNK_MAX},
    delivery_order::{DeliveryOrder, Mode as Order},
    describe::RoleDescribe,
    ops::SESSION_SEND_STATUS,
    read::HeadMeta,
    send::{states, Delivery, SendReply, SendRequest as RunnerSend},
};
use occipital_app_reader::{
    send::{codes, Mode, SendRefusal, SendRequest},
    source::{Attachments, InputCapability, SendGrant},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;

use super::{
    resolve::SessionRef,
    send_store::Receipts,
    uploads::{RunnerUpload, Uploads},
};

/// Only a backstop if the runner cannot retract: the follower normally withdraws
/// a worker steer when its target run ends, not while a long tool call is running.
pub const WORKER_STEER_EXPIRY: Duration = Duration::from_secs(30 * 60);

/// How often a held worker steer is checked until it lands or expires.
pub const FOLLOW_INTERVAL: Duration = Duration::from_secs(5);

/// A steer still undelivered at its expiry can never be delivered: the runner drops
/// it at its next boundary. Past the expiry and this margin for clock reads, an
/// answer of `pending` therefore settles it as expired.
pub const SETTLE_GRACE: Duration = Duration::from_secs(5);

/// How often stored worker steers that no task follows are picked up again.
pub const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

/// The longest a head send is followed. The runner's own hold on a queued prompt is
/// 7 days; a day more covers a restart near its end. A send the runner still reports
/// as waiting after this is listed as unsent with [`HOLD_EXPIRED`]; one the runner
/// doesn't answer for stays queued, and the next recovery sweep asks again.
pub const HEAD_HOLD: Duration = Duration::from_secs(8 * 24 * 60 * 60);

/// Why a head send the runner still held after [`HEAD_HOLD`] is listed as unsent.
pub const HOLD_EXPIRED: &str = "hold_expired";
/// A head send whose session the runner no longer has.
pub const SESSION_ENDED: &str = "session_ended";
/// A head send that ended undelivered for a reason the runner didn't name.
pub const NOT_SENT_REASON: &str = "not_sent";
/// A worker steer withdrawn because the run it was sent into is no longer active.
pub const RUN_ENDED: &str = "run_ended";

/// A file the send names can't go into this model's prompt: its runner takes no
/// files, or its model refuses the type. Nothing was sent; the app can place the
/// file in the project (`upload.place`) and send its path instead.
pub const MEDIA_UNSUPPORTED: &str = "media_unsupported";
/// An image the send names is above the model's limit (`detail` names it). Nothing
/// was sent.
pub const MEDIA_TOO_LARGE: &str = "media_too_large";
/// The send names more images, or more other files, than a message to this
/// session may carry (`detail`: `maxImages`, `maxFiles`, and the send's `images`
/// and `files`). Nothing was sent. The limits are the ones `session.details`
/// reports as `attachments`.
pub const TOO_MANY_FILES: &str = "too_many_files";
/// The runner lost a file this module copied to it before the send named it.
/// Nothing was sent; a resend copies the file again.
pub const UPLOAD_GONE: &str = "upload_gone";

/// Rows resumed per recovery sweep, oldest deadline first.
pub const RECOVERY_BATCH: usize = 64;

/// How often expired receipts are removed. Repeats never read an expired row, so
/// this bounds the table's size, not correctness.
pub const PRUNE_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Chunks removed per prune pass, each [`super::send_store::PRUNE_CHUNK`] rows.
const PRUNE_CHUNKS_PER_PASS: usize = 8;

/// Whether the session is a lead agent's (a head) or a worker's. Core's plans grant
/// them different deliveries: a head lets this module queue, steer and interrupt; a
/// worker lets it only steer the run in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Head,
    Worker,
}

/// What the runner answered a call with, when it did not answer a result.
#[derive(Debug, Clone, PartialEq)]
pub enum RunnerError {
    /// A named refusal: the runner wrote nothing.
    Refused {
        code: String,
        message: String,
        detail: Option<Value>,
    },
    /// The call never reached the runner, or its outcome is unknown. A resend
    /// under the same derived `send_id` cannot deliver twice.
    Unreachable(String),
}

/// One resolved runner session, ready to take calls as this module.
#[async_trait]
pub trait SendTarget: Send + Sync {
    fn reference(&self) -> &SessionRef;
    async fn call(&self, method: &'static str, params: Value) -> Result<Value, RunnerError>;
}

/// Resolution of the app's `session` to the runner session that hosts it.
#[async_trait]
pub trait SendTargets: Send + Sync {
    /// Recheck placement before replaying a receipt without opening the runner.
    async fn validate(
        &self,
        _session: &str,
        _harness: Option<&str>,
    ) -> Result<(), occipital_app_reader::source::ReadError> {
        Ok(())
    }

    /// The runner session hosting the app's `session`, named with its `harness` when it
    /// isn't a Broca session. `Err` with `session_unknown` for a session no runner this
    /// module sends to is registered for; any other error means the lookup itself failed.
    async fn open(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<Arc<dyn SendTarget>, occipital_app_reader::source::ReadError>;
}

/// The send path's targets, each runner call counted and timed in the request log
/// (`remote_calls`, `steps`) as a read's runner calls are, so the log shows what a
/// send, Stop or upload asked the runner and how long it took.
struct Observed(Arc<dyn SendTargets>);

#[async_trait]
impl SendTargets for Observed {
    async fn validate(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<(), occipital_app_reader::source::ReadError> {
        self.0.validate(session, harness).await
    }

    async fn open(
        &self,
        session: &str,
        harness: Option<&str>,
    ) -> Result<Arc<dyn SendTarget>, occipital_app_reader::source::ReadError> {
        let target = self.0.open(session, harness).await?;
        Ok(Arc::new(ObservedTarget(target)))
    }
}

struct ObservedTarget(Arc<dyn SendTarget>);

#[async_trait]
impl SendTarget for ObservedTarget {
    fn reference(&self) -> &SessionRef {
        self.0.reference()
    }

    async fn call(&self, method: &'static str, params: Value) -> Result<Value, RunnerError> {
        let runner = self.0.reference().runner.clone();
        super::observe::remote(&runner, method, self.0.call(method, params)).await
    }
}

/// A send's late outcome, put on the session's transcript watches. Today the only
/// one is a worker steer that expired before a run took it.
#[derive(Debug, Clone, PartialEq)]
pub struct LateOutcome {
    pub session: String,
    pub notice: Value,
}

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
/// One lock per session and `clientMessageId`, dropped when no attempt holds it.
type MessageLocks = HashMap<(String, String), Weak<tokio::sync::Mutex<()>>>;

pub struct Sender {
    targets: Arc<dyn SendTargets>,
    receipts: Option<Arc<Receipts>>,
    /// The apps' uploads, whose prompt files a send copies to the runner. Without
    /// them, a send naming a file is refused.
    uploads: Option<Arc<Uploads>>,
    clock: Clock,
    locks: Mutex<MessageLocks>,
    late: broadcast::Sender<LateOutcome>,
    /// The session whose queued or unsent list just changed.
    queue_changed: broadcast::Sender<String>,
    /// Sends a task here follows, by session and runner `send_id`.
    following: Arc<Mutex<HashSet<(String, String)>>>,
    /// The follow tasks; dropping the sender stops them.
    tasks: Mutex<tokio::task::JoinSet<()>>,
    /// Stop answers by session and `clientRequestId`, so a repeat gets the first answer.
    stops: Mutex<HashMap<(String, String), Value>>,
    /// The model catalog that bounds how many files a message carries. Until
    /// `set_model_names` supplies it, the fallback limits apply.
    model_names: std::sync::OnceLock<Arc<crate::display::model_names::ModelNames>>,
}

fn wall_clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

impl Sender {
    pub fn new(targets: Arc<dyn SendTargets>, receipts: Option<Arc<Receipts>>) -> Self {
        Self::with_clock(targets, receipts, Arc::new(wall_clock))
    }

    pub fn with_clock(
        targets: Arc<dyn SendTargets>,
        receipts: Option<Arc<Receipts>>,
        clock: Clock,
    ) -> Self {
        Self {
            targets: Arc::new(Observed(targets)),
            receipts,
            uploads: None,
            clock,
            locks: Mutex::new(HashMap::new()),
            late: broadcast::channel(64).0,
            queue_changed: broadcast::channel(64).0,
            following: Arc::default(),
            tasks: Mutex::default(),
            stops: Mutex::default(),
            model_names: std::sync::OnceLock::new(),
        }
    }

    pub fn with_uploads(mut self, uploads: Option<Arc<Uploads>>) -> Self {
        self.uploads = uploads;
        self
    }

    /// Hold sends to the attachment limits `names` reads, once.
    pub fn set_model_names(&self, names: Arc<crate::display::model_names::ModelNames>) {
        let _ = self.model_names.set(names);
    }

    /// Late outcomes for every session; a transcript watch keeps those for its own.
    pub fn late_outcomes(&self) -> broadcast::Receiver<LateOutcome> {
        self.late.subscribe()
    }

    /// Each session whose queued or unsent list changed; a transcript watch keeps
    /// those for its own.
    pub fn queue_changes(&self) -> broadcast::Receiver<String> {
        self.queue_changed.subscribe()
    }

    /// The session's sends not yet delivered, and those that ended unsent.
    pub async fn queued(&self, session: &str) -> Option<(Value, bool)> {
        let receipts = self.receipts.clone()?;
        let (session, now) = (session.to_owned(), (self.clock)());
        match tokio::task::spawn_blocking(move || receipts.queue().list(&session, now)).await {
            Ok(Ok(list)) => list,
            _ => {
                tracing::warn!(target: "app_send", reason = "queued_sends_unreadable", "queued app sends could not be read");
                None
            }
        }
    }

    /// Put the session's queued and unsent sends on an app reply (`queuedSends`).
    pub async fn attach_queued(&self, reply: &mut Value, session: &str) {
        let list = self.queued(session).await;
        super::send_queue::attach(reply, list);
    }

    /// Remove expired receipts in bounded chunks, every [`PRUNE_INTERVAL`].
    fn spawn_pruning(&self) -> Option<tokio::task::JoinHandle<()>> {
        let receipts = self.receipts.clone()?;
        let clock = self.clock.clone();
        Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(PRUNE_INTERVAL);
            loop {
                tick.tick().await;
                let receipts = receipts.clone();
                let now = clock();
                let _ = tokio::task::spawn_blocking(move || {
                    for _ in 0..PRUNE_CHUNKS_PER_PASS {
                        match receipts.prune(now) {
                            Ok(removed) if removed == super::send_store::PRUNE_CHUNK => {}
                            Ok(_) => break,
                            Err(error) => {
                                tracing::warn!(target: "app_send", reason = "receipt_prune_failed", %error, "app send receipt pruning failed");
                                break;
                            }
                        }
                    }
                    for _ in 0..PRUNE_CHUNKS_PER_PASS {
                        match receipts.queue().prune(now, super::send_store::PRUNE_CHUNK) {
                            Ok(removed) if removed == super::send_store::PRUNE_CHUNK => {}
                            Ok(_) => break,
                            Err(error) => {
                                tracing::warn!(target: "app_send", reason = "queue_prune_failed", %error, "unsent app send pruning failed");
                                break;
                            }
                        }
                    }
                })
                .await;
            }
        }))
    }

    fn lock(&self, session: &str, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.guard();
        locks.retain(|_, lock| lock.strong_count() > 0);
        let slot = locks.entry((session.into(), id.into())).or_default();
        match slot.upgrade() {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                *slot = Arc::downgrade(&lock);
                lock
            }
        }
    }

    /// Answer one admitted send that names no device; it can name no file either.
    pub async fn send(&self, params: Value) -> Result<Value, SendRefusal> {
        self.send_from(None, params).await
    }

    /// Answer one admitted send from `device`, the fingerprint the route admitted
    /// the caller with: the files a send names must be uploads that device opened.
    /// `Err` is a refusal the route sends as an error; `Ok` is a receipt, delivered
    /// or not.
    pub async fn send_from(
        &self,
        device: Option<&str>,
        params: Value,
    ) -> Result<Value, SendRefusal> {
        let request = SendRequest::parse(&params)?;
        let content = if request.attachments().is_empty() {
            request.content()
        } else {
            let Some(uploads) = self.uploads.clone() else {
                return Err(SendRefusal::new(
                    "provider_unavailable",
                    "upload storage is unavailable",
                ));
            };
            let Some(device) = device else {
                return Err(SendRefusal::new(
                    super::uploads::codes::UPLOAD_NOT_YOURS,
                    "only the device that uploaded a file may send it",
                ));
            };
            let (device, session, ids) = (
                device.to_owned(),
                request.session.clone(),
                request.attachments().to_vec(),
            );
            let digests =
                tokio::task::spawn_blocking(move || uploads.digests(&device, &session, &ids))
                    .await
                    .map_err(|error| store_error(error.to_string()))?
                    .map_err(|error| SendRefusal {
                        code: error.code,
                        message: error.message,
                        detail: error.detail,
                    })?;
            request.content_by_bytes(&digests)
        };
        let requested = request.mode.unwrap_or(Mode::Queue);
        let digest = format!("{:x}", Sha256::digest(content.to_string()));
        // Concurrent attempts for one message are answered one after the other,
        // so the second sees the first one's receipt instead of racing it.
        let lock = request
            .client_message_id
            .as_deref()
            .map(|id| self.lock(&request.session, id));
        let _guard = match &lock {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        if let Some(id) = request.client_message_id.as_deref() {
            if let Some(stored) = self.stored(&request.session, id)? {
                self.targets
                    .validate(&request.session, request.harness.as_deref())
                    .await
                    .map_err(|error| SendRefusal {
                        code: error.code,
                        message: error.message,
                        detail: error.detail,
                    })?;
                if stored.digest != digest || stored.mode != requested.as_str() {
                    return Err(conflict());
                }
                crate::app_reader::observe::answer(format!(
                    "mode={} duplicate",
                    requested.as_str()
                ));
                let mut receipt = stored.receipt;
                receipt["duplicate"] = json!(true);
                return Ok(receipt);
            }
        }

        let target = match self
            .targets
            .open(&request.session, request.harness.as_deref())
            .await
        {
            Ok(target) => target,
            Err(error) if error.code == "session_on_other_machine" => {
                return Err(SendRefusal {
                    code: error.code,
                    message: error.message,
                    detail: error.detail,
                });
            }
            Err(error) if error.code == "session_unknown" => {
                // Keep `detail.runner`, the runner Entorhinal records (null when none),
                // as reads do, so the app can route the session to whoever serves it.
                return Err(SendRefusal {
                    code: codes::NOT_FOUND,
                    message: format!(
                        "no runner this module sends to hosts session {}",
                        request.session
                    ),
                    detail: error.detail,
                });
            }
            Err(error) if error.code == "invalid_provider_reply" => {
                if error
                    .detail
                    .as_ref()
                    .is_some_and(|detail| detail["field"] == "heldOn")
                {
                    return Err(SendRefusal {
                        code: error.code,
                        message: error.message,
                        detail: error.detail,
                    });
                }
                return Err(SendRefusal::new("provider_unavailable", error.message));
            }
            Err(error) => return Ok(not_sent(None, &request, &error.message)),
        };

        let head_id = runner_send_id(target.reference(), &request, Role::Head);
        let head = match target.call("session.head", json!({})).await {
            Ok(head) => head,
            Err(RunnerError::Unreachable(message)) => {
                return Ok(not_sent(Some(&head_id), &request, &message))
            }
            Err(RunnerError::Refused {
                code,
                message,
                detail,
            }) => return Err(refused(&code, &message, detail.as_ref())),
        };
        let meta: HeadMeta = serde_json::from_value(head.clone()).map_err(|error| {
            SendRefusal::new(
                "invalid_provider_reply",
                format!("runner head reply: {error}"),
            )
        })?;
        // Files need the description too: only a runner declaring `attachments`
        // takes them.
        let describe: Option<RoleDescribe> =
            if meta.sender.is_none() || !request.attachments().is_empty() {
                target
                    .call("role.describe", json!({}))
                    .await
                    .ok()
                    .and_then(|value| serde_json::from_value(value).ok())
            } else {
                None
            };
        let grant = super::broca::send_grant(&meta, describe.as_ref(), false);
        let trial = matches!(&grant, SendGrant::Unknown);
        // A worker's grant lacks idle_start. Choose its steer before sending:
        // runners may reject a missing model before checking a forbidden delivery.
        let worker = match grant {
            SendGrant::Granted(sender) => {
                let modes = sender.get("modes").and_then(Value::as_array);
                let has = |mode: &str| modes.is_some_and(|modes| modes.iter().any(|m| m == mode));
                let worker = !has("idle_start") && has("steer");
                if (!has("idle_start") && !worker)
                    || (requested == Mode::Interrupt
                        && (worker || sender.get("interrupt") != Some(&Value::Bool(true))))
                {
                    return Err(SendRefusal::new(
                        "enqueue_not_authorized",
                        "the session's sender grant does not authorize this delivery",
                    ));
                }
                worker
            }
            SendGrant::MayNotSend => {
                return Err(SendRefusal::new(
                    "enqueue_not_authorized",
                    "the session does not grant this sender permission to send",
                ));
            }
            SendGrant::Unknown => false,
        };
        let content = match self
            .prompt_content(target.as_ref(), &request, &meta, describe.as_ref(), device)
            .await?
        {
            Ok(content) => content,
            Err(message) => return Ok(not_sent(Some(&head_id), &request, &message)),
        };
        let (send_id, reply, follow_until) = if worker {
            match self
                .worker_steer(target.as_ref(), &request, &head, &content)
                .await?
            {
                Ok(sent) => sent,
                Err(message) => return Ok(not_sent(None, &request, &message)),
            }
        } else {
            let delivery = match requested {
                Mode::Queue => Planned::Order(DeliveryOrder::new(vec![Order::IdleStart])),
                Mode::Steer => {
                    Planned::Order(DeliveryOrder::new(vec![Order::Steer, Order::IdleStart]))
                }
                Mode::Interrupt => Planned::String(Delivery::Interrupt),
            };
            let body = runner_body(&request, &head_id, &delivery, &content)?;
            let outcome = target.call("session.send", body).await;
            let worker = trial
                && matches!(&outcome, Err(RunnerError::Refused { code, detail, .. })
            if code == "invalid_params" && names_delivery(detail.as_ref()));
            // A worker takes steers only, and a mode the app did not name is a queue:
            // both become the worker steer. An interrupt is never turned into a steer:
            // the runner's refusal of it is the answer, since only the process that
            // owns the worker may interrupt it.
            if worker && requested != Mode::Interrupt {
                match self
                    .worker_steer(target.as_ref(), &request, &head, &content)
                    .await?
                {
                    Ok(sent) => sent,
                    Err(message) => return Ok(not_sent(None, &request, &message)),
                }
            } else {
                match outcome {
                    Ok(reply) => (head_id, reply, None),
                    Err(RunnerError::Unreachable(message)) => {
                        return Ok(not_sent(Some(&head_id), &request, &message))
                    }
                    Err(RunnerError::Refused {
                        code,
                        message,
                        detail,
                    }) => return Err(refused(&code, &message, detail.as_ref())),
                }
            }
        };
        let reply: SendReply = serde_json::from_value(reply).map_err(|error| {
            SendRefusal::new(
                "invalid_provider_reply",
                format!("runner send reply: {error}"),
            )
        })?;
        let receipt = match reply.state.as_str() {
            states::EXPIRED => steer_expired_for(&send_id, request.client_message_id.as_deref()),
            states::SUPERSEDED | states::TEMPLATE_PART_GONE => json!({
                "ok": false, "effectId": send_id, "code": reply.state, "terminal": true,
            }),
            _ => accepted(&send_id, &request),
        };
        // The log line names the mode the app asked for, how it went out (to a
        // head, or as a worker's steer) and the runner's state, so a trace of
        // "did this steer reach a running turn" doesn't need the runner's store.
        crate::app_reader::observe::answer(format!(
            "mode={} as={} state={}",
            requested.as_str(),
            if follow_until.is_some() {
                "worker_steer"
            } else {
                "head"
            },
            reply.state
        ));
        if receipt["ok"] == true {
            self.mark_sent(request.attachments()).await;
        }
        // A steer the runner holds for its next boundary answers `pending`; one it
        // already rendered answers the run's state with `delivered`.
        let held = follow_until
            .as_ref()
            .filter(|_| reply.state == states::PENDING && !settled(&reply))
            .map(|(until, _)| *until);
        // What the follower watches until the message is stored: a held worker steer
        // until its run ends, and a head send for as long as the runner holds it.
        let follow = match follow_until {
            Some((_, run_id)) => held.map(|until| Follow::Steer {
                until,
                run_id: Some(run_id),
            }),
            None => (head_reply_fate(&reply) == Fate::Waiting).then(|| Follow::Head {
                give_up_at: (self.clock)().saturating_add(HEAD_HOLD.as_millis() as i64),
            }),
        };
        if let (Some(id), Some(receipts)) = (&request.client_message_id, &self.receipts) {
            let (session, id, row) = (request.session.clone(), id.clone(), receipt.clone());
            let receipts = receipts.clone();
            let mode = requested.as_str();
            let now = (self.clock)();
            let sid = send_id.clone();
            let run_id = match &follow {
                Some(Follow::Steer { run_id, .. }) => run_id.clone(),
                _ => None,
            };
            let queued = follow.as_ref().map(|_| super::send_queue::Queued {
                session: session.clone(),
                client_message_id: id.clone(),
                harness: request.harness.clone(),
                send_id: sid.clone(),
                accepted_at: now,
                steer_until: match &follow {
                    Some(Follow::Steer { until, .. }) => Some(*until),
                    _ => None,
                },
            });
            tokio::task::spawn_blocking(move || {
                receipts.record(
                    &session,
                    &id,
                    &digest,
                    mode,
                    &sid,
                    &row,
                    held.zip(run_id.as_deref()),
                    now,
                )?;
                if let Some(queued) = queued {
                    receipts.queue().queue(&queued)?;
                }
                Ok::<_, crate::Error>(())
            })
            .await
            .map_err(|error| store_error(error.to_string()))?
            .map_err(|error| store_error(error.to_string()))?;
            if follow.is_some() {
                let _ = self.queue_changed.send(request.session.clone());
            }
        }
        if let Some(follow) = follow {
            self.follow(
                target,
                request.session.clone(),
                request.client_message_id.clone(),
                send_id,
                follow,
            );
        }
        Ok(receipt)
    }

    fn stored(
        &self,
        session: &str,
        id: &str,
    ) -> Result<Option<super::send_store::Stored>, SendRefusal> {
        let Some(receipts) = &self.receipts else {
            return Ok(None);
        };
        receipts
            .get(session, id, (self.clock)())
            .map_err(|error| store_error(error.to_string()))
    }

    /// The send's `content`: each file the message names, copied into the runner
    /// session in the message's order. A file already copied for this runner
    /// session is reused while the runner still holds it committed, so a resent
    /// message never uploads twice. Every refusal comes before the send writes
    /// anything. The inner `Err` is a call that did not reach the runner.
    async fn prompt_content(
        &self,
        target: &dyn SendTarget,
        request: &SendRequest,
        meta: &HeadMeta,
        describe: Option<&RoleDescribe>,
        device: Option<&str>,
    ) -> Result<Result<Vec<ContentPart>, String>, SendRefusal> {
        if request.attachments().is_empty() {
            return Ok(Ok(Vec::new()));
        }
        let Some(uploads) = self.uploads.clone() else {
            return Err(SendRefusal::new(
                "provider_unavailable",
                "upload storage is unavailable",
            ));
        };
        // A runner without the group ignores `content` and would drop the files.
        let Some(describe) = describe.filter(|describe| describe.declares("attachments")) else {
            return Err(SendRefusal::new(
                MEDIA_UNSUPPORTED,
                "this session's runner takes no files in the prompt; place them in the project",
            ));
        };
        // Only a runner declaring `media_only_send` stores a message of files
        // alone. Elsewhere the app's own words are needed, and none are made up.
        if request.text.trim().is_empty() && !describe.declares("media_only_send") {
            return Err(SendRefusal::new(
                "invalid_request",
                "a message with files needs text too",
            ));
        }
        let Some(device) = device else {
            return Err(SendRefusal::new(
                super::uploads::codes::UPLOAD_NOT_YOURS,
                "only the device that uploaded a file may send it",
            ));
        };
        let input = meta.input.as_ref().map(|input| InputCapability {
            attachments: match input.carries_attachments() {
                Some(true) => Attachments::Yes,
                Some(false) => Attachments::No,
                None => Attachments::Unknown,
            },
            mime_types: input.mime_types.clone(),
            modalities: input.modalities.clone(),
            ..InputCapability::default()
        });
        let mut files = Vec::with_capacity(request.attachments().len());
        for id in request.attachments() {
            let (uploads, device, session, id) = (
                uploads.clone(),
                device.to_owned(),
                request.session.clone(),
                id.clone(),
            );
            let file =
                tokio::task::spawn_blocking(move || uploads.prompt_file(&device, &session, &id))
                    .await
                    .map_err(|error| store_error(error.to_string()))?
                    .map_err(|error| SendRefusal {
                        code: error.code,
                        message: error.message,
                        detail: error.detail,
                    })?;
            if input.as_ref().and_then(|input| input.accepts(&file.mime)) == Some(false) {
                return Err(SendRefusal {
                    detail: Some(json!({"attachment_id": file.upload_id, "mime": file.mime})),
                    ..SendRefusal::new(
                        MEDIA_UNSUPPORTED,
                        format!("this session's model can't take {} files", file.mime),
                    )
                });
            }
            files.push(file);
        }
        // Counted over the files this send names, all of them already uploaded,
        // before anything is copied to the runner.
        let model = meta
            .model
            .as_ref()
            .and_then(|model| serde_json::to_value(model).ok());
        let limits = crate::display::model_names::attachment_limits(
            self.model_names.get(),
            model.as_ref(),
            meta.input.as_ref().and_then(|input| input.max_image_bytes),
            meta.input
                .as_ref()
                .and_then(|input| input.max_image_dimension)
                .map(u64::from),
        )
        .await;
        let images = files
            .iter()
            .filter(|file| file.mime.starts_with("image/"))
            .count() as u64;
        let others = files.len() as u64 - images;
        if images > limits.max_images || others > limits.max_files {
            return Err(SendRefusal {
                detail: Some(json!({"maxImages": limits.max_images,
                    "maxFiles": limits.max_files, "images": images, "files": others})),
                ..SendRefusal::new(
                    TOO_MANY_FILES,
                    format!(
                        "a message to this session may carry {} images and {} other files",
                        limits.max_images, limits.max_files
                    ),
                )
            });
        }
        let reference = target.reference();
        let runner = format!("{}/{}", reference.runner, reference.session);
        let mut content = Vec::with_capacity(files.len());
        for file in files {
            // The copy this upload already has in the runner session, or else one
            // an earlier attempt made of the same bytes under another upload id.
            let mut kept = file
                .runner_upload
                .as_ref()
                .filter(|kept| kept.runner == runner)
                .map(|kept| kept.upload_id.clone());
            if kept.is_none() {
                let (uploads, device, session, file, runner) = (
                    uploads.clone(),
                    device.to_owned(),
                    request.session.clone(),
                    file.clone(),
                    runner.clone(),
                );
                kept = tokio::task::spawn_blocking(move || {
                    uploads.runner_copy(&device, &session, &file, &runner)
                })
                .await
                .map_err(|error| store_error(error.to_string()))?
                .map_err(|error| store_error(error.to_string()))?;
            }
            let mut copied = None;
            if let Some(kept) = kept {
                match target
                    .call("session.upload_status", json!({"upload_id": kept}))
                    .await
                {
                    Ok(status) if status["state"] == "committed" => copied = Some(kept),
                    Ok(_) | Err(RunnerError::Refused { .. }) => {}
                    Err(RunnerError::Unreachable(message)) => return Ok(Err(message)),
                }
            }
            let upload_id = match copied {
                Some(upload_id) => upload_id,
                None => match self.copy_to_runner(target, &file).await? {
                    Ok(upload_id) => upload_id,
                    Err(message) => return Ok(Err(message)),
                },
            };
            let recorded = RunnerUpload {
                runner: runner.clone(),
                upload_id: upload_id.clone(),
            };
            let (uploads, id) = (uploads.clone(), file.upload_id.clone());
            tokio::task::spawn_blocking(move || uploads.record_runner_upload(&id, Some(&recorded)))
                .await
                .map_err(|error| store_error(error.to_string()))?
                .map_err(|error| store_error(error.to_string()))?;
            content.push(ContentPart::upload(upload_id));
        }
        Ok(Ok(content))
    }

    /// Upload one prompt file to the runner session: begin, append in pieces of
    /// the contract's largest chunk, commit. Answers the runner's upload id. The
    /// inner `Err` is a call that did not reach the runner; an upload left open
    /// that way is dropped by the runner after its hour.
    async fn copy_to_runner(
        &self,
        target: &dyn SendTarget,
        file: &super::uploads::PromptFile,
    ) -> Result<Result<String, String>, SendRefusal> {
        let bytes = tokio::fs::read(&file.blob)
            .await
            .map_err(|error| store_error(format!("upload {}: {error}", file.upload_id)))?;
        if bytes.len() as u64 != file.size {
            return Err(store_error(format!(
                "upload {} holds {} of its {} bytes",
                file.upload_id,
                bytes.len(),
                file.size
            )));
        }
        let call = |method: &'static str, params: Value| async move {
            match target.call(method, params).await {
                Ok(reply) => Ok(Ok(reply)),
                Err(RunnerError::Unreachable(message)) => Ok(Err(message)),
                Err(RunnerError::Refused {
                    code,
                    message,
                    detail,
                }) => Err(refused(&code, &message, detail.as_ref())),
            }
        };
        let begun = match call(
            "session.upload_begin",
            json!({"mime": file.mime, "bytes": file.size, "sha256": file.sha256}),
        )
        .await?
        {
            Ok(reply) => reply,
            Err(message) => return Ok(Err(message)),
        };
        let Some(upload_id) = begun["upload_id"].as_str().filter(|id| !id.is_empty()) else {
            return Err(SendRefusal::new(
                "invalid_provider_reply",
                "the runner's upload_begin reply names no upload_id",
            ));
        };
        let upload_id = upload_id.to_owned();
        for (index, piece) in bytes.chunks(CHUNK_MAX).enumerate() {
            let offset = (index * CHUNK_MAX) as u64;
            let appended = json!({"upload_id": upload_id, "offset": offset, "data": crate::base64::encode(piece)});
            if let Err(message) = call("session.upload_append", appended).await? {
                return Ok(Err(message));
            }
        }
        match call("session.upload_commit", json!({"upload_id": upload_id})).await? {
            Ok(committed) if committed["sha256"] == file.sha256.as_str() => Ok(Ok(upload_id)),
            Ok(_) => Err(SendRefusal::new(
                "invalid_provider_reply",
                "the runner committed an upload with a different sha256",
            )),
            Err(message) => Ok(Err(message)),
        }
    }

    /// Mark each file of an accepted send as sent: the runner holds its bytes now.
    /// A failure leaves the blob to expire with the upload; it is never resent.
    async fn mark_sent(&self, attachments: &[String]) {
        let Some(uploads) = self.uploads.clone() else {
            return;
        };
        if attachments.is_empty() {
            return;
        }
        let ids = attachments.to_vec();
        let outcome = tokio::task::spawn_blocking(move || {
            ids.iter().try_for_each(|id| uploads.mark_sent(id))
        })
        .await;
        if !matches!(outcome, Ok(Ok(()))) {
            tracing::warn!(target: "app_send", reason = "upload_sent_unrecorded", "a sent upload could not be recorded as sent");
        }
    }

    /// A worker message joins the run in progress or is refused at once: the
    /// session head names the active run. Its lifetime bounds the steer; expiry
    /// after [`WORKER_STEER_EXPIRY`] is only a backstop. The inner `Err` is a call
    /// that did not reach the runner.
    #[allow(clippy::type_complexity)]
    async fn worker_steer(
        &self,
        target: &dyn SendTarget,
        request: &SendRequest,
        head: &Value,
        content: &[ContentPart],
    ) -> Result<Result<(String, Value, Option<(i64, String)>), String>, SendRefusal> {
        // A held worker steer is followed across restarts from its stored row, which
        // names the session but not its harness, so recovery could only reopen a Broca
        // worker. Until the row carries the harness, other harnesses' workers refuse.
        if request
            .harness
            .as_deref()
            .is_some_and(|harness| harness != "broca")
        {
            return Err(SendRefusal::new(
                "unsupported",
                "steering a worker on this harness from the app is not served yet",
            ));
        }
        if head["last_run_state"]["state"].as_str()
            != Some(cortexkit_role_llm_runner::run::states::ACTIVE)
        {
            return Err(SendRefusal::new(
                codes::TASK_PAUSED,
                "the worker has no run in progress; a message to a worker can only join a running turn",
            ));
        }
        let run_id = head["last_run_state"]["run_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                SendRefusal::new("invalid_provider_reply", "active worker head has no run_id")
            })?
            .to_owned();
        let send_id = runner_send_id(target.reference(), request, Role::Worker);
        let expires = (self.clock)().saturating_add(WORKER_STEER_EXPIRY.as_millis() as i64);
        let steer = Planned::Order(
            DeliveryOrder::new(vec![Order::Steer]).with_expires_at_ms(expires.max(0) as u64),
        );
        let body = runner_body(request, &send_id, &steer, content)?;
        match target.call("session.send", body).await {
            Ok(reply) => Ok(Ok((send_id, reply, Some((expires, run_id))))),
            Err(RunnerError::Unreachable(message)) => Ok(Err(message)),
            Err(RunnerError::Refused {
                code,
                message,
                detail,
            }) => Err(refused(&code, &message, detail.as_ref())),
        }
    }

    /// Follow a held worker steer until it lands or its target run ends. A pending
    /// steer is retracted when that run ends, so it cannot join a later turn.
    ///
    /// Before the deadline an unanswered check just waits for the next one. After
    /// it, the send is settled from one answer: delivered keeps the first receipt,
    /// expired or never landed (still pending, or unknown to the runner) becomes
    /// `steer_expired`. A runner that cannot be asked leaves the row followed, and
    /// the recovery sweep tries again; no outcome is ever assumed.
    ///
    /// A head send is followed the same way for as long as the runner holds it:
    /// delivered removes it from the session's queued list, and any undelivered end
    /// lists it as unsent with the runner's reason. Its receipt is left as accepted.
    fn follow(
        &self,
        target: Arc<dyn SendTarget>,
        session: String,
        client_message_id: Option<String>,
        send_id: String,
        follow: Follow,
    ) {
        let key = (session.clone(), send_id.clone());
        if !self.following.guard().insert(key.clone()) {
            return;
        }
        let following = self.following.clone();
        let receipts = self.receipts.clone();
        let late = self.late.clone();
        let changed = self.queue_changed.clone();
        let clock = self.clock.clone();
        let mut tasks = self.tasks.guard();
        while tasks.try_join_next().is_some() {}
        tasks.spawn(async move {
            let settle_after = match &follow {
                Follow::Steer { until, .. } => {
                    until.saturating_add(SETTLE_GRACE.as_millis() as i64)
                }
                Follow::Head { give_up_at } => *give_up_at,
            };
            let mut retractable = None;
            // A row recovered after its deadline is settled at once.
            let mut wait = clock() <= settle_after;
            loop {
                if wait {
                    tokio::time::sleep(FOLLOW_INTERVAL).await;
                }
                wait = true;
                let past = clock() > settle_after;
                let answer = target
                    .call(SESSION_SEND_STATUS, json!({"send_id": send_id}))
                    .await;
                // A head send's answer, kept to ask why its run ended without it.
                let head_answer = match &follow {
                    Follow::Head { .. } => answer.as_ref().ok().cloned(),
                    Follow::Steer { .. } => None,
                };
                let fate = match &follow {
                    Follow::Steer { run_id, .. } => {
                        let submission = answer
                            .as_ref()
                            .ok()
                            .and_then(|reply| reply["submission_id"].as_str())
                            .map(str::to_owned);
                        match steer_fate(answer, past) {
                            Fate::Waiting => {
                                pending_steer_fate(
                                    target.as_ref(),
                                    &send_id,
                                    run_id.as_deref(),
                                    submission.as_deref(),
                                    &mut retractable,
                                    past,
                                )
                                .await
                            }
                            fate => fate,
                        }
                    }
                    Follow::Head { .. } => match head_fate(answer.as_ref()) {
                        Fate::Waiting if past => Fate::Unsent(HOLD_EXPIRED.into()),
                        fate => fate,
                    },
                };
                let ended = match &fate {
                    Fate::Expired => {
                        let mut receipt = steer_expired_for(&send_id, client_message_id.as_deref());
                        settle(
                            &receipts,
                            &session,
                            client_message_id.as_deref(),
                            Some(&receipt),
                        )
                        .await;
                        receipt["session"] = json!(session);
                        let _ = late.send(LateOutcome {
                            session: session.clone(),
                            notice: json!({"sendOutcome": receipt}),
                        });
                        Some(Some(codes::STEER_EXPIRED.to_owned()))
                    }
                    Fate::Delivered => {
                        if matches!(follow, Follow::Steer { .. }) {
                            settle(&receipts, &session, client_message_id.as_deref(), None).await;
                        }
                        Some(None)
                    }
                    Fate::Unsent(reason) => {
                        if matches!(follow, Follow::Steer { .. }) {
                            settle(&receipts, &session, client_message_id.as_deref(), None).await;
                        }
                        Some(Some(reason.clone()))
                    }
                    Fate::Unanswered if past => break,
                    Fate::Waiting | Fate::Unanswered => None,
                };
                if let Some(reason) = ended {
                    let now = clock();
                    let run_end = match reason {
                        Some(_) => ended_run(target.as_ref(), head_answer.as_ref(), now).await,
                        None => None,
                    };
                    if dequeue(
                        &receipts,
                        &session,
                        client_message_id.as_deref(),
                        reason.as_deref(),
                        run_end,
                        now,
                    )
                    .await
                    {
                        let _ = changed.send(session.clone());
                    }
                    break;
                }
            }
            following.guard().remove(&key);
        });
    }

    /// Resume following every stored worker steer no task here follows yet: those
    /// held when the module last stopped, and those a sweep could not settle
    /// because the runner did not answer. Bounded by [`RECOVERY_BATCH`] per sweep.
    async fn recover(&self) {
        let Some(receipts) = self.receipts.clone() else {
            return;
        };
        let rows = match tokio::task::spawn_blocking(move || receipts.followed(RECOVERY_BATCH))
            .await
        {
            Ok(Ok(rows)) => rows,
            _ => {
                tracing::warn!(target: "app_send", reason = "followed_sends_unreadable", "held app sends could not be read");
                return;
            }
        };
        for row in rows {
            let key = (row.session.clone(), row.send_id.clone());
            if self.following.guard().contains(&key) {
                continue;
            }
            // An unreachable lookup leaves the row for the next sweep. Only Broca workers
            // are followed (`worker_steer`), so the row's session is a Broca one.
            let Ok(target) = self.targets.open(&row.session, None).await else {
                continue;
            };
            self.follow(
                target,
                row.session,
                Some(row.client_message_id),
                row.send_id,
                Follow::Steer {
                    until: row.follow_until,
                    run_id: row.run_id,
                },
            );
        }
        // Then the oldest queued head sends, up to [`RECOVERY_BATCH`], that no task
        // here follows: those queued when the module last stopped, and those whose
        // runner didn't answer before their follower gave up.
        let Some(receipts) = self.receipts.clone() else {
            return;
        };
        let queued = match tokio::task::spawn_blocking(move || {
            receipts.queue().queued_heads(RECOVERY_BATCH)
        })
        .await
        {
            Ok(Ok(rows)) => rows,
            _ => {
                tracing::warn!(target: "app_send", reason = "queued_sends_unreadable", "queued app sends could not be read");
                return;
            }
        };
        for row in queued {
            let key = (row.session.clone(), row.send_id.clone());
            if self.following.guard().contains(&key) {
                continue;
            }
            let Ok(target) = self
                .targets
                .open(&row.session, row.harness.as_deref())
                .await
            else {
                continue;
            };
            self.follow(
                target,
                row.session,
                Some(row.client_message_id),
                row.send_id,
                Follow::Head {
                    give_up_at: row.accepted_at.saturating_add(HEAD_HOLD.as_millis() as i64),
                },
            );
        }
    }

    /// Start the background work a serving module runs: receipt pruning, and the
    /// recovery sweep that resumes held worker steers, first at once and then every
    /// [`RECOVERY_INTERVAL`]. Neither blocks the caller. The sweep holds the sender
    /// weakly, so it ends with it. Each task comes with a name for health reports.
    pub fn start(self: &Arc<Self>) -> Vec<(&'static str, tokio::task::JoinHandle<()>)> {
        let mut started = Vec::new();
        if let Some(pruning) = self.spawn_pruning() {
            started.push(("send receipt pruning", pruning));
        }
        let sender = Arc::downgrade(self);
        let follow_up = tokio::spawn(async move {
            loop {
                match sender.upgrade() {
                    Some(sender) => sender.recover().await,
                    None => return,
                }
                tokio::time::sleep(RECOVERY_INTERVAL).await;
            }
        });
        started.push(("held send follow-up", follow_up));
        started
    }
}

/// What a follower watches a send for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Follow {
    /// A held worker steer, bound to its run (absent only for legacy receipts).
    Steer { until: i64, run_id: Option<String> },
    /// A head send, for as long as the runner holds it, and at most until
    /// `give_up_at`, after which a send still waiting is listed as unsent.
    Head { give_up_at: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Fate {
    Delivered,
    /// A worker steer that expired before a run took it.
    Expired,
    /// A head send that ended undelivered, with the reason the phone shows.
    Unsent(String),
    Waiting,
    Unanswered,
}

/// A held worker steer's fate from one `session.send_status` answer. Past its deadline, an
/// answer of `pending` or an unknown send settles it as expired.
fn steer_fate(answer: Result<Value, RunnerError>, past: bool) -> Fate {
    match answer {
        Ok(reply) => match serde_json::from_value::<SendReply>(reply) {
            Ok(reply) if reply.state == states::EXPIRED => Fate::Expired,
            Ok(reply) if reply.state == "retracted" => Fate::Unsent(RUN_ENDED.into()),
            Ok(reply)
                if matches!(
                    reply.state.as_str(),
                    states::SUPERSEDED | states::TEMPLATE_PART_GONE
                ) =>
            {
                Fate::Unsent(reply.state)
            }
            Ok(reply) if reply.state != states::PENDING || settled(&reply) => Fate::Delivered,
            Ok(_) if past => Fate::Expired,
            Ok(_) => Fate::Waiting,
            Err(_) => Fate::Unanswered,
        },
        // The runner keeps no record of a send it never wrote.
        Err(RunnerError::Refused { code, .. })
            if code == cortexkit_role_llm_runner::errors::UNKNOWN_SEND && past =>
        {
            Fate::Expired
        }
        Err(_) => Fate::Unanswered,
    }
}

/// A pending steer may only join the run observed at send time. Retract serializes
/// with delivery; if delivery won, its durable status, not the retract reply,
/// decides whether the phone may show it as delivered.
async fn pending_steer_fate(
    target: &dyn SendTarget,
    send_id: &str,
    run_id: Option<&str>,
    submission_id: Option<&str>,
    retractable: &mut Option<bool>,
    past: bool,
) -> Fate {
    let Some(run_id) = run_id else {
        // Older receipts do not identify a run, so only their expiry is safe.
        return Fate::Waiting;
    };
    let Ok(head) = target.call("session.head", json!({})).await else {
        return Fate::Unanswered;
    };
    if serde_json::from_value::<HeadMeta>(head.clone()).is_err() {
        return Fate::Unanswered;
    }
    if head["last_run_state"]["run_id"] == run_id
        && head["last_run_state"]["state"] == cortexkit_role_llm_runner::run::states::ACTIVE
    {
        return Fate::Waiting;
    }
    if retractable.is_none() {
        let Ok(describe) = target.call("role.describe", json!({})).await else {
            return Fate::Unanswered;
        };
        let Some(groups) = describe["capabilities"].as_array() else {
            return Fate::Unanswered;
        };
        let supported = groups
            .iter()
            .any(|group| group == cortexkit_role_llm_runner::capabilities::RETRACT);
        *retractable = Some(supported);
        if !supported {
            tracing::warn!(target: "app_send", reason = "retract_unsupported", %send_id, %run_id,
                "worker steer cannot be withdrawn at run end; waiting for backstop expiry");
        }
    }
    if *retractable == Some(false) {
        return Fate::Waiting;
    }
    let Some(submission_id) = submission_id else {
        return Fate::Unanswered;
    };
    match target
        .call("session.retract", json!({"submission_id": submission_id}))
        .await
    {
        Ok(reply) if reply == "retracted" => Fate::Unsent(RUN_ENDED.into()),
        // Retract is owner-only, and a phone send reaches a worker as a plan-named
        // sender, so a runner that grants it only to the owner refuses it every
        // time. Stop asking, and let the steer settle at its deadline: the runner
        // never delivers it after `expires_at_ms`.
        Err(RunnerError::Refused { code, .. }) if code == "scope_owner_mismatch" => {
            *retractable = Some(false);
            tracing::warn!(target: "app_send", reason = "retract_refused", %code, %send_id, %run_id,
                "worker steer cannot be withdrawn at run end; waiting for its deadline");
            steer_fate(
                target
                    .call(SESSION_SEND_STATUS, json!({"send_id": send_id}))
                    .await,
                past,
            )
        }
        Ok(_) | Err(RunnerError::Refused { .. }) => steer_fate(
            target
                .call(SESSION_SEND_STATUS, json!({"send_id": send_id}))
                .await,
            past,
        ),
        Err(_) => Fate::Unanswered,
    }
}

/// A head send's fate from one `session.send_status` answer (or the send's own reply).
/// Delivered once its message is in a step or a turn, or handed to a run that can't
/// confirm it; unsent when the runner ended it undelivered, naming why; waiting while
/// it is queued or handed to the host without a confirmed landing.
fn head_fate(answer: Result<&Value, &RunnerError>) -> Fate {
    match answer {
        Ok(reply) => match serde_json::from_value::<SendReply>(reply.clone()) {
            Ok(reply) => head_reply_fate(&reply),
            Err(_) => Fate::Unanswered,
        },
        // A session the runner no longer has can't take the send.
        Err(RunnerError::Refused { code, .. })
            if matches!(
                code.as_str(),
                "session_unknown" | "not_found" | "session_ended"
            ) =>
        {
            Fate::Unsent(SESSION_ENDED.into())
        }
        Err(RunnerError::Refused { code, .. })
            if code == cortexkit_role_llm_runner::errors::UNKNOWN_SEND =>
        {
            Fate::Unsent(NOT_SENT_REASON.into())
        }
        Err(_) => Fate::Unanswered,
    }
}

fn head_reply_fate(reply: &SendReply) -> Fate {
    if settled(reply) {
        return Fate::Delivered;
    }
    match reply.state.as_str() {
        states::EXPIRED | states::SUPERSEDED | states::TEMPLATE_PART_GONE => {
            Fate::Unsent(reply.state.clone())
        }
        // The run that would have carried it ended without it.
        states::FINISHED => Fate::Unsent(
            reply
                .reason
                .clone()
                .unwrap_or_else(|| NOT_SENT_REASON.into()),
        ),
        _ => Fate::Waiting,
    }
}

/// Take a followed send off the queued list: delivered (`reason` none) removes it,
/// and an undelivered end keeps it as unsent, with the code its run ended with
/// when there is one. Whether the list changed.
async fn dequeue(
    receipts: &Option<Arc<Receipts>>,
    session: &str,
    id: Option<&str>,
    reason: Option<&str>,
    run_end: Option<super::send_queue::EndedRun>,
    now: i64,
) -> bool {
    let (Some(receipts), Some(id)) = (receipts.clone(), id) else {
        return false;
    };
    let (session, id, reason) = (session.to_owned(), id.to_owned(), reason.map(str::to_owned));
    let result = tokio::task::spawn_blocking(move || match reason {
        None => receipts.queue().delivered(&session, &id),
        Some(reason) => receipts
            .queue()
            .unsent(&session, &id, &reason, run_end.as_ref(), now),
    })
    .await;
    match result {
        Ok(Ok(changed)) => changed,
        _ => {
            tracing::warn!(target: "app_send", reason = "queue_settle_failed", "an app send's queued entry was not updated");
            false
        }
    }
}

/// Record a followed send's end: `receipt` replaces the first one when the outcome
/// changed it, and the row is no longer followed either way.
async fn settle(
    receipts: &Option<Arc<Receipts>>,
    session: &str,
    id: Option<&str>,
    receipt: Option<&Value>,
) {
    let (Some(receipts), Some(id)) = (receipts.clone(), id) else {
        return;
    };
    let (session, id, receipt) = (session.to_owned(), id.to_owned(), receipt.cloned());
    let result =
        tokio::task::spawn_blocking(move || receipts.settle(&session, &id, receipt.as_ref())).await;
    if !matches!(result, Ok(Ok(()))) {
        tracing::warn!(target: "app_send", reason = "receipt_settle_failed", "an app send's final outcome was not recorded");
    }
}

enum Planned {
    String(Delivery),
    Order(DeliveryOrder),
}

/// The runner key: derived from what is delivered, never from the attempt, so every
/// retry of one app message names the same send, and two sessions' messages never
/// share one.
///
/// Every mode to a head shares one key, so a later repeat in another mode is
/// refused by the runner instead of delivered again. A worker steer has its own
/// key: the head delivery tried first on a worker is refused without being
/// recorded, but keeping the two apart means a retry of that probe can never be
/// mistaken for a reuse of the steer's key.
pub fn runner_send_id(reference: &SessionRef, request: &SendRequest, role: Role) -> String {
    let (kind, id) = match (&request.client_message_id, &request.send_id) {
        (Some(id), _) => ("client_message_id", id.clone()),
        (None, Some(id)) => ("send_id", id.clone()),
        // Core's sends without either id were not idempotent either.
        (None, None) => ("attempt", uuid::Uuid::new_v4().to_string()),
    };
    let role = match role {
        Role::Head => "head",
        Role::Worker => "worker-steer",
    };
    // The namespace keeps Thalamus's name, which sent these messages before this
    // module: a message retried after the move derives the key it was first sent
    // under, so the runner recognizes the repeat. A JSON value's text is its
    // compact serialization, the bytes this digest has always covered.
    let digest = Sha256::digest(
        json!([
            "thalamus-app-send",
            reference.runner,
            reference.session,
            kind,
            id,
            role
        ])
        .to_string(),
    );
    format!("app-{:x}", digest)
}

/// The runner's `session.send` request. Its plain fields always serialize; were
/// that ever to fail, the send is refused before anything reaches the runner.
fn runner_body(
    request: &SendRequest,
    send_id: &str,
    delivery: &Planned,
    content: &[ContentPart],
) -> Result<Value, SendRefusal> {
    let mut body = RunnerSend::new(request.text.clone(), send_id);
    body = match delivery {
        Planned::String(mode) => body.with_delivery(*mode),
        Planned::Order(order) => body.with_delivery_order(order),
    };
    if let Some(id) = &request.client_message_id {
        body = body.with_client_message_id(id.clone());
    }
    // A message without files keeps its bytes: the member is left out, not empty.
    if !content.is_empty() {
        body = body.with_content(content.to_vec());
    }
    serde_json::to_value(&body)
        .map_err(|error| SendRefusal::new("internal", format!("cannot encode the send: {error}")))
}

/// Whether a refusal names `delivery` (`detail.field == "delivery"`).
///
/// The send reads an `invalid_params` refusal naming `delivery`, answered to the
/// head delivery it tries first, as "this session is a worker". That rests on one
/// assumption: core's plans grant a head's entry for this module `idle_start` and
/// `interrupt`, and a worker's entry neither, so only a worker's
/// plan refuses the head delivery this way. The refusal writes nothing, so the
/// probe is safe. No direct signal for the session's kind exists yet: Entorhinal's
/// session replies and the runner's reads carry none. If one is added, it should
/// replace this probe.
fn names_delivery(detail: Option<&Value>) -> bool {
    detail.and_then(|detail| detail["field"].as_str()) == Some("delivery")
}

/// Whether a reply says the send left the queue: rendered into a step or a turn,
/// or handed to a run that cannot confirm it (`unknown`, which is never re-sent).
fn settled(reply: &SendReply) -> bool {
    reply.delivered.as_ref().is_some_and(|delivered| {
        delivered.is_delivered()
            || delivered.r#as() == cortexkit_role_llm_runner::send::DeliveredAs::Unknown
    })
}

fn accepted(send_id: &str, request: &SendRequest) -> Value {
    let mut receipt = accepted_for(send_id, request.client_message_id.as_deref());
    if let Some(id) = &request.send_id {
        receipt["sendId"] = json!(id);
    }
    receipt
}

fn accepted_for(send_id: &str, client_message_id: Option<&str>) -> Value {
    let mut receipt = json!({"ok": true, "effectId": send_id});
    if let Some(id) = client_message_id {
        receipt["clientMessageId"] = json!(id);
        receipt["duplicate"] = json!(false);
    }
    receipt
}

fn steer_expired_for(send_id: &str, client_message_id: Option<&str>) -> Value {
    let mut receipt = json!({
        "ok": false, "effectId": send_id, "code": codes::STEER_EXPIRED, "terminal": true,
    });
    if let Some(id) = client_message_id {
        receipt["clientMessageId"] = json!(id);
        receipt["duplicate"] = json!(false);
    }
    receipt
}

fn not_sent(send_id: Option<&str>, request: &SendRequest, message: &str) -> Value {
    let mut receipt = json!({
        "ok": false, "code": codes::NOT_SENT, "terminal": false, "message": message,
    });
    if let Some(send_id) = send_id {
        receipt["effectId"] = json!(send_id);
    }
    if let Some(id) = &request.client_message_id {
        receipt["clientMessageId"] = json!(id);
    }
    receipt
}

fn conflict() -> SendRefusal {
    SendRefusal::new(
        codes::CONFLICT,
        "clientMessageId was already used for different text or attachments",
    )
}

fn store_error(message: String) -> SendRefusal {
    SendRefusal::new("store_error", message)
}

/// Map the runner's refusal onto the app's codes. Every refusal wrote nothing.
pub(super) fn refused(code: &str, message: &str, detail: Option<&Value>) -> SendRefusal {
    let field = detail.and_then(|detail| detail["field"].as_str());
    let said = format!("the runner answered {code}: {message}");
    match code {
        // The session's plan does not name this module as a sender, or does not
        // grant it the delivery asked for.
        "scope_owner_mismatch" => SendRefusal::new(codes::NOT_AUTHORIZED, said),
        "invalid_params" if field == Some("delivery") => {
            SendRefusal::new(codes::NOT_AUTHORIZED, said)
        }
        "send_id_reuse" => conflict(),
        "session_unknown" | "unknown_session" => SendRefusal::new(codes::NOT_FOUND, said),
        "delivery_unsupported" => SendRefusal::new(codes::UNSUPPORTED, said),
        "run_paused" => SendRefusal::new("run_paused", said),
        "context_ceiling" => SendRefusal::new("context_ceiling", said),
        // The model can't take a file's type. The app places the file in the
        // project instead (`upload.place`) and sends its path.
        "media_unsupported" => SendRefusal {
            detail: detail.cloned(),
            ..SendRefusal::new(MEDIA_UNSUPPORTED, said)
        },
        // An image above the model's limit, named in `detail`. The app places it
        // in the project, or encodes it smaller, and sends again.
        "media_too_large" => SendRefusal {
            detail: detail.cloned(),
            ..SendRefusal::new(MEDIA_TOO_LARGE, said)
        },
        // The runner lost a file this module copied to it. A resend under the same
        // `clientMessageId` copies it again.
        "upload_gone" | "unknown_upload" | "upload_not_committed" => {
            SendRefusal::new(UPLOAD_GONE, said)
        }
        _ => {
            let mut refusal = SendRefusal::new("runner_refused", said);
            refusal.detail = Some(runner_refused_detail(code, detail, crate::now_ms()));
            refusal
        }
    }
}

/// What the app needs to decide whether to resend a `runner_refused` send under the
/// same `clientMessageId`: `class`, `runnerCode` (the runner's own code) and
/// `retryAfterMs`, the wait before resending, when the runner named one.
///
/// `class` is `transient` when the runner contract says the same request may
/// succeed later, and `final` otherwise. Two kinds are transient. A plain retry
/// (`errors::is_retryable`). And an account wait (`errors::is_account_wait`): the
/// account that would serve the send is at its usage limit, or routing can't
/// choose an account yet, so the send must wait for the named time and must
/// never be shown as failed. An app holds any transient
/// refusal at least `retryAfterMs` before sending it again, so an account wait
/// needs no class of its own; its `runnerCode` lets the app say why it waits.
///
/// The wait is the runner's relative hint when it gives one (`retry_after_ms`,
/// `retry_after_s`, `retry_after_secs`), else the time until `resets_at_ms`, as
/// the contract orders them; `now_ms` is when the refusal arrived.
fn runner_refused_detail(code: &str, detail: Option<&Value>, now_ms: i64) -> Value {
    use cortexkit_role_llm_runner::errors::{is_account_wait, is_retryable};
    let class = if is_retryable(code) || is_account_wait(code) {
        "transient"
    } else {
        "final"
    };
    let mut out = json!({"class": class, "runnerCode": code});
    // A final refusal names no wait, even when its detail carries a hint: only
    // the outer code decides, and waiting can't cure it.
    if let Some(wait_ms) = detail
        .filter(|_| class == "transient")
        .and_then(|detail| named_wait_ms(detail, now_ms))
    {
        out["retryAfterMs"] = json!(wait_ms);
    }
    out
}

/// The wait a refusal's detail or a run's error names, in milliseconds from
/// `now_ms`: the runner's relative hint when it gives one (`retry_after_ms`,
/// `retry_after_s`, `retry_after_secs`), else the time until `resets_at_ms`, as
/// the contract orders them.
fn named_wait_ms(detail: &Value, now_ms: i64) -> Option<u64> {
    detail["retry_after_ms"]
        .as_u64()
        .or_else(|| {
            detail["retry_after_s"]
                .as_u64()
                .map(|secs| secs.saturating_mul(1000))
        })
        .or_else(|| {
            detail["retry_after_secs"]
                .as_f64()
                .filter(|secs| secs.is_finite() && *secs >= 0.0)
                .map(|secs| (secs * 1000.0).ceil() as u64)
        })
        .or_else(|| {
            let resets_at = detail["resets_at_ms"].as_i64()?;
            u64::try_from(resets_at.saturating_sub(now_ms).max(0)).ok()
        })
}

/// Why the run that would have carried a queued head send ended without it, when
/// the runner says it ended in an error (`finished`, reason `error`): the code of
/// that run's error from `run.status`, if the runner contract names it, and for
/// an account wait the wait it names, from `now_ms`. Asked once; `None` when the
/// runner names no run or no contract code, or doesn't answer, and the entry is
/// then listed unsent with its reason alone.
async fn ended_run(
    target: &dyn SendTarget,
    answer: Option<&Value>,
    now_ms: i64,
) -> Option<super::send_queue::EndedRun> {
    use cortexkit_role_llm_runner::errors::{is_account_wait, provider_codes};
    let reply: SendReply = serde_json::from_value(answer?.clone()).ok()?;
    if reply.state != states::FINISHED || reply.reason.as_deref() != Some("error") {
        return None;
    }
    let status = target
        .call(
            cortexkit_role_llm_runner::ops::RUN_STATUS,
            json!({"run_id": reply.run_id?}),
        )
        .await
        .ok()?;
    let error = &status["error"];
    let code = error["provider_code"]
        .as_str()
        .filter(|code| provider_codes::CODES.contains(code))?;
    let retry_after_ms = if is_account_wait(code) {
        named_wait_ms(error, now_ms).and_then(|wait| i64::try_from(wait).ok())
    } else {
        None
    };
    Some(super::send_queue::EndedRun {
        runner_code: code.to_owned(),
        retry_after_ms,
    })
}

pub mod stop;
pub mod stop_all;
#[cfg(test)]
mod tests;
