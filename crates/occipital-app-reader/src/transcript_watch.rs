//! Event-driven transcript notices with claim advancement only after delivery.
//!
//! Each notice answers the last claim the client actually received. Large replies
//! become a newest-page resync trimmed to the relay's byte bound. Session changes
//! coalesce behind a minimum interval; a slow sweep covers silent sources.

use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::time::Instant;

use crate::claim::{NewestPageFit, TranscriptPageSnapshot};
use crate::source::ReadError;

/// The largest notice body: the serialized UTF-8 JSON passed to `emit`.
/// The app relay accepts up to this many bytes per notice, including the bound.
pub const NOTICE_BOUND: usize = 196_608;

/// A transcript watch evaluates at most once per this interval; events arriving
/// meanwhile fold into the one evaluation that follows.
pub const TRANSCRIPT_MIN_INTERVAL: Duration = Duration::from_millis(350);

/// The transcript watch's one fallback: when no event has woken it for this
/// long, it evaluates once anyway, for a session whose host or runner sends
/// no events. Every event restarts the wait.
pub const TRANSCRIPT_SWEEP: Duration = Duration::from_secs(30);

/// After a transient failure the watch re-evaluates this long later, whether
/// or not another event arrives.
pub const RETRY_AFTER: Duration = Duration::from_secs(5);

/// Claim errors that end a transcript watch with that code.
const TRANSCRIPT_ENDING_CODES: &[&str] = &[
    "session_unknown",
    "identity_unsupported",
    "harness_mismatch",
    "invalid_claim",
    "claim_limit_zero",
    "claim_conflicts_with_activity_token",
    "claim_conflicts_with_history_cursor",
];

/// Read failures a later evaluation may not repeat.
const TRANSIENT_CODES: &[&str] = &[
    "store_error",
    "storage_error",
    "storage",
    "executor_error",
    "busy",
    "provider_unavailable",
    "observation_unstable",
    "transient",
];

/// Fields a claim reply's nested `resync` page never carries; the top-level
/// reply carries them instead.
const RESYNC_DECORATIONS: &[&str] = &[
    "activityToken",
    "discardActivityToken",
    "activity",
    "activityAtMs",
    "attachmentCapability",
    "timing",
    "version",
    "historyVersion",
    "model",
    "sender",
    crate::source::SENDER_UNKNOWN,
    "lastRun",
];

/// What the transport supplies to [`run`].
#[async_trait::async_trait]
pub trait TranscriptWatchHost: Send + Sync {
    /// What ends the watch when `session.transcript_page`'s admission now
    /// refuses the caller.
    type Refusal: Send;

    async fn read(&self, params: Value) -> Result<Value, ReadError>;

    fn trimmed_newest(
        &self,
        session: &str,
        version: &str,
        newest: &Value,
        fits: &mut (dyn FnMut(&Value) -> bool + Send),
    ) -> Option<NewestPageFit>;

    fn subscribe_changes(&self, session: &str) -> tokio::sync::watch::Receiver<u64>;

    /// Run `session.transcript_page`'s admission again and return the params it
    /// dispatches. Called once per evaluation.
    fn readmit(&self) -> Result<Value, Self::Refusal>;

    /// Emit one notice body. An error ends the watch with StreamEnd.
    async fn emit(&self, body: Vec<u8>) -> Result<(), ()>;

    /// Resolves once the watch must end with StreamEnd (cancel, route gone,
    /// module stopping).
    async fn stopped(&self);

    fn is_stopped(&self) -> bool;
}

/// How a transcript watch ended.
#[derive(Debug)]
pub enum TranscriptWatchEnd<R> {
    /// StreamEnd.
    Streamed,
    /// An Error end with this code, as the read reported it.
    Error { code: String, message: String },
    /// The read's admission now refuses the caller.
    Refused(R),
}

fn encode(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("a JSON value always serializes")
}

/// The message ids of a page's `messages`, in page order.
pub fn message_ids(page: &Value) -> Vec<String> {
    page.get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| message.get("messageId").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The material of a page for comparing two emitted pages.
fn page_material(page: &Value) -> Option<TranscriptPageSnapshot> {
    TranscriptPageSnapshot::from_page("", None, page.clone()).ok()
}

fn same_material(
    left: Option<&TranscriptPageSnapshot>,
    right: Option<&TranscriptPageSnapshot>,
) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.same_page_material(right),
        _ => false,
    }
}

/// What the watch does after one evaluation.
enum TranscriptStep<R> {
    /// A claim reply to emit (or suppress); `end_after` ends the watch with
    /// StreamEnd once it is sent.
    Reply {
        reply: Value,
        end_after: bool,
    },
    /// Emit nothing; evaluate again after the retry delay.
    Retry,
    End(TranscriptWatchEnd<R>),
}

fn error_end<R>(code: &str, message: impl Into<String>) -> TranscriptStep<R> {
    TranscriptStep::End(TranscriptWatchEnd::Error {
        code: code.to_string(),
        message: message.into(),
    })
}

/// Classify a transcript reply or error.
fn transcript_outcome<R>(result: Result<Value, ReadError>) -> Result<Value, TranscriptStep<R>> {
    let reply = match result {
        Ok(reply) => reply,
        Err(error) if TRANSIENT_CODES.contains(&error.code) => return Err(TranscriptStep::Retry),
        Err(error) => return Err(error_end(error.code, error.message)),
    };
    let Some(error) = reply.get("error") else {
        return Ok(reply);
    };
    let code = error.get("code").and_then(Value::as_str).unwrap_or("");
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if TRANSCRIPT_ENDING_CODES.contains(&code) {
        return Err(error_end(code, message));
    }
    // provider_unavailable keeps the watch open whatever its `terminal` value;
    // observation_unstable is transient.
    if matches!(code, "provider_unavailable" | "observation_unstable") {
        return Err(TranscriptStep::Retry);
    }
    if error.get("terminal").and_then(Value::as_bool) == Some(true) {
        return Err(error_end(code, message));
    }
    Err(TranscriptStep::Retry)
}

async fn transcript_read<H: TranscriptWatchHost>(
    host: &H,
    params: Value,
) -> Result<Value, ReadError> {
    host.read(params).await
}

/// Fit a fresh newest page by trimming its oldest end. An outside-window verdict
/// survives only while that page still has the held version; a rewrite expires it.
/// Version, lineage and metadata come from the fresh read.
async fn oversize_transcript<H: TranscriptWatchHost>(
    host: &H,
    session: &str,
    params: &Value,
    claim: &Map<String, Value>,
    outside_window: bool,
) -> TranscriptStep<H::Refusal> {
    let accepts_outside_window = match crate::claim::accepts_outside_window(params) {
        Ok(accepts) => accepts,
        Err(error) => return error_end(error.code, error.message),
    };
    let mut newest_params = params.clone();
    if let Some(object) = newest_params.as_object_mut() {
        object.remove("claim");
    }
    let newest = match transcript_outcome(transcript_read(host, newest_params).await) {
        Ok(newest) => newest,
        Err(step) => return step,
    };
    // A fresh read without a stable token cannot vouch for a claim.
    let Some(version) = newest
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return TranscriptStep::Retry;
    };
    let reply_with = |resync: Option<&Value>| {
        let verdict = if outside_window
            && accepts_outside_window
            && resync.is_some()
            && claim.get("heldVersion").and_then(Value::as_str) == Some(version.as_str())
        {
            "outside_window"
        } else {
            "cursor_expired"
        };
        let mut reply = json!({
            "claim": verdict,
            "answers": claim,
            "version": version,
            "diagnostics": newest
                .get("diagnostics")
                .cloned()
                .unwrap_or_else(|| crate::claim::diagnostics("durable", "scan")),
        });
        if let Some(resync) = resync {
            let mut resync = resync.clone();
            if let Some(object) = resync.as_object_mut() {
                for field in RESYNC_DECORATIONS {
                    object.remove(*field);
                }
            }
            reply["resync"] = resync;
        }
        for field in [
            "attachmentCapability",
            "acceptsAttachments",
            "acceptedMimeTypes",
            "acceptedModalities",
            "activity",
            "activityAtMs",
            "historyVersion",
            "model",
            "sender",
            crate::source::SENDER_UNKNOWN,
            "lastRun",
        ] {
            if let Some(value) = newest.get(field) {
                reply[field] = value.clone();
            }
        }
        reply
    };
    let fit = host.trimmed_newest(session, &version, &newest, &mut |page| {
        encode(&reply_with(Some(page))).len() <= NOTICE_BOUND
    });
    match fit {
        Some(NewestPageFit::Whole) => {
            let Ok(snapshot) =
                TranscriptPageSnapshot::from_page(version.clone(), None, newest.clone())
            else {
                return error_end(
                    "identity_unsupported",
                    "the newest page carries no message identity",
                );
            };
            TranscriptStep::Reply {
                reply: reply_with(Some(&snapshot.resync_page())),
                end_after: false,
            }
        }
        Some(NewestPageFit::Trimmed(snapshot)) => TranscriptStep::Reply {
            reply: reply_with(Some(&snapshot.resync_page())),
            end_after: false,
        },
        // Even the newest message alone does not fit: the phone reads
        // session.transcript_page itself and subscribes again.
        Some(NewestPageFit::NewestTooLarge) => TranscriptStep::Reply {
            reply: reply_with(None),
            end_after: true,
        },
        None => error_end(
            "reply_too_large",
            "the transcript reply exceeds one notice and this session's page cannot be trimmed",
        ),
    }
}

async fn evaluate_transcript<H: TranscriptWatchHost>(
    host: &H,
    session: &str,
    claim: &Map<String, Value>,
) -> TranscriptStep<H::Refusal> {
    let mut params = match host.readmit() {
        Ok(params) => params,
        Err(refusal) => return TranscriptStep::End(TranscriptWatchEnd::Refused(refusal)),
    };
    if params.get("session").is_none() {
        params["session"] = json!(session);
    }
    let accepts_outside_window = match crate::claim::accepts_outside_window(&params) {
        Ok(accepts) => accepts,
        Err(error) => return error_end(error.code, error.message),
    };
    params["claim"] = Value::Object(claim.clone());
    let mut reply = match transcript_outcome(transcript_read(host, params.clone()).await) {
        Ok(reply) => reply,
        Err(step) => return step,
    };
    reply["answers"] = Value::Object(claim.clone());
    let outside_window = reply["claim"] == "outside_window";
    // Ordinary notices and oversize_transcript resyncs use cursor_expired unless outside_window was accepted.
    if outside_window && !accepts_outside_window {
        reply["claim"] = json!("cursor_expired");
    }
    if encode(&reply).len() > NOTICE_BOUND {
        return oversize_transcript(host, session, &params, claim, outside_window).await;
    }
    TranscriptStep::Reply {
        reply,
        end_after: false,
    }
}

/// Advance the watch's claim only after successfully sending a notice to the
/// phone. Suppressed replies leave both claims unchanged. heldVersion is always
/// the reply envelope's version.
pub fn advance_claim(claim: &mut Map<String, Value>, reply: &Value) {
    if let Some(version) = reply.get("version").and_then(Value::as_str) {
        claim.insert("heldVersion".into(), json!(version));
    }
    match reply.get("claim").and_then(Value::as_str) {
        Some("matched") => {
            if let Some(last) = reply
                .get("newTurns")
                .and_then(Value::as_array)
                .and_then(|turns| turns.last())
                .and_then(|turn| turn.get("messageId"))
                .and_then(Value::as_str)
            {
                claim.insert("sinceMessageId".into(), json!(last));
            }
        }
        Some("cursor_expired") | Some("outside_window") => {
            // The delivered page is the watch's baseline. The phone can independently
            // stitch older held turns to an outside-window page by ordinal.
            if let Some(page) = reply.get("resync") {
                let ids = message_ids(page);
                if let (Some(oldest), Some(newest)) = (ids.first(), ids.last()) {
                    claim.insert("heldFloorMessageId".into(), json!(oldest));
                    claim.insert("sinceMessageId".into(), json!(newest));
                }
            }
        }
        Some("diverged") => {
            if let Some(vouched) = reply.get("vouchedFromMessageId").and_then(Value::as_str) {
                claim.insert("heldFloorMessageId".into(), json!(vouched));
            }
            if let Some(newest) = reply
                .get("resync")
                .map(message_ids)
                .and_then(|ids| ids.last().cloned())
            {
                claim.insert("sinceMessageId".into(), json!(newest));
            }
        }
        _ => {}
    }
}

/// Which emitted notices the suppression rules compare against.
#[derive(Default)]
pub struct TranscriptEmitted {
    any: bool,
    /// The page of the last emitted diverged, cursor_expired or outside_window notice.
    page: Option<TranscriptPageSnapshot>,
    /// A matched notice with newTurns has been emitted since that page.
    turns_since_page: bool,
    activity: Option<Value>,
    history_version: Option<Value>,
    last_run: Option<Value>,
}

impl TranscriptEmitted {
    /// The first reply is always sent. After that a matched reply is sent only
    /// with new turns or changed run/activity facts; a diverged, cursor_expired or outside_window
    /// reply is held back only when its page equals the last page sent, no
    /// turns were sent since, and `activity` is unchanged.
    pub fn should_emit(&self, reply: &Value) -> bool {
        if !self.any {
            return true;
        }
        let activity = reply.get("activity").cloned();
        // An unchanged page under a new history counter still invalidates the
        // client's old claim; content-only suppression must not hide that reset.
        if reply.get("historyVersion") != self.history_version.as_ref() {
            return true;
        }
        // A failed run can leave both the transcript and idle activity unchanged.
        if reply.get("lastRun") != self.last_run.as_ref() {
            return true;
        }
        match reply.get("claim").and_then(Value::as_str) {
            Some("matched") => {
                reply
                    .get("newTurns")
                    .and_then(Value::as_array)
                    .is_some_and(|turns| !turns.is_empty())
                    || activity != self.activity
            }
            Some("diverged") | Some("cursor_expired") | Some("outside_window") => {
                let page = reply.get("resync").and_then(page_material);
                let suppressed = same_material(self.page.as_ref(), page.as_ref())
                    && !self.turns_since_page
                    && activity == self.activity
                    && reply.get("resync").is_some();
                !suppressed
            }
            _ => true,
        }
    }

    /// Note a reply that was sent.
    pub fn record(&mut self, reply: &Value) {
        self.any = true;
        self.activity = reply.get("activity").cloned();
        self.history_version = reply.get("historyVersion").cloned();
        self.last_run = reply.get("lastRun").cloned();
        match reply.get("claim").and_then(Value::as_str) {
            Some("matched") => {
                if reply
                    .get("newTurns")
                    .and_then(Value::as_array)
                    .is_some_and(|turns| !turns.is_empty())
                {
                    self.turns_since_page = true;
                }
            }
            Some("diverged") | Some("cursor_expired") | Some("outside_window") => {
                self.page = reply.get("resync").and_then(page_material);
                self.turns_since_page = false;
            }
            _ => {}
        }
    }
}

/// Sleep for `duration` unless the watch stops first; false when it stopped.
async fn sleep_unless_stopped<H: TranscriptWatchHost>(host: &H, duration: Duration) -> bool {
    tokio::select! {
        _ = host.stopped() => false,
        _ = tokio::time::sleep(duration) => true,
    }
}

/// Serve one transcript watch until it ends. `params` are the admitted
/// `session.transcript_page` params, holding `session` and `claim`.
pub async fn run<H: TranscriptWatchHost>(
    host: &H,
    params: &Value,
) -> TranscriptWatchEnd<H::Refusal> {
    let Some(session) = params
        .get("session")
        .or_else(|| params.get("sessionID"))
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return TranscriptWatchEnd::Error {
            code: "invalid_request".into(),
            message: "session.transcript_watch requires 'session'".into(),
        };
    };
    let Some(mut claim) = params.get("claim").and_then(Value::as_object).cloned() else {
        return TranscriptWatchEnd::Error {
            code: "invalid_claim".into(),
            message: "session.transcript_watch requires a claim object (heldFloorMessageId, sinceMessageId, heldVersion; or heldVersion alone for an empty page)".into(),
        };
    };
    let mut emitted = TranscriptEmitted::default();
    let mut last_evaluation: Option<Instant> = None;
    let mut retry_at: Option<Instant> = None;
    // Subscribed before the first read, so an event landing during it is not
    // lost: the receiver stays marked changed until the next wait takes it.
    let mut events = host.subscribe_changes(&session);
    events.borrow_and_update();
    loop {
        if let Some(last) = last_evaluation {
            // Wait for an event for the session, a retry after a failure, or
            // the slow sweep, whichever comes first.
            let sweep = Instant::now() + TRANSCRIPT_SWEEP;
            let wake = retry_at.map_or(sweep, |retry| retry.min(sweep));
            if !events.has_changed().unwrap_or(false) {
                tokio::select! {
                    biased;
                    _ = host.stopped() => return TranscriptWatchEnd::Streamed,
                    _ = events.changed() => {}
                    _ = tokio::time::sleep_until(wake) => {}
                }
            }
            // At most one evaluation per interval; events arriving meanwhile
            // fold into the one evaluation that follows.
            let not_before = last + TRANSCRIPT_MIN_INTERVAL;
            let wait = not_before.saturating_duration_since(Instant::now());
            if !wait.is_zero() && !sleep_unless_stopped(host, wait).await {
                return TranscriptWatchEnd::Streamed;
            }
            events.borrow_and_update();
        }
        if host.is_stopped() {
            return TranscriptWatchEnd::Streamed;
        }
        last_evaluation = Some(Instant::now());
        retry_at = None;
        match evaluate_transcript(host, &session, &claim).await {
            TranscriptStep::Retry => {
                retry_at = Some(Instant::now() + RETRY_AFTER);
            }
            TranscriptStep::End(end) => return end,
            // A claim holding no messages is answered `unchanged` while the page is
            // still empty at its version; the watch says nothing until that changes.
            TranscriptStep::Reply { reply, .. } if reply["unchanged"] == true => {}
            TranscriptStep::Reply { reply, end_after } => {
                if emitted.should_emit(&reply) || end_after {
                    emitted.record(&reply);
                    if host.emit(encode(&reply)).await.is_err() {
                        return TranscriptWatchEnd::Streamed;
                    }
                    advance_claim(&mut claim, &reply);
                }
                if end_after {
                    return TranscriptWatchEnd::Streamed;
                }
            }
        }
    }
}
