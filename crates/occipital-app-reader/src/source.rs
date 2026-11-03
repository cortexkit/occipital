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
    if let Some(error) = error {
        let mut error_value = json!({"class": error.class});
        if let Some(retry) = error.retry_after_secs {
            error_value["retryAfterSecs"] = json!(retry);
        }
        if let Some(resets) = error.resets_at_ms {
            error_value["resetsAtMs"] = json!(resets);
        }
        value["error"] = error_value;
    }
    value
}

impl Head {
    fn claim_version(&self) -> Option<String> {
        self.history_version.map(|version| match &self.incarnation {
            Some(incarnation) => format!("history:{version}:incarnation:{incarnation}"),
            None => format!("history:{version}"),
        })
    }

    /// The session facts every newest page and every turn-carrying claim reply
    /// repeats from the head read in the same request: the next send's model and
    /// whether the app may send, and the most recent run's outcome.
    pub(crate) fn describe_session(&self, reply: &mut Value) {
        if let Some(model) = &self.model {
            reply["model"] = model.clone();
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
    fn changes(&self) -> watch::Receiver<u64>;
    fn activity(&self) -> Option<(&'static str, i64)>;
    /// A disconnected feed cannot authorize an activity-token cache hit.
    fn activity_feed_live(&self) -> bool {
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

    /// The same page body before the transport-size guard. A watch needs to
    /// inspect even very large deltas so it can replace them with a fitted
    /// resync; this result must never be sent directly as a transport reply.
    pub async fn read_for_watch(
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
                    let summary = source
                        .older(&Anchor::Ordinal(ordinal as u64 + 1), 1)
                        .await
                        .map(|page| page.rows)
                        .unwrap_or_default();
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
                let page = if let Some(cursor) = request.before_cursor.as_deref() {
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
                let numbered_anchor = if let Some(held) = request.claim.as_ref().filter(|held| {
                    head.claim_version().as_deref() == Some(held.held_version.as_str())
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
                Ok::<_, ReadError>((head, page, boundary, numbered_anchor))
            })
            .await?;
        let (head, history, boundary, numbered_anchor) = stable.page;
        let mut mapped = map_page(history, &head, request.before_cursor.is_some(), boundary)?;
        // A newest page without rows that reached the start is a session with no
        // messages. Apps read a page without rows by its `emptyReason`, so it
        // names that; a resync built from this page carries it too.
        if newest_request
            && mapped["messages"].as_array().is_some_and(Vec::is_empty)
            && mapped["reachedStart"] == true
        {
            mapped["emptyReason"] = json!("empty");
        }
        let version = head
            .claim_version()
            .unwrap_or_else(|| self.activity.ensure_transcript_version(&request.session_id));
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
                self.record(&request.session_id, current);
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
            let current = match TranscriptPageSnapshot::from_page(
                &version,
                head.claim_version(),
                mapped.clone(),
            ) {
                Ok(snapshot) => snapshot,
                Err(_) => return Ok(claim::identity_unsupported_reply()),
            };
            let mut reply = if native {
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
                let previous = self
                    .claims
                    .lock()
                    .expect("transcript claims poisoned")
                    .held_snapshot(&request.session_id, held);
                claim::claim_reply(
                    &version,
                    claim::evaluate_claim(held, previous.as_ref(), &current),
                )
            };
            self.record(&request.session_id, current);
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
                    self.record(&request.session_id, snapshot);
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
        match &fit {
            NewestPageFit::Whole => self.record(session, snapshot),
            NewestPageFit::Trimmed(snapshot) => self.record(session, snapshot.clone()),
            NewestPageFit::NewestTooLarge => {}
        }
        Some(fit)
    }

    pub fn remove_session(&self, session: &str) {
        self.claims
            .lock()
            .expect("transcript claims poisoned")
            .remove_session(session);
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
