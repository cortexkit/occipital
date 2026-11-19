//! One transcript page body over session-bound, in-process sources.
//!
//! Adapters supply canonical display rows and durable cursors, not transport or
//! authorization. The caller resolves the session and admits the request first.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use cortexkit_role_llm_runner::read::{LastRunError, LastRunState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::activity::ActivityTokens;
use crate::claim::{
    self, NewestPageFit, OlderPageCursor, TranscriptClaimState, TranscriptPageSnapshot,
};
use crate::page::{self, PageRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadError {
    pub code: &'static str,
    pub message: String,
    pub detail: Option<Value>,
}

impl ReadError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }
}

impl From<claim::ClaimRefusal> for ReadError {
    fn from(value: claim::ClaimRefusal) -> Self {
        Self::new(value.code, value.message)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Attachments {
    Yes,
    No,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub struct InputCapability {
    pub attachments: Attachments,
    pub mime_types: Option<Vec<String>>,
    pub modalities: Option<Vec<String>>,
    /// The largest image file the model takes, in bytes as uploaded, and the
    /// longest side of one image in a request, in pixels, when the runner
    /// reports them (runner contract 0.8.4).
    pub max_image_bytes: Option<u64>,
    pub max_image_dimension: Option<u64>,
}

impl InputCapability {
    /// Whether a send can carry a file of type `mime`: `Some(false)` when that is
    /// known not to be so, `Some(true)` when it is known to be, `None` when nothing
    /// says. The runner's exact types decide when it names them; otherwise the
    /// model's modalities do, by the type's kind (`image/png` is an `image`,
    /// `application/pdf` a `pdf`). An unknown fact is never a known refusal, as the
    /// runner contract says.
    pub fn accepts(&self, mime: &str) -> Option<bool> {
        let mime = mime.trim().to_ascii_lowercase();
        if self.attachments == Attachments::No {
            return Some(false);
        }
        if let Some(types) = &self.mime_types {
            return Some(types.iter().any(|known| known.eq_ignore_ascii_case(&mime)));
        }
        let kind = match mime.split_once('/') {
            Some(("application", "pdf")) => "pdf",
            Some((kind @ ("image" | "audio" | "video" | "text"), _)) => kind,
            _ => return None,
        };
        self.modalities
            .as_ref()
            .map(|modalities| modalities.iter().any(|known| known == kind))
    }
}

/// The reply member that says the runner's silence about a sender grant can't be
/// read as "no grant". Every reply names it through this constant, so a change to
/// the wire name the apps agree on is made here alone.
pub const SENDER_UNKNOWN: &str = "senderUnknown";

/// Whether the app may send to this session, as replies tell the phone. A reply
/// carries exactly one of three forms, and never both members: `sender` (an
/// object, never null), `senderUnknown: true`, or neither.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum SendGrant {
    /// The runner reports this grant (`{modes, interrupt, owner?}`) for the module
    /// the app's messages go through, so the app may send. Holding a JSON object
    /// rather than any value makes a `null` sender unrepresentable.
    Granted(serde_json::Map<String, Value>),
    /// The app may not send: the runner declares that it reports grants and
    /// reports none for that module, or the module itself won't deliver the app's
    /// messages to this session. It's the default, so a source with no send path
    /// never claims one.
    #[default]
    MayNotSend,
    /// Whether the app may send is unknown: for example, the head has no `sender`
    /// and the runner doesn't declare `head_sender` (an older runner leaves the
    /// member out because it doesn't know it), or its `role.describe` couldn't be read.
    Unknown,
}

impl SendGrant {
    fn write(&self, reply: &mut Value) {
        match self {
            Self::Granted(sender) => reply["sender"] = Value::Object(sender.clone()),
            Self::MayNotSend => {}
            Self::Unknown => reply[SENDER_UNKNOWN] = json!(true),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Head {
    /// The oldest readable ordinal, if numbered history exists.
    pub first_ordinal: Option<u64>,
    /// The newest assigned ordinal, for bounded source reads; not a phone field.
    pub last_ordinal: Option<u64>,
    /// Rises when already-served history is rewritten (an edit, a revert or a
    /// renumbering), never on an append, and never at a compaction cut, which
    /// leaves every stored message in place.
    pub history_version: Option<u64>,
    /// An opaque identity of the session's log: unchanged across cuts, reverts,
    /// restarts, restores and moves, and new only when the log is created again.
    /// Claims and cursors bind it with `history_version`: the counter alone can't
    /// tell a rewrite from a recreated log that restarted at the same count.
    pub incarnation: Option<String>,
    /// A watermark derived only from durable source facts. Unlike a live-feed
    /// watermark, it can be revalidated by a head read after a source restart.
    pub activity_token: Option<String>,
    pub input: InputCapability,
    /// What a send naming no model would use next, `{provider, model, variant?}`.
    /// `None` means unknown.
    pub model: Option<Value>,
    /// The service tier a send naming none would use next, such as `standard` or
    /// `fast`, as the runner's head states it. `None` means the head named none:
    /// standard on a runner that selects heads, unknown on any other.
    pub service_tier: Option<String>,
    /// The runner is still taking in history the host already held, as when a
    /// head has just moved onto it (`session.head.history_intake`). It takes the
    /// history in oldest first, so until it's done the newest page is the newest
    /// message taken in so far, not the chat's latest. Any value the head gives
    /// counts, since the contract reads an unknown one as still running.
    pub history_intake: bool,
    /// The state of the session's host program when the runner can't show it is
    /// alive (`session.head.host`), as the runner spells it: `unknown` while it
    /// can't tell after a restart. The run stays active, but nothing more arrives
    /// until the host is found, and Stop ends the run at once. The contract's
    /// later spellings mean the same: never that the host is alive. `None` when
    /// the head names none.
    pub host: Option<String>,
    /// The session's durable spend in the runner's own shape. `None` means unknown.
    pub usage: Option<Value>,
    pub sender: SendGrant,
    pub last_run: Option<LastRunState>,
    /// Absent means the runner did not report an error classification.
    pub last_run_error: Option<LastRunError>,
}

/// Only runner codes reach the phone, never provider messages or diagnostic bodies.
pub fn last_run_value(run: &LastRunState) -> Value {
    last_run_with_error_value(run, None)
}

fn last_run_with_error_value(run: &LastRunState, error: Option<&LastRunError>) -> Value {
    let mut value = json!({"runId": run.run_id, "state": run.state});
    if let Some(reason) = &run.reason {
        value["reason"] = json!(reason);
    }
    // While a run is paused because its account is at its limit
    // (`reason: "account_limit"`), the earliest time the runner resumes it
    // itself, so the app can say how long it waits.
    if let Some(at) = run.resume_not_before_ms {
        value["resumeNotBeforeMs"] = json!(at);
    }
    if let Some(wait) = &run.awaits {
        value["awaits"] = awaits_value(wait);
    }
    if let Some(error) = error {
        let mut error_value = json!({"class": error.class});
        if let Some(retry) = error.retry_after_secs {
            error_value["retryAfterSecs"] = json!(retry);
        }
        if let Some(resets) = error.resets_at_ms {
            error_value["resetsAtMs"] = json!(resets);
        }
        // The runner contract's code for the run's error, such as
        // `account_limit_exhausted`, which an app reads as an account wait.
        // Only codes the contract names pass: a provider may put its own text
        // there, and that never reaches the apps.
        if let Some(code) = error
            .provider_code
            .as_deref()
            .filter(|code| cortexkit_role_llm_runner::errors::provider_codes::CODES.contains(code))
        {
            error_value["providerCode"] = json!(code);
        }
        value["error"] = error_value;
    }
    value
}

/// What the run waits on, in the apps' spelling: who ends the wait (`party`),
/// why, since when, when it moves by itself or gives up, which attempt, and
/// what the person must do. A `party` or `person.action` this crate doesn't
/// know passes as written, for the app to word from the other members.
///
/// `message` is quoted words about the wait, labelled by whose words they are
/// (`from`). The apps never draw it as the wait's own line; they open it on
/// tap, quoted under that label. So it passes for every source the contract
/// names, the provider's and the host's words included, and is
/// left out when `from` is one this crate doesn't know, since the apps would
/// have no true label for it.
fn awaits_value(wait: &cortexkit_role_llm_runner::awaits::RunAwaits) -> Value {
    use cortexkit_role_llm_runner::awaits::AwaitMessageFrom;
    let mut value = json!({"party": String::from(wait.party.clone()), "sinceMs": wait.since_ms});
    if let Some(reason) = &wait.reason {
        value["reason"] = json!(reason);
    }
    if let Some(next) = wait.next_ms {
        value["nextMs"] = json!(next);
    }
    if let Some(deadline) = wait.deadline_ms {
        value["deadlineMs"] = json!(deadline);
    }
    if let Some(attempt) = wait.attempt {
        value["attempt"] = json!(attempt);
    }
    if let Some(person) = &wait.person {
        value["person"] = json!({"action": String::from(person.action.clone())});
        if let Some(on) = &person.on {
            value["person"]["on"] = json!(on);
        }
    }
    if let Some(message) = wait
        .message
        .as_ref()
        .filter(|message| !matches!(message.from, AwaitMessageFrom::Unknown(_)))
    {
        value["message"] =
            json!({"text": message.text, "from": String::from(message.from.clone())});
    }
    value
}

/// The members [`Head::describe_session`] writes. A claim reply carries them at
/// its top level and never inside its `resync` page, so anything that rebuilds a
/// reply around a page (a watch trimming an oversized one) moves all of them.
pub const SESSION_FACTS: &[&str] = &[
    "historyIntake",
    "host",
    "model",
    "serviceTier",
    "sender",
    SENDER_UNKNOWN,
    "lastRun",
];

impl Head {
    fn claim_version(&self) -> Option<String> {
        self.history_version.map(|version| match &self.incarnation {
            Some(incarnation) => format!("history:{version}:incarnation:{incarnation}"),
            None => format!("history:{version}"),
        })
    }

    /// The session facts every newest page and every turn-carrying claim reply
    /// repeats from the head read in the same request: the next send's model and
    /// service tier, whether the app may send, the most recent run's outcome,
    /// `historyIntake: "running"` while the runner is still taking in history, and
    /// `host` while the runner can't show the session's host program is alive.
    pub(crate) fn describe_session(&self, reply: &mut Value) {
        if self.history_intake {
            reply["historyIntake"] = json!("running");
        }
        if let Some(host) = &self.host {
            reply["host"] = json!(host);
        }
        if let Some(model) = &self.model {
            reply["model"] = model.clone();
        }
        if let Some(tier) = &self.service_tier {
            reply["serviceTier"] = json!(tier);
        }
        self.sender.write(reply);
        if let Some(run) = &self.last_run {
            reply["lastRun"] = last_run_with_error_value(run, self.last_run_error.as_ref());
        }
    }
}

/// Cursors are opaque to the reader; adapters must preserve their message anchor
/// across restarts and refuse deleted anchors with `cursor_expired`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Anchor {
    Cursor(String),
    Mid(String),
    Ordinal(u64),
    /// Re-read the newest history window when there are no numbered rows yet.
    Newest,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub mid: String,
    pub ordinal: Option<u64>,
    pub written_at: Option<i64>,
    /// The app's display form, including optional clientMessageId and file references.
    pub display: Value,
}

impl Row {
    fn value(&self) -> Result<Value, ReadError> {
        if self.mid.is_empty()
            || self
                .ordinal
                .is_some_and(|ordinal| ordinal > i64::MAX as u64)
        {
            return Err(ReadError::new(
                "identity_unsupported",
                "invalid transcript row identity",
            ));
        }
        let mut value = self.display.clone();
        let Some(object) = value.as_object_mut() else {
            return Err(ReadError::new(
                "identity_unsupported",
                "transcript row is not an object",
            ));
        };
        object.insert("messageId".into(), json!(self.mid));
        object.remove("ordinal");
        if let Some(ordinal) = self.ordinal {
            object.insert("ordinal".into(), json!(ordinal));
        }
        if let Some(written_at) = self.written_at {
            object.insert("createdAtMs".into(), json!(written_at));
        }
        Ok(value)
    }
}

/// Rows are oldest first, with unnumbered history before numbered history. The
/// cursors address the next older/newer window exclusively, without overlap.
#[derive(Debug, Clone, Default)]
pub struct HistoryPage {
    pub rows: Vec<Row>,
    pub older: Option<String>,
    pub newer: Option<String>,
    pub history_version: Option<u64>,
    pub incarnation: Option<String>,
}

impl HistoryPage {
    fn agrees_with(&self, head: &Head) -> bool {
        self.history_version == head.history_version && self.incarnation == head.incarnation
    }
}

#[async_trait]
pub trait TranscriptSource: Send + Sync {
    fn harness(&self) -> &'static str;
    async fn head(&self) -> Result<Head, ReadError>;
    async fn newest(&self, limit: u32) -> Result<HistoryPage, ReadError>;
    /// Mid and ordinal anchors are exclusive upper bounds. Newest includes
    /// unnumbered history; ordinary newest app pages filter it out themselves.
    async fn older(&self, anchor: &Anchor, limit: u32) -> Result<HistoryPage, ReadError>;
    async fn newer(&self, from_mid: &str, limit: u32) -> Result<HistoryPage, ReadError>;
    /// The numbered message `mid` with up to `limit` numbered messages before and after
    /// it. `None` when the current history line holds no numbered message `mid`. The work
    /// is bounded by that window: the message is looked up once, then only its range is
    /// read.
    async fn around(&self, _mid: &str, _limit: u32) -> Result<Option<Positioned>, ReadError> {
        Err(unsupported("aroundMessageId"))
    }
    /// Up to `limit` numbered messages after `ordinal`, oldest first.
    async fn after(&self, _ordinal: u64, _limit: u32) -> Result<Positioned, ReadError> {
        Err(unsupported("afterOrdinal"))
    }
    /// Exact reads use full content, with none of the display summary budgets.
    async fn exact(&self, anchor: &Anchor) -> Result<Option<Row>, ReadError>;
    /// Prove that the held message ID still names a row with an ordinal.
    /// Missing rows and read errors keep the caller on the cursor-expired path.
    async fn numbered_claim_anchor(&self, mid: &str) -> bool {
        self.exact(&Anchor::Mid(mid.into()))
            .await
            .ok()
            .flatten()
            .is_some_and(|row| row.mid == mid && row.ordinal.is_some())
    }
    /// Of `rows`, the ids of the wake rows whose record holds the person's
    /// answer to an ask, which the Simple view shows. Only the module that
    /// keeps wake records knows; a source without them answers none.
    async fn answered_asks(&self, _rows: &[Row]) -> HashSet<String> {
        HashSet::new()
    }
    fn changes(&self) -> watch::Receiver<u64>;
    fn activity(&self) -> Option<(&'static str, i64)>;
    /// A disconnected feed cannot authorize an activity-token cache hit.
    fn activity_feed_live(&self) -> bool {
        true
    }
    /// Whether the fleet's session directory records this session. One read
    /// without a record serves reads only, so it is named in no command
    /// subject.
    fn recorded(&self) -> bool {
        true
    }
}

/// A page read from a position: around a message, or after an ordinal.
#[derive(Debug, Clone)]
pub struct Positioned {
    pub page: HistoryPage,
    /// The ordinal of the message a page around a message was read around.
    pub anchor: Option<u64>,
    /// The ordinal of the history line's newest numbered message when the page was read;
    /// `None` when it has none.
    pub newest: Option<u64>,
}

/// The refusal for a page form this session's source can't serve, naming the field.
pub fn unsupported(field: &str) -> ReadError {
    ReadError::new(
        "unsupported",
        format!("this session's runner can't serve {field} pages"),
    )
    .with_detail(json!({"field": field}))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryCursor {
    anchor: Anchor,
    version: Option<String>,
}

/// Shared by page reads, claim evaluation and the watch's oversize fallback.
#[derive(Debug, Default)]
pub struct TranscriptReader {
    pub activity: ActivityTokens,
    claims: Mutex<TranscriptClaimState>,
    revisions: Mutex<HashMap<String, u64>>,
    /// Each chat's Simple rows, kept between reads.
    simple: std::sync::Arc<crate::simple::tails::Tails>,
}

impl TranscriptReader {
    fn sync_revision(&self, session: &str, revision: u64) {
        let previous = self
            .revisions
            .lock()
            .expect("transcript revisions poisoned")
            .insert(session.to_string(), revision);
        if previous != Some(revision) {
            self.activity.note_transcript_activity(session);
        }
    }

    /// The page handler's bounded reply. `None` is a session the resolver does
    /// not know, not an offline provider.
    pub async fn read(
        &self,
        source: Option<&dyn TranscriptSource>,
        params: Value,
    ) -> Result<Value, ReadError> {
        bounded(self.read_for_watch(source, params).await?)
    }

    /// The kept Simple rows, for the module that learns when a `say` record
    /// one of them draws is written ([`crate::simple::tails::Tails::forget_call`]).
    pub fn simple_tails(&self) -> std::sync::Arc<crate::simple::tails::Tails> {
        self.simple.clone()
    }

    /// The same page body before the transport-size guard. A watch needs to
    /// inspect even very large deltas so it can replace them with a fitted
    /// resync; this result must never be sent directly as a transport reply.
    /// Every reply to a Simple read says so (`view: "simple"`), so an app
    /// that saves its copies can tell which one a reply belongs to.
    pub async fn read_for_watch(
        &self,
        source: Option<&dyn TranscriptSource>,
        params: Value,
    ) -> Result<Value, ReadError> {
        let simple = params.get("view").and_then(Value::as_str) == Some(crate::simple::VIEW);
        let mut reply = self.read_page(source, params).await?;
        if simple && reply.is_object() {
            reply["view"] = json!(crate::simple::VIEW);
        }
        Ok(reply)
    }

    async fn read_page(
        &self,
        source: Option<&dyn TranscriptSource>,
        params: Value,
    ) -> Result<Value, ReadError> {
        let request = match PageRequest::parse(&params) {
            Ok(request) => request,
            Err(error) if error.code != "invalid_request" => return Ok(error.reply()),
            Err(error) => return Err(error.into()),
        };
        let Some(source) = source else {
            return Ok(
                if request.claim.is_some() || request.empty_claim.is_some() {
                    claim::error_reply(
                        "session_unknown",
                        &format!(
                            "no session '{}' is known to this daemon",
                            request.session_id
                        ),
                        None,
                    )
                } else {
                    // No source can say whether older messages exist, so the page
                    // never claims the start.
                    let mut page = json!({"messages": [], "emptyReason": "provider_unavailable"});
                    if request.message_ordinal.is_some() {
                        page["fullContentUnavailable"] = json!("no available transcript provider");
                    }
                    page
                },
            );
        };
        if let Some(harness) = request.requested_harness.as_deref() {
            if harness != source.harness()
                && (request.claim.is_some() || request.empty_claim.is_some())
            {
                return Ok(claim::harness_mismatch_reply(
                    harness,
                    &request.session_id,
                    source.harness(),
                ));
            }
        }
        page::assert_transcript_harness(request.requested_harness.as_deref(), source.harness())?;
        let events = source.changes();
        self.sync_revision(&request.session_id, *events.borrow());
        if !source.activity_feed_live() {
            self.activity
                .invalidate_transcript_activity_token(&request.session_id);
        }
        let newest_request = request.before_ordinal.is_none() && request.before_cursor.is_none();
        if request.claim.is_none() && newest_request && request.activity_token.is_some() {
            let head = source.head().await?;
            if head.activity_token.is_some() && head.activity_token == request.activity_token {
                let mut reply = json!({"unchanged": true, "activityToken": head.activity_token});
                decorate(&mut reply, &head, source);
                return Ok(reply);
            }
        }
        if request.claim.is_none()
            && newest_request
            && source.activity_feed_live()
            && request.activity_token.as_deref().is_some_and(|token| {
                self.activity
                    .transcript_activity_token_matches(&request.session_id, token)
            })
        {
            let head = source.head().await?;
            // A head read may itself deliver a pending change notification.
            self.sync_revision(&request.session_id, *events.borrow());
            if request.activity_token.as_deref().is_some_and(|token| {
                self.activity
                    .transcript_activity_token_matches(&request.session_id, token)
            }) {
                let mut reply = json!({"unchanged": true, "activityToken": request.activity_token});
                decorate(&mut reply, &head, source);
                return Ok(reply);
            }
        }
        if request.around_message_id.is_some() || request.after_ordinal.is_some() {
            return self.read_positioned(source, &request).await;
        }
        if let Some(ordinal) = request.message_ordinal {
            let head = source.head().await.unwrap_or_default();
            let full = source.exact(&Anchor::Ordinal(ordinal as u64)).await;
            let mut reply = match full {
                Ok(Some(row)) if row.ordinal == Some(ordinal as u64) => {
                    json!({"messages": [row.value()?], "reachedStart": true, "fullContent": true})
                }
                unavailable => {
                    let reason = match unavailable {
                        Err(error) => error.message,
                        _ => format!("source no longer contains message [{ordinal}]"),
                    };
                    // A retained summary is useful during an outage, but must
                    // never masquerade as the full arguments or tool output.
                    // The summary is read as the one row after `ordinal - 1`.
                    // Reading the one row older than `ordinal + 1` instead finds
                    // nothing when `ordinal` is the newest message, which is where
                    // a call still waiting for its result usually is; it is kept
                    // only for a source that can't read after an ordinal.
                    let summary = match source.after((ordinal as u64).saturating_sub(1), 1).await {
                        Ok(positioned) => positioned.page.rows,
                        Err(_) => source
                            .older(&Anchor::Ordinal(ordinal as u64 + 1), 1)
                            .await
                            .map(|page| page.rows)
                            .unwrap_or_default(),
                    };
                    let rows = summary
                        .into_iter()
                        .filter(|row| row.ordinal == Some(ordinal as u64))
                        .map(|row| row.value())
                        .collect::<Result<Vec<_>, _>>()?;
                    let mut reply = json!({"messages": rows, "reachedStart": true});
                    reply["fullContentUnavailable"] = json!(reason);
                    reply
                }
            };
            decorate(&mut reply, &head, source);
            return Ok(reply);
        }
        let stable = self
            .activity
            .read_newest_transcript_page_stably(&request.session_id, || async {
                let revision = *events.borrow();
                let head = source.head().await?;
                // A Simple page is found by reading the full history back and
                // keeping what the view shows; where that read stopped is the
                // page's own cursor, apart from its rows.
                let mut simple_next = None;
                let page = if request.simple {
                    let found = self.simple_page(source, &request, &head).await?;
                    simple_next = Some(found.next_before);
                    HistoryPage {
                        rows: found.rows,
                        older: None,
                        newer: None,
                        history_version: head.history_version,
                        incarnation: head.incarnation.clone(),
                    }
                } else if let Some(cursor) = request.before_cursor.as_deref() {
                    let cursor: HistoryCursor = serde_json::from_str(cursor).map_err(|_| {
                        ReadError::new("cursor_expired", "the history cursor is not recognized")
                    })?;
                    if cursor.version != head.claim_version() {
                        return Err(ReadError::new(
                            "cursor_expired",
                            "the history cursor names rewritten history",
                        ));
                    }
                    source.older(&cursor.anchor, request.limit as u32).await?
                } else if let Some(ordinal) = request.before_ordinal {
                    source
                        .older(&Anchor::Ordinal(ordinal as u64), request.limit as u32)
                        .await?
                } else {
                    source.newest(request.limit as u32).await?
                };
                let boundary = if request.before_cursor.is_none()
                    && page.rows.iter().any(|row| row.ordinal.is_none())
                {
                    if let Some(ordinal) = request.before_ordinal {
                        let row = source
                            .exact(&Anchor::Ordinal(ordinal as u64))
                            .await?
                            .ok_or_else(|| {
                                ReadError::new(
                                    "unknown_ordinal",
                                    "the history boundary ordinal is not assigned",
                                )
                            })?;
                        Some(Anchor::Mid(row.mid))
                    } else {
                        None
                    }
                } else {
                    None
                };
                // A Simple claim holds a Simple version.
                let page_version = head.claim_version().map(|version| {
                    if request.simple {
                        crate::simple::version(&version)
                    } else {
                        version
                    }
                });
                let numbered_anchor = if let Some(held) = request.claim.as_ref().filter(|held| {
                    page_version.as_deref() == Some(held.held_version.as_str())
                        && !page
                            .rows
                            .iter()
                            .any(|row| row.mid == held.since_message_id && row.ordinal.is_some())
                }) {
                    source.numbered_claim_anchor(&held.since_message_id).await
                } else {
                    false
                };
                let after = source.head().await?;
                self.sync_revision(&request.session_id, *events.borrow());
                if revision != *events.borrow()
                    || !page.agrees_with(&head)
                    || head.claim_version() != after.claim_version()
                    || head.activity_token != after.activity_token
                {
                    self.activity.note_transcript_activity(&request.session_id);
                }
                Ok::<_, ReadError>((head, page, boundary, numbered_anchor, simple_next))
            })
            .await?;
        let (head, history, boundary, numbered_anchor, simple_next) = stable.page;
        let mut mapped = match simple_next {
            Some(next_before) => map_simple_page(history, next_before)?,
            None => map_page(history, &head, request.before_cursor.is_some(), boundary)?,
        };
        // A newest page without rows that reached the start is a session with no
        // messages. Apps read a page without rows by its `emptyReason`, so it
        // names that; a resync built from this page carries it too. A Simple
        // page can also reach the start with no rows in a chat that has
        // messages, none of which the view shows.
        if newest_request
            && mapped["messages"].as_array().is_some_and(Vec::is_empty)
            && mapped["reachedStart"] == true
        {
            mapped["emptyReason"] = json!(if request.simple && head.last_ordinal.is_some() {
                "nothing_visible"
            } else {
                "empty"
            });
        }
        let version = head
            .claim_version()
            .unwrap_or_else(|| self.activity.ensure_transcript_version(&request.session_id));
        let version = if request.simple {
            crate::simple::version(&version)
        } else {
            version
        };
        // The Simple copies served are kept apart from the full ones: a claim
        // is checked against the copy its claimant was served.
        let claims_key = if request.simple {
            crate::simple::claims_key(&request.session_id)
        } else {
            request.session_id.clone()
        };
        let native = head.claim_version().is_some();
        let mut reply = if let Some(held_version) = request.empty_claim.as_deref() {
            if stable.activity_token.is_none() {
                return Ok(claim::observation_unstable_reply(if native {
                    "native"
                } else {
                    "scan"
                }));
            }
            let still_empty = mapped["messages"].as_array().is_some_and(Vec::is_empty);
            let mut reply = if still_empty && held_version == version {
                json!({"unchanged": true, "version": version})
            } else {
                // A first message or a new version replaces the empty page the app
                // holds, as a replaced held page is answered.
                let current = match TranscriptPageSnapshot::from_page(
                    &version,
                    head.claim_version(),
                    mapped.clone(),
                ) {
                    Ok(snapshot) => snapshot,
                    Err(_) => return Ok(claim::identity_unsupported_reply()),
                };
                let resync = current.resync_page();
                self.record(&claims_key, current);
                if native {
                    claim::runner_claim_reply(
                        &version,
                        true,
                        claim::RunnerClaimEvaluation::CursorExpired { resync },
                    )
                } else {
                    claim::claim_reply(&version, claim::ClaimEvaluation::CursorExpired { resync })
                }
            };
            head.describe_session(&mut reply);
            reply
        } else if let Some(held) = request.claim.as_ref() {
            if stable.activity_token.is_none() {
                return Ok(claim::observation_unstable_reply(if native {
                    "native"
                } else {
                    "scan"
                }));
            }
            let mut current = match TranscriptPageSnapshot::from_page(
                &version,
                head.claim_version(),
                mapped.clone(),
            ) {
                Ok(snapshot) => snapshot,
                Err(_) => return Ok(claim::identity_unsupported_reply()),
            };
            // An equal version proves the runner's entries unchanged, but a row can still
            // change in place: a runner that stores a step before its calls run sends each
            // result as a later entry, which gives the call's row its finish. So a native
            // claim is also checked against the copy its claimant was served, when one is
            // kept, and a changed held row answers `diverged` with a resync from it.
            let changed = (native && held.held_version == version)
                .then(|| {
                    let claims = self.claims.lock().expect("transcript claims poisoned");
                    claims.share_unchanged(&claims_key, &mut current);
                    claims.held_snapshot(&claims_key, held)
                })
                .flatten()
                .map(|previous| claim::evaluate_claim(held, Some(&previous), &current))
                .filter(|evaluation| matches!(evaluation, claim::ClaimEvaluation::Diverged { .. }));
            let mut reply = if let Some(diverged) = changed {
                let mut reply = claim::claim_reply(&version, diverged);
                reply["diagnostics"] = claim::diagnostics("durable", "native");
                reply
            } else if native {
                let evaluation = if held.held_version == version {
                    let messages = mapped["messages"].as_array().expect("mapped messages");
                    if let Some(index) = messages
                        .iter()
                        .position(|row| row["messageId"] == held.since_message_id)
                    {
                        claim::RunnerClaimEvaluation::Matched {
                            new_turns: messages[index + 1..].to_vec(),
                            vouched_from_message_id: held.held_floor_message_id.clone(),
                        }
                    } else if numbered_anchor && request.accepts_outside_window {
                        // Equal versions preserve held ordinals; only the delivery window was missed.
                        claim::RunnerClaimEvaluation::OutsideWindow {
                            resync: current.resync_page(),
                        }
                    } else {
                        claim::RunnerClaimEvaluation::CursorExpired {
                            resync: current.resync_page(),
                        }
                    }
                } else {
                    claim::RunnerClaimEvaluation::CursorExpired {
                        resync: current.resync_page(),
                    }
                };
                claim::runner_claim_reply(&version, true, evaluation)
            } else {
                let previous = {
                    let claims = self.claims.lock().expect("transcript claims poisoned");
                    // Rows unchanged since the last page keep their fingerprints, so the
                    // comparison below fingerprints only the new and changed rows.
                    claims.share_unchanged(&claims_key, &mut current);
                    claims.held_snapshot(&claims_key, held)
                };
                claim::claim_reply(
                    &version,
                    claim::evaluate_claim(held, previous.as_ref(), &current),
                )
            };
            self.record(&claims_key, current);
            // Matched replies carry newTurns; diverged, cursor_expired and outside_window
            // carry resync messages. All repeat the session facts from the same head.
            head.describe_session(&mut reply);
            if native && reply["claim"] == "matched" {
                // Native matches have never carried activityToken; preserve that wire shape.
                decorate(&mut reply, &head, source);
                return Ok(reply);
            }
            reply
        } else {
            mapped["version"] = json!(version);
            mapped["diagnostics"] =
                claim::diagnostics("durable", if native { "native" } else { "scan" });
            if newest_request && stable.activity_token.is_some() {
                if let Ok(snapshot) = TranscriptPageSnapshot::from_page(
                    &version,
                    head.claim_version(),
                    mapped.clone(),
                ) {
                    self.record(&claims_key, snapshot);
                }
            }
            // Only the newest page describes the session: an older history page
            // is a step back through the transcript, not a fresh look at it.
            if newest_request {
                head.describe_session(&mut mapped);
            }
            mapped
        };
        if stable.activity_token.is_some() {
            if let Some(token) = head.activity_token.as_deref() {
                reply["activityToken"] = json!(token);
            } else {
                self.activity.attach_captured_transcript_activity_token(
                    &request.session_id,
                    stable.activity_token.as_deref(),
                    &mut reply,
                );
            }
        } else {
            self.activity
                .attach_unstable_transcript_activity_discard(&request.session_id, &mut reply);
        }
        decorate(&mut reply, &head, source);
        Ok(reply)
    }

    /// A page around a message (`aroundMessageId`) or after an ordinal (`afterOrdinal`).
    /// Older pages continue as on any page: `nextBeforeOrdinal`, or `nextBeforeCursor` at
    /// the first numbered message when older unnumbered history remains. Newer pages
    /// continue with `nextAfterOrdinal` until a page ends at the newest message, which
    /// says `reachedNewest`.
    async fn read_positioned(
        &self,
        source: &dyn TranscriptSource,
        request: &PageRequest,
    ) -> Result<Value, ReadError> {
        let head = source.head().await?;
        let limit = request.limit as u32;
        let version = head
            .claim_version()
            .unwrap_or_else(|| self.activity.ensure_transcript_version(&request.session_id));
        let positioned = if let Some(mid) = request.around_message_id.as_deref() {
            match source.around(mid, limit).await? {
                Some(positioned) => positioned,
                None => {
                    return Err(ReadError::new(
                        "message_not_found",
                        format!("message {mid} is not in this version of the transcript"),
                    )
                    .with_detail(json!({"version": version})))
                }
            }
        } else {
            let ordinal = request.after_ordinal.expect("checked by the caller") as u64;
            source.after(ordinal, limit).await?
        };
        // The page ends at its last message; an empty page (limit 0) ends where it began.
        let end = positioned
            .page
            .rows
            .last()
            .and_then(|row| row.ordinal)
            .or(positioned.anchor)
            .or(request.after_ordinal.map(|after| after as u64));
        let mut reply = map_page(positioned.page, &head, false, None)?;
        if let Some(ordinal) = positioned.anchor {
            reply["anchorOrdinal"] = json!(ordinal);
        }
        match (end, positioned.newest) {
            (Some(end), Some(newest)) if end < newest => reply["nextAfterOrdinal"] = json!(end),
            _ => reply["reachedNewest"] = json!(true),
        }
        reply["version"] = json!(version);
        decorate(&mut reply, &head, source);
        Ok(reply)
    }

    /// The Simple rows a page of `request` serves: the newest, or those before
    /// its `beforeOrdinal`. Only a runner that versions its history serves the
    /// view, since its pages are told apart by that version. A full page's
    /// cursor never continues a Simple page, which goes on by ordinal.
    async fn simple_page(
        &self,
        source: &dyn TranscriptSource,
        request: &PageRequest,
        head: &Head,
    ) -> Result<crate::simple::tails::Found, ReadError> {
        let Some(full) = head.claim_version() else {
            return Err(ReadError::new(
                "unsupported",
                "the Simple view needs a runner that versions its history",
            ));
        };
        if request.before_cursor.is_some() {
            return Err(ReadError::new(
                "cursor_expired",
                "a Simple page goes on with beforeOrdinal, never with another page's cursor",
            ));
        }
        let version = crate::simple::version(&full);
        let limit = request.limit as usize;
        match request.before_ordinal {
            Some(before) => {
                self.simple
                    .older(
                        source,
                        &request.session_id,
                        head,
                        &version,
                        before as u64,
                        limit,
                    )
                    .await
            }
            None => {
                self.simple
                    .newest(source, &request.session_id, head, &version, limit)
                    .await
            }
        }
    }

    fn record(&self, session: &str, snapshot: TranscriptPageSnapshot) {
        self.claims
            .lock()
            .expect("transcript claims poisoned")
            .record(session, snapshot);
    }

    pub fn trimmed_newest(
        &self,
        session: &str,
        version: &str,
        newest: &Value,
        fits: &mut (dyn FnMut(&Value) -> bool + Send),
    ) -> Option<NewestPageFit> {
        let lineage = newest
            .get("historyVersion")
            .and_then(Value::as_u64)
            .map(|version| format!("history:{version}"));
        let snapshot = TranscriptPageSnapshot::from_page(version, lineage, newest.clone()).ok()?;
        let fit = snapshot.fit_newest(OlderPageCursor::BelowOldestRetained, fits);
        // A Simple page's version names its view, and its copies are kept
        // apart from the full ones.
        let key = if crate::simple::is_version(version) {
            crate::simple::claims_key(session)
        } else {
            session.to_owned()
        };
        match &fit {
            NewestPageFit::Whole => self.record(&key, snapshot),
            NewestPageFit::Trimmed(snapshot) => self.record(&key, snapshot.clone()),
            NewestPageFit::NewestTooLarge => {}
        }
        Some(fit)
    }

    pub fn remove_session(&self, session: &str) {
        self.claims
            .lock()
            .expect("transcript claims poisoned")
            .remove_session(session);
        self.claims
            .lock()
            .expect("transcript claims poisoned")
            .remove_session(&crate::simple::claims_key(session));
        self.simple.forget(session);
        self.revisions
            .lock()
            .expect("transcript revisions poisoned")
            .remove(session);
        self.activity.invalidate_transcript_activity_token(session);
    }
}

fn map_page(
    page: HistoryPage,
    head: &Head,
    history: bool,
    boundary: Option<Anchor>,
) -> Result<Value, ReadError> {
    let mut rows = page.rows;
    let mut ids = HashSet::new();
    let mut last_ordinal = None;
    let mut numbered = false;
    for row in &rows {
        if !ids.insert(&row.mid) || (numbered && row.ordinal.is_none()) {
            return Err(ReadError::new(
                "identity_unsupported",
                "history rows are repeated or out of order",
            ));
        }
        if let Some(ordinal) = row.ordinal {
            if last_ordinal.is_some_and(|last| ordinal <= last) {
                return Err(ReadError::new(
                    "identity_unsupported",
                    "history ordinals are not increasing",
                ));
            }
            last_ordinal = Some(ordinal);
            numbered = true;
        }
    }
    let has_unnumbered = rows.iter().any(|row| row.ordinal.is_none());
    let has_numbered = rows.iter().any(|row| row.ordinal.is_some());
    let mut cursor = page.older.map(Anchor::Cursor);
    // Newest pages and claim baselines never lend unnumbered rows to the
    // ordinal-based claim protocol. The boundary is an exclusive message anchor.
    if !history && has_unnumbered {
        cursor = Some(rows.iter().find(|row| row.ordinal.is_some()).map_or_else(
            || boundary.unwrap_or(Anchor::Newest),
            |row| Anchor::Mid(row.mid.clone()),
        ));
        rows.retain(|row| row.ordinal.is_some());
    } else if !history
        && rows.first().and_then(|row| row.ordinal) == head.first_ordinal
        && head.first_ordinal.is_some()
        && cursor.is_some()
    {
        cursor = Some(Anchor::Mid(rows[0].mid.clone()));
    } else if history && has_unnumbered && has_numbered {
        return Err(ReadError::new(
            "identity_unsupported",
            "a history window mixes numbered and unnumbered rows",
        ));
    }
    let mut reply =
        json!({"messages": rows.iter().map(Row::value).collect::<Result<Vec<_>, _>>()?});
    if let Some(anchor) = cursor {
        if let Some(ordinal) = rows.first().and_then(|row| row.ordinal) {
            reply["nextBeforeOrdinal"] = json!(ordinal);
        } else {
            reply["nextBeforeCursor"] = json!(serde_json::to_string(&HistoryCursor {
                anchor,
                version: head.claim_version()
            })
            .expect("a history cursor serializes"));
        }
        // A numbered boundary above unnumbered history must be traversed with
        // beforeCursor, not beforeOrdinal, even if its ordinal is known.
        if !history
            && has_numbered
            && (has_unnumbered || rows.first().and_then(|row| row.ordinal) == head.first_ordinal)
        {
            let mid = rows.first().expect("numbered rows remain").mid.clone();
            reply
                .as_object_mut()
                .expect("a page is an object")
                .remove("nextBeforeOrdinal");
            reply["nextBeforeCursor"] = json!(serde_json::to_string(&HistoryCursor {
                anchor: Anchor::Mid(mid),
                version: head.claim_version()
            })
            .expect("a history cursor serializes"));
        }
    } else {
        reply["reachedStart"] = json!(true);
    }
    Ok(reply)
}

/// A Simple page's reply: its rows, then where an older page goes on, which
/// is where the read stopped and can be older than its oldest row, or that
/// it reached the start.
fn map_simple_page(page: HistoryPage, next_before: Option<u64>) -> Result<Value, ReadError> {
    let messages = page
        .rows
        .iter()
        .map(Row::value)
        .collect::<Result<Vec<_>, _>>()?;
    let mut reply = json!({ "messages": messages });
    match next_before {
        Some(ordinal) => reply["nextBeforeOrdinal"] = json!(ordinal),
        None => reply["reachedStart"] = json!(true),
    }
    Ok(reply)
}

fn decorate(reply: &mut Value, head: &Head, source: &dyn TranscriptSource) {
    if let Some(version) = head.history_version {
        reply["historyVersion"] = json!(version);
    }
    match head.input.attachments {
        Attachments::Yes => {
            reply["acceptsAttachments"] = json!(true);
            if let Some(modalities) = &head.input.modalities {
                reply["acceptedModalities"] = json!(modalities);
            }
        }
        Attachments::No => reply["acceptsAttachments"] = json!(false),
        Attachments::Unknown => {}
    }
    if let Some(types) = &head.input.mime_types {
        reply["acceptedMimeTypes"] = json!(types);
    }
    if let Some((activity, at)) = source.activity() {
        reply["activity"] = json!(activity);
        reply["activityAtMs"] = json!(at);
    }
}

fn bounded(reply: Value) -> Result<Value, ReadError> {
    page::check_reply_size(
        serde_json::to_vec(&reply)
            .expect("a JSON reply serializes")
            .len(),
    )?;
    Ok(reply)
}
