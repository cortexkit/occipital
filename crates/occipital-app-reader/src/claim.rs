//! Transcript claim parsing, bounded snapshots, and claim-reply construction.
//!
//! Message ids are the only claim anchors. Ordinals remain page-order metadata and are excluded
//! from held-material comparisons, so changing page ordinals alone does not change the comparison.

use std::collections::{HashMap, VecDeque};

use serde_json::{json, Map, Value};

// A slow client can retain several versions while turns arrive, but process-local history must not
// grow without bound. Missing evicted versions honestly evaluate as cursor_expired.
const MAX_SNAPSHOTS_PER_SESSION: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptClaim {
    pub held_floor_message_id: String,
    pub since_message_id: String,
    pub held_version: String,
}

/// What a request's `claim` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The app holds messages: the oldest and newest it holds, and their version.
    Held(TranscriptClaim),
    /// The app holds no messages: the newest page it read was empty at this
    /// version, so it sends `heldVersion` alone. The answer is `{unchanged: true}`
    /// while the newest page is still empty at that version, and otherwise
    /// `cursor_expired` with the newest page as `resync`, the verdict an app
    /// already applies to a held page that was replaced.
    Empty { held_version: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRefusal {
    pub code: &'static str,
    pub message: String,
}

impl ClaimRefusal {
    pub fn reply(&self) -> Value {
        error_reply(self.code, &self.message, None)
    }
}

/// Older clients reject unfamiliar verdicts, so a new verdict requires explicit acceptance.
pub fn accepts_outside_window(params: &Value) -> Result<bool, ClaimRefusal> {
    let outside_window = if let Some(raw) = params.get("acceptClaims") {
        let verdicts = raw
            .as_array()
            .filter(|values| values.iter().all(Value::is_string))
            .ok_or_else(|| ClaimRefusal {
                code: "invalid_claim",
                message: "acceptClaims must be an array of strings".into(),
            })?;
        verdicts.iter().any(|verdict| verdict == "outside_window")
    } else {
        false
    };
    Ok(outside_window)
}

pub fn parse_claim(params: &Value) -> Result<Option<Claim>, ClaimRefusal> {
    let Some(raw_claim) = params.get("claim") else {
        return Ok(None);
    };
    if params.get("activityToken").is_some() {
        return Err(ClaimRefusal {
            code: "claim_conflicts_with_activity_token",
            message: "a claim carries its own change probe (heldVersion); send claim or activityToken, never both".to_string(),
        });
    }
    if params.get("beforeOrdinal").is_some() || params.get("beforeCursor").is_some() {
        return Err(ClaimRefusal {
            code: "claim_conflicts_with_history_cursor",
            message: if params.get("beforeCursor").is_some() {
                "a claim addresses the newest end of held material; page history with beforeCursor separately"
            } else {
                "a claim addresses the newest end of held material; page history with beforeOrdinal separately"
            }.to_string(),
        });
    }

    let claim = raw_claim.as_object().ok_or_else(|| ClaimRefusal {
        code: "invalid_claim",
        message: "claim must be an object".to_string(),
    })?;
    if !claim.contains_key("heldFloorMessageId") && !claim.contains_key("sinceMessageId") {
        return Ok(Some(Claim::Empty {
            held_version: required_claim_string(claim, "heldVersion")?,
        }));
    }
    Ok(Some(Claim::Held(TranscriptClaim {
        held_floor_message_id: required_claim_string(claim, "heldFloorMessageId")?,
        since_message_id: required_claim_string(claim, "sinceMessageId")?,
        held_version: required_claim_string(claim, "heldVersion")?,
    })))
}

fn required_claim_string(
    claim: &Map<String, Value>,
    field: &'static str,
) -> Result<String, ClaimRefusal> {
    claim
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| ClaimRefusal {
            code: "invalid_claim",
            message: format!("claim.{field} must be a non-empty string"),
        })
}

pub fn diagnostics(identity: &str, delta_read: &str) -> Value {
    json!({
        "identity": identity,
        "deltaRead": delta_read,
        "invalidation": "none",
    })
}

pub fn error_reply(code: &str, message: &str, diagnostics: Option<Value>) -> Value {
    let mut reply = json!({
        "error": {
            "code": code,
            "message": message,
            "terminal": true,
        }
    });
    if let Some(diagnostics) = diagnostics {
        reply["diagnostics"] = diagnostics;
    }
    reply
}

pub fn durable_error_reply(code: &str, message: &str) -> Value {
    error_reply(code, message, Some(diagnostics("durable", "scan")))
}

pub fn harness_mismatch_reply(requested: &str, session_id: &str, actual: &str) -> Value {
    // The recorded refusals report scan support for Thalamus's harnesses (OpenCode
    // 1, as they name it, OpenCode 2 and Pi), and none for other harnesses. Claims
    // a runner answers itself are refused by `runner_claim_reply` instead; this
    // refusal keeps its recorded shape.
    let delta_read = if matches!(actual, "opencode" | "opencode2" | "pi") {
        "scan"
    } else {
        "none"
    };
    error_reply(
        "harness_mismatch",
        &format!(
            "request asserted harness '{requested}' but session '{session_id}' resolves to '{actual}'"
        ),
        Some(diagnostics("durable", delta_read)),
    )
}

pub fn identity_unsupported_reply() -> Value {
    error_reply(
        "identity_unsupported",
        "session's transcript arm declares identity: none; claims require durable identity - fetch full pages",
        Some(diagnostics("none", "none")),
    )
}

pub fn observation_unstable_reply(delta_read: &str) -> Value {
    json!({
        "error": {
            "code": "observation_unstable",
            "message": "session changed during both observation attempts; retry the claim",
            "terminal": false,
        },
        "diagnostics": diagnostics("durable", delta_read),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerLineageDisposition {
    Matched,
    CursorExpired,
}

/// Runner lineage ids are opaque generation claims: compare their bytes only.
/// Their spelling carries no ordering, UUID, or parseable cursor semantics.
pub fn discriminate_runner_lineage(
    held_lineage_id: &str,
    current_lineage_id: Option<&str>,
) -> RunnerLineageDisposition {
    if current_lineage_id == Some(held_lineage_id) {
        RunnerLineageDisposition::Matched
    } else {
        RunnerLineageDisposition::CursorExpired
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunnerClaimEvaluation {
    /// `vouched_from_message_id` is required: an equal native lineage proves the whole claimed
    /// range unchanged, so the runner arm vouches from the claim's held floor.
    Matched {
        new_turns: Vec<Value>,
        vouched_from_message_id: String,
    },
    CursorExpired {
        resync: Value,
    },
    OutsideWindow {
        resync: Value,
    },
}

pub fn runner_claim_reply(
    version: &str,
    has_native_lineage: bool,
    evaluation: RunnerClaimEvaluation,
) -> Value {
    let diagnostics = diagnostics(
        "durable",
        if has_native_lineage { "native" } else { "none" },
    );
    let mut reply = match evaluation {
        RunnerClaimEvaluation::Matched {
            new_turns,
            vouched_from_message_id,
        } => json!({
            "claim": "matched",
            "version": version,
            "newTurns": new_turns,
            "vouchedFromMessageId": vouched_from_message_id,
            "diagnostics": diagnostics,
        }),
        RunnerClaimEvaluation::CursorExpired { resync } => json!({
            "claim": "cursor_expired",
            "version": version,
            "resync": resync,
            "diagnostics": diagnostics,
        }),
        RunnerClaimEvaluation::OutsideWindow { resync } => json!({
            "claim": "outside_window",
            "version": version,
            "resync": resync,
            "diagnostics": diagnostics,
        }),
    };
    remove_module_decorations_from_resync(&mut reply);
    reply
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptPageSnapshot {
    version: String,
    lineage_id: Option<String>,
    page: Value,
    rows: Vec<TranscriptRow>,
    /// True when older rows of the observed newest page were dropped so the page fits a
    /// size bound. Claim lookup (`TranscriptClaimState::held_snapshot`) uses a trimmed
    /// snapshot only for a claim whose heldFloorMessageId is one of the snapshot's rows.
    trimmed: bool,
}

/// How a transcript provider's `beforeOrdinal` read addresses older messages, which decides the
/// `nextBeforeOrdinal` a trimmed newest page carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OlderPageCursor {
    /// `beforeOrdinal` is exclusive and the read returns the newest rows below
    /// it. The trimmed page's cursor is its oldest retained ordinal, so the read starts with the
    /// newest trimmed-out message.
    BelowOldestRetained,
    /// Legacy Broca compatibility: `beforeOrdinal` was the inclusive start of a forward read (its
    /// `session.read from_ordinal`). The trimmed page's cursor is the ordinal of the oldest
    /// trimmed-out message. A read with that cursor and `limit` equal to the number of trimmed
    /// rows returns exactly the trimmed-out messages; a larger limit continues into the
    /// retained rows, which the reader already holds and merges by message id. It never skips
    /// a message.
    FromOldestTrimmed,
}

/// The result of fitting an observed newest page into a size bound.
#[derive(Debug, Clone, PartialEq)]
pub enum NewestPageFit {
    /// The whole page fits; nothing was trimmed.
    Whole,
    /// The newest rows that fit, as a snapshot marked trimmed.
    Trimmed(TranscriptPageSnapshot),
    /// Even the newest message alone does not fit.
    NewestTooLarge,
}

#[derive(Debug, Clone, PartialEq)]
struct TranscriptRow {
    message_id: String,
    ordinal: i64,
    body: Value,
}

/// A page without durable row identity or renderable content cannot support a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTranscriptPage;

impl TranscriptPageSnapshot {
    pub fn from_page(
        version: impl Into<String>,
        lineage_id: Option<String>,
        page: Value,
    ) -> Result<Self, InvalidTranscriptPage> {
        let messages = page
            .get("messages")
            .and_then(Value::as_array)
            .ok_or(InvalidTranscriptPage)?;
        let mut rows = Vec::with_capacity(messages.len());
        for message in messages {
            let object = message.as_object().ok_or(InvalidTranscriptPage)?;
            let message_id = object
                .get("messageId")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or(InvalidTranscriptPage)?
                .to_string();
            let ordinal = object
                .get("ordinal")
                .and_then(Value::as_i64)
                .ok_or(InvalidTranscriptPage)?;
            if !object.get("role").is_some_and(Value::is_string)
                || !object.get("text").is_some_and(Value::is_string)
                || !object.get("blockSummaries").is_some_and(Value::is_array)
                || !object.get("parts").is_some_and(Value::is_array)
            {
                return Err(InvalidTranscriptPage);
            }
            // Identity and content determine whether held material changed. The ordinal is retained
            // for response ordering but cannot participate in the comparison.
            let mut body = message.clone();
            body.as_object_mut()
                .expect("checked above")
                .remove("ordinal");
            rows.push(TranscriptRow {
                message_id,
                ordinal,
                body,
            });
        }
        rows.sort_by_key(|row| row.ordinal);
        Ok(Self {
            version: version.into(),
            lineage_id,
            page,
            rows,
            trimmed: false,
        })
    }

    pub fn lineage_id(&self) -> Option<&str> {
        self.lineage_id.as_deref()
    }

    pub fn is_trimmed(&self) -> bool {
        self.trimmed
    }

    /// Mark a snapshot built from a trimmed one (for example a trimmed held page extended by
    /// newly read turns) so lookup keeps treating it as lacking the older rows.
    pub fn with_trimmed(mut self, trimmed: bool) -> Self {
        self.trimmed = trimmed;
        self
    }

    /// The page in the form a `cursor_expired` or `diverged` claim reply carries it as its
    /// `resync` field (the page the client replaces its held messages with): rows in
    /// conversation order plus the page metadata the snapshot holds.
    pub fn resync_page(&self) -> Value {
        self.page_with_rows(&self.rows)
    }

    /// Keep the most newest rows for which `fits` accepts the resulting page, dropping rows
    /// from the oldest end. `fits` receives the page exactly as [`Self::resync_page`] would
    /// serve it, so the caller can wrap it in the reply it measures. The binary search assumes
    /// that `fits` remains true as fewer rows are kept.
    pub fn fit_newest(
        &self,
        cursor: OlderPageCursor,
        mut fits: impl FnMut(&Value) -> bool,
    ) -> NewestPageFit {
        if fits(&self.resync_page()) {
            return NewestPageFit::Whole;
        }
        let total = self.rows.len();
        if total < 2 {
            return NewestPageFit::NewestTooLarge;
        }
        // Invariant: keeping `low` rows fits (0 stands for "none checked yet") and keeping
        // `high` rows does not.
        let (mut low, mut high) = (0, total);
        let mut best = None;
        while high - low > 1 {
            let keep = low + (high - low) / 2;
            let candidate = self.trimmed_to_newest(keep, cursor);
            if fits(&candidate.resync_page()) {
                low = keep;
                best = Some(candidate);
            } else {
                high = keep;
            }
        }
        best.map_or(NewestPageFit::NewestTooLarge, NewestPageFit::Trimmed)
    }

    /// The newest `keep` rows (1 <= keep < rows) as a trimmed snapshot under the same version
    /// and lineage. The page metadata is corrected for the dropped rows: `nextBeforeOrdinal`
    /// addresses the trimmed-out messages through `cursor`, and `olderUnavailable` and
    /// `emptyReason` are removed because older messages are, by construction, available.
    fn trimmed_to_newest(&self, keep: usize, cursor: OlderPageCursor) -> Self {
        let first_kept = self.rows.len() - keep;
        let rows = self.rows[first_kept..].to_vec();
        let next_before_ordinal = match cursor {
            OlderPageCursor::BelowOldestRetained => rows[0].ordinal,
            OlderPageCursor::FromOldestTrimmed => self.rows[0].ordinal,
        };
        let mut page = self.page_with_rows(&rows);
        if let Some(object) = page.as_object_mut() {
            object.remove("olderUnavailable");
            object.remove("emptyReason");
            object.remove("nextBeforeCursor");
            object.remove("reachedStart");
            object.insert("nextBeforeOrdinal".to_string(), json!(next_before_ordinal));
        }
        Self {
            version: self.version.clone(),
            lineage_id: self.lineage_id.clone(),
            page,
            rows,
            trimmed: true,
        }
    }

    pub fn ordinal_for(&self, message_id: &str) -> Option<i64> {
        self.rows
            .iter()
            .find(|row| row.message_id == message_id)
            .map(|row| row.ordinal)
    }

    pub fn contains(&self, message_id: &str) -> bool {
        self.rows.iter().any(|row| row.message_id == message_id)
    }

    pub fn same_page_material(&self, other: &Self) -> bool {
        self.lineage_id == other.lineage_id
            && self.rows.len() == other.rows.len()
            && self
                .rows
                .iter()
                .zip(&other.rows)
                .all(|(left, right)| left.message_id == right.message_id && left.body == right.body)
    }

    fn chronological_values(&self) -> Vec<Value> {
        self.rows
            .iter()
            .map(|row| {
                let mut value = row.body.clone();
                value["ordinal"] = json!(row.ordinal);
                value
            })
            .collect()
    }

    fn page_with_rows(&self, rows: &[TranscriptRow]) -> Value {
        let mut page = self.page.clone();
        page["messages"] = Value::Array(
            rows.iter()
                .map(|row| {
                    let mut value = row.body.clone();
                    value["ordinal"] = json!(row.ordinal);
                    value
                })
                .collect(),
        );
        page
    }
}

#[derive(Debug, Default)]
pub struct TranscriptClaimState {
    snapshots: HashMap<String, VecDeque<TranscriptPageSnapshot>>,
}

impl TranscriptClaimState {
    /// The newest untrimmed snapshot at `version` that contains `since_message_id`. A trimmed
    /// snapshot lacks the older rows of its page, so it is never served without a floor check.
    pub fn snapshot(
        &self,
        session_id: &str,
        version: &str,
        since_message_id: Option<&str>,
    ) -> Option<TranscriptPageSnapshot> {
        self.snapshots
            .get(session_id)?
            .iter()
            .rev()
            .find(|snapshot| {
                !snapshot.trimmed
                    && snapshot.version == version
                    && since_message_id.is_none_or(|message_id| snapshot.contains(message_id))
            })
            .cloned()
    }

    /// The snapshot a claim is evaluated against: the newest snapshot at the claimed version
    /// that contains both sinceMessageId and heldFloorMessageId, trimmed or not; failing that,
    /// [`Self::snapshot`]'s choice among untrimmed snapshots. A trimmed snapshot therefore
    /// serves only claimants whose floor it holds, and a claimant whose floor lies below every
    /// snapshot (history paged with beforeOrdinal) is compared against a full page.
    pub fn held_snapshot(
        &self,
        session_id: &str,
        claim: &TranscriptClaim,
    ) -> Option<TranscriptPageSnapshot> {
        self.snapshots
            .get(session_id)?
            .iter()
            .rev()
            .find(|snapshot| {
                snapshot.version == claim.held_version
                    && snapshot.contains(&claim.since_message_id)
                    && snapshot.contains(&claim.held_floor_message_id)
            })
            .cloned()
            .or_else(|| {
                self.snapshot(
                    session_id,
                    &claim.held_version,
                    Some(&claim.since_message_id),
                )
            })
    }

    pub fn record(&mut self, session_id: &str, snapshot: TranscriptPageSnapshot) {
        let snapshots = self.snapshots.entry(session_id.to_string()).or_default();
        // The same version, lineage, and row material make this snapshot redundant, unless only
        // a trimmed copy is recorded: lookup without a floor match needs the untrimmed copy.
        if snapshots.iter().any(|existing| {
            existing.version == snapshot.version
                && existing.same_page_material(&snapshot)
                && (!existing.trimmed || snapshot.trimmed)
        }) {
            return;
        }
        snapshots.push_back(snapshot);
        while snapshots.len() > MAX_SNAPSHOTS_PER_SESSION {
            snapshots.pop_front();
        }
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.snapshots.remove(session_id);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClaimEvaluation {
    /// Every matched reply names the oldest message the server compared and therefore vouches
    /// for. Clients treat a matched reply without it as a decode failure, so there is no
    /// "absent means the whole claimed range" reading.
    Matched {
        new_turns: Vec<Value>,
        vouched_from_message_id: String,
    },
    /// `vouched_from_message_id` is present only when the claimed floor lies below the server's
    /// retained window, bounding which held messages the divergence verdict speaks for.
    Diverged {
        valid_through_message_id: Option<String>,
        resync: Value,
        vouched_from_message_id: Option<String>,
    },
    CursorExpired {
        resync: Value,
    },
}

pub fn evaluate_claim(
    claim: &TranscriptClaim,
    held: Option<&TranscriptPageSnapshot>,
    current: &TranscriptPageSnapshot,
) -> ClaimEvaluation {
    let Some(held) = held else {
        return ClaimEvaluation::CursorExpired {
            resync: current.page_with_rows(&current.rows),
        };
    };
    if held.lineage_id != current.lineage_id || !current.contains(&claim.since_message_id) {
        return ClaimEvaluation::CursorExpired {
            resync: current.page_with_rows(&current.rows),
        };
    }
    let Some(held_since) = held
        .rows
        .iter()
        .position(|row| row.message_id == claim.since_message_id)
    else {
        return ClaimEvaluation::CursorExpired {
            resync: current.page_with_rows(&current.rows),
        };
    };
    let current_since = current
        .rows
        .iter()
        .position(|row| row.message_id == claim.since_message_id)
        .expect("checked above");

    // If the requested held range extends below the server's retained window, compare only from
    // the current window floor and return that floor id as the earliest content actually verified.
    let current_floor = current
        .rows
        .iter()
        .position(|row| row.message_id == claim.held_floor_message_id);
    let current_start = current_floor.unwrap_or(0).min(current_since);
    // The comparison below covers current rows from current_start through sinceMessageId, so the
    // row at current_start is the oldest message the server vouches for. It always exists because
    // current_since indexes a row and current_start <= current_since. That row is the held floor
    // when the floor is inside the window, the window's oldest row when the floor is below it, and
    // sinceMessageId itself when the claimed floor sits after sinceMessageId.
    let vouched_from_message_id = current.rows[current_start].message_id.clone();
    let below_window_vouch = current_floor
        .is_none()
        .then(|| vouched_from_message_id.clone());
    let held_floor = held
        .rows
        .iter()
        .position(|row| row.message_id == claim.held_floor_message_id);
    let held_start = if let Some(current_oldest) = current.rows.get(current_start) {
        held.rows
            .iter()
            .position(|row| row.message_id == current_oldest.message_id)
            .or(held_floor)
            .unwrap_or(0)
    } else {
        held_floor.unwrap_or(0)
    }
    .min(held_since);

    let held_scope = &held.rows[held_start..=held_since];
    let current_scope = &current.rows[current_start..=current_since];
    // Compare ids and row fields other than ordinal in conversation order. Appends after sinceMessageId
    // are returned as newTurns and therefore never enter this held-material comparison.
    let first_mismatch = held_scope
        .iter()
        .zip(current_scope)
        .position(|(old, new)| old.message_id != new.message_id || old.body != new.body)
        .or_else(|| {
            (held_scope.len() != current_scope.len())
                .then_some(held_scope.len().min(current_scope.len()))
        });

    if let Some(relative_index) = first_mismatch {
        let affected_current_index = current_start
            .saturating_add(relative_index)
            .min(current.rows.len());
        let valid_through_message_id = relative_index
            .checked_sub(1)
            .and_then(|index| held_scope.get(index))
            .map(|row| row.message_id.clone());
        return ClaimEvaluation::Diverged {
            valid_through_message_id,
            resync: current.page_with_rows(&current.rows[affected_current_index..]),
            vouched_from_message_id: below_window_vouch,
        };
    }

    ClaimEvaluation::Matched {
        new_turns: current.rows[current_since.saturating_add(1)..]
            .iter()
            .map(|row| {
                let mut value = row.body.clone();
                value["ordinal"] = json!(row.ordinal);
                value
            })
            .collect(),
        vouched_from_message_id,
    }
}

pub fn claim_reply(version: &str, evaluation: ClaimEvaluation) -> Value {
    let mut reply = match evaluation {
        ClaimEvaluation::Matched {
            new_turns,
            vouched_from_message_id,
        } => {
            json!({
                "claim": "matched",
                "version": version,
                "newTurns": new_turns,
                "vouchedFromMessageId": vouched_from_message_id,
                "diagnostics": diagnostics("durable", "scan"),
            })
        }
        ClaimEvaluation::Diverged {
            valid_through_message_id,
            resync,
            vouched_from_message_id,
        } => {
            let mut reply = json!({
                "claim": "diverged",
                "version": version,
                "resync": resync,
                "diagnostics": diagnostics("durable", "scan"),
            });
            if let Some(message_id) = valid_through_message_id {
                reply["validThroughMessageId"] = json!(message_id);
            }
            if let Some(message_id) = vouched_from_message_id {
                reply["vouchedFromMessageId"] = json!(message_id);
            }
            reply
        }
        ClaimEvaluation::CursorExpired { resync } => json!({
            "claim": "cursor_expired",
            "version": version,
            "resync": resync,
            "diagnostics": diagnostics("durable", "scan"),
        }),
    };
    remove_module_decorations_from_resync(&mut reply);
    reply
}

fn remove_module_decorations_from_resync(reply: &mut Value) {
    let Some(resync) = reply.get_mut("resync").and_then(Value::as_object_mut) else {
        return;
    };
    for field in [
        "activityToken",
        "activity",
        "activityAtMs",
        "attachmentCapability",
        "timing",
    ] {
        resync.remove(field);
    }
}

pub fn chronological_page_values(snapshot: &TranscriptPageSnapshot) -> Vec<Value> {
    snapshot.chronological_values()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn accept_claims_requires_strings_and_ignores_unknown_verdicts() {
        assert!(!accepts_outside_window(&json!({})).unwrap());
        for (accepted, expected) in [
            (json!([]), false),
            (json!(["future_verdict"]), false),
            (json!(["outside_window"]), true),
            (json!(["future_verdict", "outside_window"]), true),
        ] {
            assert_eq!(
                accepts_outside_window(&json!({"acceptClaims":accepted})).unwrap(),
                expected
            );
        }
        for accepted in [
            Value::Null,
            json!(false),
            json!(1),
            json!("outside_window"),
            json!({}),
            json!(["outside_window", 1]),
            json!([null]),
        ] {
            let error = accepts_outside_window(&json!({"acceptClaims":accepted})).unwrap_err();
            assert_eq!(error.code, "invalid_claim");
            assert_eq!(error.message, "acceptClaims must be an array of strings");
        }
    }

    fn message(ordinal: i64, message_id: &str, text: &str) -> Value {
        json!({
            "ordinal": ordinal,
            "role": if ordinal % 2 == 0 { "user" } else { "assistant" },
            "text": text,
            "blockSummaries": [],
            "parts": [{ "type": "text", "text": text }],
            "messageId": message_id,
        })
    }

    fn page(messages: Vec<Value>) -> Value {
        json!({
            "messages": messages,
            "reportsCompletion": true,
            "nextBeforeOrdinal": null,
        })
    }

    fn snapshot(version: &str, lineage: u64, messages: Vec<Value>) -> TranscriptPageSnapshot {
        TranscriptPageSnapshot::from_page(version, Some(lineage.to_string()), page(messages))
            .unwrap()
    }

    fn claim(floor: &str, since: &str, version: &str) -> TranscriptClaim {
        TranscriptClaim {
            held_floor_message_id: floor.to_string(),
            since_message_id: since.to_string(),
            held_version: version.to_string(),
        }
    }

    /// `heldVersion` alone is the claim of an app holding no messages. Naming one
    /// held message without the other is still refused.
    #[test]
    fn a_claim_with_only_the_held_version_holds_no_messages() {
        assert_eq!(
            parse_claim(&json!({"claim":{"heldVersion":"v"}})).unwrap(),
            Some(Claim::Empty {
                held_version: "v".into()
            })
        );
        for claim in [
            json!({}),
            json!({"heldVersion":""}),
            json!({"heldVersion":null}),
        ] {
            let error = parse_claim(&json!({ "claim": claim })).unwrap_err();
            assert_eq!(error.code, "invalid_claim");
            assert!(error.message.contains("heldVersion"), "{error:?}");
        }
        for (field, missing) in [
            ("heldFloorMessageId", "sinceMessageId"),
            ("sinceMessageId", "heldFloorMessageId"),
        ] {
            let error =
                parse_claim(&json!({"claim":{"heldVersion":"v", (field):"a"}})).unwrap_err();
            assert_eq!(error.code, "invalid_claim");
            assert!(error.message.contains(missing), "{error:?}");
        }
        // An empty claim conflicts with the other request forms as any claim does.
        assert_eq!(
            parse_claim(&json!({"activityToken":"t", "claim":{"heldVersion":"v"}}))
                .unwrap_err()
                .code,
            "claim_conflicts_with_activity_token"
        );
    }

    #[test]
    fn claim_requires_an_object_and_three_nonempty_string_anchors() {
        assert_eq!(parse_claim(&json!({})).unwrap(), None);
        for raw in [json!(null), json!("claim"), json!([])] {
            assert_eq!(
                parse_claim(&json!({"claim":raw})).unwrap_err().code,
                "invalid_claim"
            );
        }
        for field in ["heldFloorMessageId", "sinceMessageId", "heldVersion"] {
            for value in [None, Some(json!("")), Some(json!(null)), Some(json!(42))] {
                let mut request = json!({"claim":{
                    "heldFloorMessageId":"a", "sinceMessageId":"b", "heldVersion":"v"
                }});
                request["claim"].as_object_mut().unwrap().remove(field);
                if let Some(value) = value {
                    request["claim"][field] = value;
                }
                let error = parse_claim(&request).unwrap_err();
                assert_eq!(error.code, "invalid_claim");
                assert!(error.message.contains(field));
            }
        }
    }

    #[test]
    fn claims_cannot_page_unnumbered_history() {
        let request = json!({"beforeCursor":"opaque", "claim":{
            "heldFloorMessageId":"a", "sinceMessageId":"b", "heldVersion":"v"
        }});
        assert_eq!(
            parse_claim(&request).unwrap_err().code,
            "claim_conflicts_with_history_cursor"
        );
    }

    #[test]
    fn snapshots_require_durable_ids_numbered_rows_and_renderable_content() {
        for field in [
            "messageId",
            "ordinal",
            "role",
            "text",
            "blockSummaries",
            "parts",
        ] {
            let mut row = message(1, "a", "one");
            row.as_object_mut().unwrap().remove(field);
            assert!(
                TranscriptPageSnapshot::from_page("v", None, page(vec![row])).is_err(),
                "{field}"
            );
        }
        let mut row = message(1, "a", "one");
        row["messageId"] = json!("");
        assert!(TranscriptPageSnapshot::from_page("v", None, page(vec![row])).is_err());
    }

    #[test]
    fn claim_comparison_ignores_ordinals_but_returns_turns_in_ordinal_order() {
        let held = snapshot(
            "old",
            1,
            vec![message(1, "a", "one"), message(2, "b", "two")],
        );
        let current = snapshot(
            "new",
            1,
            vec![
                message(44, "d", "four"),
                message(22, "b", "two"),
                message(33, "c", "three"),
                message(11, "a", "one"),
            ],
        );
        let ClaimEvaluation::Matched {
            new_turns,
            vouched_from_message_id,
        } = evaluate_claim(&claim("a", "b", "old"), Some(&held), &current)
        else {
            panic!("renumbering is not a content rewrite");
        };
        assert_eq!(vouched_from_message_id, "a");
        assert_eq!(
            new_turns
                .iter()
                .map(|row| row["messageId"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["c", "d"]
        );
        assert_eq!(
            new_turns
                .iter()
                .map(|row| row["ordinal"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [33, 44]
        );
    }

    #[test]
    fn missing_baselines_or_since_anchors_expire_instead_of_vouching() {
        let current = snapshot("new", 1, vec![message(1, "a", "one")]);
        assert!(matches!(
            evaluate_claim(&claim("a", "a", "old"), None, &current),
            ClaimEvaluation::CursorExpired { .. }
        ));
        let held = snapshot(
            "old",
            1,
            vec![message(1, "a", "one"), message(2, "b", "two")],
        );
        assert!(matches!(
            evaluate_claim(&claim("a", "b", "old"), Some(&held), &current),
            ClaimEvaluation::CursorExpired { .. }
        ));
        assert!(matches!(
            evaluate_claim(&claim("a", "b", "old"), Some(&current), &held),
            ClaimEvaluation::CursorExpired { .. }
        ));
    }

    #[test]
    fn snapshot_retention_is_bounded_and_session_removal_is_local() {
        let mut state = TranscriptClaimState::default();
        for index in 0..=MAX_SNAPSHOTS_PER_SESSION {
            state.record(
                "s",
                snapshot(&format!("v{index}"), 1, vec![message(1, "a", "one")]),
            );
        }
        assert!(state.snapshot("s", "v0", None).is_none());
        assert!(state.snapshot("s", "v1", None).is_some());
        state.record("other", snapshot("v", 1, vec![message(1, "a", "one")]));
        state.remove_session("s");
        assert!(state.snapshot("s", "v1", None).is_none());
        assert!(state.snapshot("other", "v", None).is_some());
    }

    #[test]
    fn trimming_a_boundary_page_replaces_the_cursor_and_reached_start_marker() {
        let mut raw = page(vec![message(1, "a", "one"), message(2, "b", "two")]);
        raw["nextBeforeCursor"] = json!("pre-adoption");
        raw["reachedStart"] = json!(true);
        let snapshot = TranscriptPageSnapshot::from_page("v", None, raw).unwrap();
        let NewestPageFit::Trimmed(trimmed) =
            snapshot.fit_newest(OlderPageCursor::BelowOldestRetained, at_most_messages(1))
        else {
            panic!("must trim");
        };
        let page = trimmed.resync_page();
        assert_eq!(page["nextBeforeOrdinal"], 2);
        assert!(page.get("nextBeforeCursor").is_none());
        assert!(page.get("reachedStart").is_none());
    }

    #[test]
    fn runner_lineage_matches_equal_ids_and_otherwise_expires_the_cursor() {
        assert_eq!(
            discriminate_runner_lineage("lineage-a", Some("lineage-a")),
            RunnerLineageDisposition::Matched
        );
        assert_eq!(
            discriminate_runner_lineage("lineage-a", None),
            RunnerLineageDisposition::CursorExpired
        );
        assert_eq!(
            discriminate_runner_lineage("lineage-a", Some("lineage-b")),
            RunnerLineageDisposition::CursorExpired
        );
    }

    #[test]
    fn claim_and_activity_token_are_mutually_exclusive() {
        let params = json!({
            "activityToken": "activity-current",
            "claim": {
                "heldFloorMessageId": "m1",
                "sinceMessageId": "m2",
                "heldVersion": "version-1",
            }
        });
        let refusal = parse_claim(&params).unwrap_err();
        assert_eq!(refusal.code, "claim_conflicts_with_activity_token");
        assert_eq!(
            refusal.reply(),
            json!({
                "error": {
                    "code": "claim_conflicts_with_activity_token",
                    "message": "a claim carries its own change probe (heldVersion); send claim or activityToken, never both",
                    "terminal": true,
                }
            })
        );
    }

    #[test]
    fn claim_with_before_ordinal_is_refused() {
        let params = json!({
            "beforeOrdinal": 40,
            "claim": {
                "heldFloorMessageId": "m1",
                "sinceMessageId": "m2",
                "heldVersion": "version-1",
            }
        });
        let refusal = parse_claim(&params).unwrap_err();
        assert_eq!(refusal.code, "claim_conflicts_with_history_cursor");
        assert!(refusal.reply().get("diagnostics").is_none());
    }

    #[test]
    fn harness_mismatch_message_names_both_harnesses() {
        let reply = harness_mismatch_reply("broca", "ses_a1", "opencode2");
        assert_eq!(reply["error"]["code"], json!("harness_mismatch"));
        assert_eq!(
            reply["error"]["message"],
            json!("request asserted harness 'broca' but session 'ses_a1' resolves to 'opencode2'")
        );
        assert_eq!(reply["diagnostics"], diagnostics("durable", "scan"));
    }

    #[test]
    fn claim_matches_unchanged_prefix_and_diverges_on_held_message_edit() {
        let request = claim("m1", "m3", "version-1");
        let held = snapshot(
            "version-1",
            1,
            vec![
                message(1, "m1", "one"),
                message(2, "m2", "two"),
                message(3, "m3", "three"),
            ],
        );
        let unchanged = snapshot(
            "version-2",
            1,
            vec![
                message(1, "m1", "one"),
                message(2, "m2", "two"),
                message(3, "m3", "three"),
                message(4, "m4", "four"),
            ],
        );
        assert!(matches!(
            evaluate_claim(&request, Some(&held), &unchanged),
            ClaimEvaluation::Matched { .. }
        ));

        let mutated = snapshot(
            "version-3",
            1,
            vec![
                message(1, "m1", "one"),
                message(2, "m2", "two (edited)"),
                message(3, "m3", "three"),
                message(4, "m4", "four"),
            ],
        );
        match evaluate_claim(&request, Some(&held), &mutated) {
            ClaimEvaluation::Diverged {
                valid_through_message_id,
                resync,
                ..
            } => {
                assert_eq!(valid_through_message_id.as_deref(), Some("m1"));
                assert_eq!(resync["messages"][0]["messageId"], json!("m2"));
            }
            verdict => panic!("a held-message mutation must diverge, got {verdict:?}"),
        }
    }

    #[test]
    fn claim_cursor_expires_on_lineage_change() {
        let request = claim("m1", "m2", "version-1");
        let held = snapshot(
            "version-1",
            2,
            vec![message(1, "m1", "one"), message(2, "m2", "two")],
        );
        let recreated = snapshot(
            "version-2",
            3,
            vec![message(1, "m1", "fresh one"), message(2, "m2", "fresh two")],
        );
        assert!(matches!(
            evaluate_claim(&request, Some(&held), &recreated),
            ClaimEvaluation::CursorExpired { .. }
        ));
    }

    #[test]
    fn below_window_floor_is_matched_with_a_bounded_vouch() {
        let request = claim("m-ancient", "m3", "version-1");
        let held = snapshot(
            "version-1",
            1,
            vec![message(2, "m2", "two"), message(3, "m3", "three")],
        );
        let current = snapshot(
            "version-2",
            1,
            vec![message(2, "m2", "two"), message(3, "m3", "three")],
        );
        assert_eq!(
            evaluate_claim(&request, Some(&held), &current),
            ClaimEvaluation::Matched {
                new_turns: Vec::new(),
                vouched_from_message_id: "m2".to_string(),
            }
        );
    }

    fn messages_of(page: &Value) -> Vec<&str> {
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["messageId"].as_str().unwrap())
            .collect()
    }

    fn at_most_messages(count: usize) -> impl FnMut(&Value) -> bool {
        move |page: &Value| page["messages"].as_array().unwrap().len() <= count
    }

    fn host_page(messages: Vec<Value>) -> Value {
        json!({
            "messages": messages,
            "reportsCompletion": true,
            "nextBeforeOrdinal": null,
            "olderUnavailable": true,
            "emptyReason": "host_window_floor",
            "generation": 4,
        })
    }

    #[test]
    fn trimmed_newest_page_keeps_newest_rows_and_points_below_them_for_opencode_and_pi() {
        let observed = TranscriptPageSnapshot::from_page(
            "version-1",
            Some("lineage-1".to_string()),
            host_page(vec![
                message(14, "m14", "fourteen"),
                message(12, "m12", "twelve"),
                message(11, "m11", "eleven"),
                message(10, "m10", "ten"),
            ]),
        )
        .unwrap();
        let NewestPageFit::Trimmed(trimmed) =
            observed.fit_newest(OlderPageCursor::BelowOldestRetained, at_most_messages(2))
        else {
            panic!("a four-row page bounded to two rows is trimmed");
        };
        assert!(trimmed.is_trimmed());
        assert!(!observed.is_trimmed());
        assert_eq!(trimmed.version, "version-1");
        assert_eq!(trimmed.lineage_id(), Some("lineage-1"));
        let page = trimmed.resync_page();
        assert_eq!(messages_of(&page), ["m12", "m14"]);
        assert_eq!(page["nextBeforeOrdinal"], json!(12));
        assert!(page.get("olderUnavailable").is_none(), "{page}");
        assert!(page.get("emptyReason").is_none(), "{page}");
        assert_eq!(page["reportsCompletion"], json!(true));
        assert_eq!(page["generation"], json!(4));
        assert_eq!(page["messages"][0]["ordinal"], json!(12));
    }

    #[test]
    fn trimmed_newest_page_points_at_the_oldest_trimmed_out_row_for_broca() {
        let observed = snapshot(
            "lineage-b",
            7,
            vec![
                message(20, "m20", "twenty"),
                message(21, "m21", "twenty-one"),
                message(22, "m22", "twenty-two"),
                message(23, "m23", "twenty-three"),
            ],
        );
        let NewestPageFit::Trimmed(trimmed) =
            observed.fit_newest(OlderPageCursor::FromOldestTrimmed, at_most_messages(1))
        else {
            panic!("a four-row page bounded to one row is trimmed");
        };
        let page = trimmed.resync_page();
        assert_eq!(messages_of(&page), ["m23"]);
        // Broca's beforeOrdinal is the inclusive start of a forward read: the first
        // trimmed-out row, never the oldest retained one.
        assert_eq!(page["nextBeforeOrdinal"], json!(20));
    }

    #[test]
    fn fit_newest_reports_whole_pages_and_a_newest_message_that_alone_is_too_large() {
        let observed = snapshot(
            "version-1",
            1,
            vec![message(1, "m1", "one"), message(2, "m2", "two")],
        );
        assert_eq!(
            observed.fit_newest(OlderPageCursor::BelowOldestRetained, at_most_messages(2)),
            NewestPageFit::Whole
        );
        assert_eq!(
            observed.fit_newest(OlderPageCursor::BelowOldestRetained, at_most_messages(0)),
            NewestPageFit::NewestTooLarge
        );
        let single = snapshot("version-1", 1, vec![message(1, "m1", "one")]);
        assert_eq!(
            single.fit_newest(OlderPageCursor::FromOldestTrimmed, |_| false),
            NewestPageFit::NewestTooLarge
        );
    }

    #[test]
    fn fit_newest_keeps_the_most_rows_whose_serialized_page_fits_a_byte_bound() {
        let rows = (0..40)
            .map(|index| message(index, &format!("m{index}"), &"x".repeat(500)))
            .collect::<Vec<_>>();
        let observed = snapshot("version-1", 1, rows);
        let bound = 8_000;
        let size = |page: &Value| serde_json::to_vec(page).unwrap().len();
        let NewestPageFit::Trimmed(trimmed) = observed
            .fit_newest(OlderPageCursor::BelowOldestRetained, |page| {
                size(page) <= bound
            })
        else {
            panic!("a 40-row page over the bound is trimmed");
        };
        let kept = trimmed.rows.len();
        assert!(size(&trimmed.resync_page()) <= bound);
        assert!(kept > 1, "several rows fit the bound");
        let one_more = observed.trimmed_to_newest(kept + 1, OlderPageCursor::BelowOldestRetained);
        assert!(
            size(&one_more.resync_page()) > bound,
            "one more row must not fit"
        );
        assert_eq!(
            trimmed.rows.last().unwrap().message_id,
            "m39",
            "trimming drops rows from the oldest end"
        );
    }

    /// A trimmed snapshot recorded beside the full page at the same version serves only
    /// claimants whose floor it holds; a floor below it falls back to the full page.
    #[test]
    fn held_snapshot_prefers_one_holding_the_floor_and_otherwise_only_untrimmed_pages() {
        let session = "ses_trim";
        let full = snapshot(
            "version-v",
            1,
            vec![
                message(1, "B", "bee"),
                message(2, "C", "see"),
                message(3, "D", "dee"),
            ],
        );
        let NewestPageFit::Trimmed(trimmed) =
            full.fit_newest(OlderPageCursor::BelowOldestRetained, at_most_messages(2))
        else {
            panic!("three rows bounded to two are trimmed");
        };
        assert_eq!(messages_of(&trimmed.resync_page()), ["C", "D"]);
        let mut state = TranscriptClaimState::default();
        state.record(session, full.clone());
        state.record(session, trimmed.clone());
        let current = snapshot(
            "version-w",
            1,
            vec![
                message(1, "B", "bee"),
                message(2, "C", "see"),
                message(3, "D", "dee"),
                message(4, "E", "ee"),
            ],
        );

        // Floor A lies below both snapshots: the untrimmed page answers, vouching from B.
        let below = claim("A", "D", "version-v");
        assert_eq!(state.held_snapshot(session, &below), Some(full.clone()));
        assert_eq!(
            evaluate_claim(
                &below,
                state.held_snapshot(session, &below).as_ref(),
                &current
            ),
            ClaimEvaluation::Matched {
                new_turns: vec![message(4, "E", "ee")],
                vouched_from_message_id: "B".to_string(),
            }
        );
        // The trimmed snapshot would have reported a false divergence for that claimant.
        assert!(matches!(
            evaluate_claim(&below, Some(&trimmed), &current),
            ClaimEvaluation::Diverged { .. }
        ));

        // A claimant holding the trimmed page gets the newest snapshot holding its floor.
        let trimmed_claimant = claim("C", "D", "version-v");
        assert_eq!(
            state.held_snapshot(session, &trimmed_claimant),
            Some(trimmed.clone())
        );
        assert_eq!(
            evaluate_claim(
                &trimmed_claimant,
                state.held_snapshot(session, &trimmed_claimant).as_ref(),
                &current
            ),
            ClaimEvaluation::Matched {
                new_turns: vec![message(4, "E", "ee")],
                vouched_from_message_id: "C".to_string(),
            }
        );
        // A full-page claimant at the same version is still answered from the full page.
        let full_claimant = claim("B", "D", "version-v");
        assert_eq!(state.held_snapshot(session, &full_claimant), Some(full));

        // With only the trimmed snapshot retained, a floor outside it finds nothing.
        let mut trimmed_only = TranscriptClaimState::default();
        trimmed_only.record(session, trimmed.clone());
        assert_eq!(trimmed_only.held_snapshot(session, &below), None);
        assert_eq!(trimmed_only.snapshot(session, "version-v", Some("D")), None);
        assert_eq!(
            trimmed_only.held_snapshot(session, &trimmed_claimant),
            Some(trimmed)
        );
    }

    #[test]
    fn recording_an_untrimmed_page_equal_to_a_retained_trimmed_one_keeps_both() {
        let session = "ses_trim";
        let rows = vec![message(2, "C", "see"), message(3, "D", "dee")];
        let trimmed = snapshot("version-v", 1, rows.clone()).with_trimmed(true);
        let untrimmed = snapshot("version-v", 1, rows);
        let mut state = TranscriptClaimState::default();
        state.record(session, trimmed.clone());
        state.record(session, untrimmed.clone());
        state.record(session, trimmed);
        assert_eq!(state.snapshots[session].len(), 2);
        assert_eq!(
            state.snapshot(session, "version-v", Some("D")),
            Some(untrimmed)
        );
    }

    /// Every matched verdict names the message it vouches from: the held floor when it is inside
    /// the window, the window floor when the held floor predates it, and sinceMessageId when the
    /// claim has no older held turn. That holds whether or not there are new turns.
    #[test]
    fn every_matched_claim_reply_carries_the_vouched_from_message_id() {
        let rows = vec![
            message(1, "m1", "one"),
            message(2, "m2", "two"),
            message(3, "m3", "three"),
        ];
        let held = snapshot("version-1", 1, rows.clone());
        let mut appended = rows.clone();
        appended.push(message(4, "m4", "four"));
        let grown = snapshot("version-2", 1, appended);
        let unchanged = snapshot("version-2", 1, rows);
        for (floor, since, current, expected_vouch, expected_new_turns) in [
            ("m1", "m3", &grown, "m1", 1),
            ("m1", "m3", &unchanged, "m1", 0),
            ("m2", "m3", &unchanged, "m2", 0),
            ("m3", "m3", &unchanged, "m3", 0),
            ("m-ancient", "m3", &grown, "m1", 1),
            ("m3", "m2", &grown, "m2", 2),
        ] {
            let reply = claim_reply(
                "version-2",
                evaluate_claim(&claim(floor, since, "version-1"), Some(&held), current),
            );
            assert_eq!(
                reply["claim"],
                json!("matched"),
                "{floor}..{since}: {reply}"
            );
            assert_matched_reply_vouches(&reply);
            assert_eq!(
                reply["vouchedFromMessageId"],
                json!(expected_vouch),
                "{floor}..{since}: {reply}"
            );
            assert_eq!(
                reply["newTurns"].as_array().unwrap().len(),
                expected_new_turns,
                "{floor}..{since}: {reply}"
            );
        }

        let runner = runner_claim_reply(
            "lineage-1",
            true,
            RunnerClaimEvaluation::Matched {
                new_turns: Vec::new(),
                vouched_from_message_id: "m0".to_string(),
            },
        );
        assert_matched_reply_vouches(&runner);
        assert_eq!(runner["vouchedFromMessageId"], json!("m0"));
    }

    /// A diverged reply keeps vouchedFromMessageId optional: it appears only when the held floor
    /// predates the server window.
    #[test]
    fn diverged_claim_reply_vouches_only_below_the_window() {
        let held = snapshot(
            "version-1",
            1,
            vec![message(1, "m1", "one"), message(2, "m2", "two")],
        );
        let edited = snapshot(
            "version-2",
            1,
            vec![message(1, "m1", "one"), message(2, "m2", "two (edited)")],
        );
        let in_window = claim_reply(
            "version-2",
            evaluate_claim(&claim("m1", "m2", "version-1"), Some(&held), &edited),
        );
        assert_eq!(in_window["claim"], json!("diverged"));
        assert!(
            in_window.get("vouchedFromMessageId").is_none(),
            "{in_window}"
        );
        let below_window = claim_reply(
            "version-2",
            evaluate_claim(&claim("m-ancient", "m2", "version-1"), Some(&held), &edited),
        );
        assert_eq!(below_window["claim"], json!("diverged"));
        assert_eq!(below_window["vouchedFromMessageId"], json!("m1"));
    }

    /// Decode rule clients apply to a matched reply: vouchedFromMessageId names the oldest message
    /// the server checked and vouches for, it must be a non-empty string, and its absence is a
    /// decode failure.
    fn assert_matched_reply_vouches(reply: &Value) {
        assert_eq!(reply["claim"], json!("matched"), "{reply}");
        assert!(
            reply
                .get("vouchedFromMessageId")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty()),
            "matched reply lacks vouchedFromMessageId: {reply}"
        );
    }

    /// Every matched reply in test-vectors/transcript-claim-v1.1 (served-reply captures) and in
    /// tests/fixtures/transcript-delta-vectors-v1.json carries vouchedFromMessageId, and every
    /// capture file still matches the digest listed in test-vectors/transcript-claim-v1.1/SHA256SUMS.
    #[test]
    fn every_matched_transcript_claim_vector_carries_vouched_from_message_id() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-vectors/transcript-claim-v1.1");
        let sums = std::fs::read_to_string(root.join("SHA256SUMS")).unwrap();
        let mut matched = Vec::new();
        let mut files = 0;
        for line in sums.lines() {
            let (digest, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(root.join(name)).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&bytes)),
                digest,
                "{name} digest"
            );
            files += 1;
            let reply: Value = serde_json::from_slice(&bytes).unwrap();
            if reply["claim"] == "matched" {
                assert_matched_reply_vouches(&reply);
                matched.push(name.to_string());
            }
        }
        let on_disk = std::fs::read_dir(&root)
            .unwrap()
            .filter(|entry| {
                entry.as_ref().unwrap().path().extension() == Some(std::ffi::OsStr::new("json"))
            })
            .count();
        assert_eq!(files, on_disk, "SHA256SUMS lists every vector file");
        assert_eq!(
            matched,
            [
                "broca-matched.json",
                "opencode-matched.json",
                "opencode-unchanged.json"
            ]
        );

        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/transcript-delta-vectors-v1.json"
        ))
        .unwrap();
        let delta_matched = fixture["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|vector| vector["reply"]["claim"] == "matched")
            .inspect(|vector| assert_matched_reply_vouches(&vector["reply"]))
            .count();
        assert_eq!(delta_matched, 8, "delta fixture matched-vector census");
    }

    fn assert_contract_shape(name: &str, actual: &Value, expected: &Value) {
        assert_eq!(
            actual.get("claim"),
            expected.get("claim"),
            "vector {name} discriminator"
        );
        assert_eq!(
            actual.pointer("/error/code"),
            expected.pointer("/error/code"),
            "vector {name} refusal code"
        );
        assert_eq!(
            actual.get("diagnostics"),
            expected.get("diagnostics"),
            "vector {name} diagnostics bytes"
        );
        for field in [
            "version",
            "newTurns",
            "resync",
            "validThroughMessageId",
            "vouchedFromMessageId",
        ] {
            assert_eq!(
                actual.get(field).is_some(),
                expected.get(field).is_some(),
                "vector {name} field {field} presence"
            );
        }
        for field in ["acceptsAttachments", "acceptedMimeTypes"] {
            if expected.get(field).is_some() {
                assert_eq!(
                    actual.get(field),
                    expected.get(field),
                    "vector {name} {field}"
                );
            }
        }
        if let Some(error) = expected.get("error") {
            assert_eq!(
                actual.get("error"),
                Some(error),
                "vector {name} error bytes"
            );
        }
    }

    #[test]
    fn transcript_delta_v1_19_fixture_digest_is_pinned_on_the_rust_end() {
        let digest = format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../tests/fixtures/transcript-delta-vectors-v1.json"
            ))
        );
        assert_eq!(
            digest,
            "5a60148190a216932367f0e55d273a51bb2295ddbfe1bad41789e91751d3d705"
        );
    }

    #[test]
    fn transcript_delta_fixture_executes_every_producer_request_shape() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/transcript-delta-vectors-v1.json"
        ))
        .unwrap();
        let vectors = fixture["vectors"].as_array().unwrap();
        assert_eq!(vectors.len(), 24, "vector census is contractual");
        let mut executed_requests = 0;
        for vector in vectors {
            let Some(request) = vector.get("request") else {
                continue;
            };
            let name = vector["name"].as_str().unwrap();
            let expected = &vector["reply"];
            executed_requests += 1;

            let actual = match name {
                "claim_with_activity_token_refuses" | "claim_with_before_ordinal_refuses" => {
                    parse_claim(request).unwrap_err().reply()
                }
                "claim_limit_zero_refuses" => error_reply(
                    "claim_limit_zero",
                    "a claim requires a usable page: limit must be at least 1",
                    None,
                ),
                "observation_unstable_is_typed_retryable" => observation_unstable_reply("scan"),
                // Recorded when the session's harness was OpenCode 1's.
                "asserted_harness_disagrees_refuses" => {
                    harness_mismatch_reply("broca", "ses_a1", "opencode")
                }
                "unknown_session_refuses_typed" => error_reply(
                    "session_unknown",
                    "no session 'ses_never_existed' is known to this daemon",
                    None,
                ),
                "no_identity_arm_serves_no_claim_path" => identity_unsupported_reply(),
                "attachment_capability_unknown_is_explicit" => {
                    let mut reply = page(Vec::new());
                    // This fixture represents unknown attachment support with the string
                    // "unknown". Keep that spelling here, not in live page decoration.
                    reply["acceptsAttachments"] = json!("unknown");
                    reply["acceptedMimeTypes"] = json!([]);
                    reply
                }
                // The reply builder includes diagnostics. Remove it here to model a reply
                // whose producer omitted its identity and delta-read support.
                "omitted_capability_fails_closed" => {
                    let mut reply = json!({
                        "claim": "matched",
                        "version": "current-version",
                        "newTurns": [],
                        "vouchedFromMessageId": "msg_f1",
                        "diagnostics": diagnostics("durable", "scan"),
                    });
                    reply.as_object_mut().unwrap().remove("diagnostics");
                    reply
                }
                "runner_arm_lineage_equal_claim_matched_native" => runner_claim_reply(
                    "current-version",
                    true,
                    RunnerClaimEvaluation::Matched {
                        new_turns: vec![message(3, "m-new", "new")],
                        vouched_from_message_id: request["claim"]["heldFloorMessageId"]
                            .as_str()
                            .unwrap()
                            .to_string(),
                    },
                ),
                "runner_arm_lineage_absent_claim_cursor_expired_non_native" => runner_claim_reply(
                    "current-version",
                    false,
                    RunnerClaimEvaluation::CursorExpired {
                        resync: page(vec![message(1, "m1", "one")]),
                    },
                ),
                "runner_arm_lineage_unequal_claim_cursor_expired_native" => runner_claim_reply(
                    "current-version",
                    true,
                    RunnerClaimEvaluation::CursorExpired {
                        resync: page(vec![message(1, "m1", "one")]),
                    },
                ),
                _ => {
                    let Some(Claim::Held(parsed)) = parse_claim(request).unwrap() else {
                        panic!("vector {name} holds messages");
                    };
                    let lineage_changed = name.contains("cursor_expired");
                    let diverged = name.contains("diverged");
                    let first_affected_is_floor = name == "diverged_first_affected_is_floor";
                    let below_window = name == "matched_below_window_floor_bounds_the_vouch";
                    let has_new_turns = name == "matched_with_new_turns";
                    let floor = parsed.held_floor_message_id.as_str();
                    let since = parsed.since_message_id.as_str();
                    let held_messages = if below_window {
                        vec![
                            message(1, "window-floor", "window"),
                            message(2, since, "tail"),
                        ]
                    } else {
                        vec![message(1, floor, "floor"), message(2, since, "tail")]
                    };
                    let held = snapshot(&parsed.held_version, 1, held_messages.clone());
                    let mut current_messages = held_messages;
                    if first_affected_is_floor {
                        current_messages[0] = message(1, floor, "floor edited");
                    } else if diverged {
                        current_messages[1] = message(2, since, "tail edited");
                    }
                    if has_new_turns {
                        current_messages.push(message(3, "m-new", "new"));
                    }
                    let current = snapshot(
                        "current-version",
                        if lineage_changed { 2 } else { 1 },
                        current_messages,
                    );
                    claim_reply(
                        "current-version",
                        evaluate_claim(&parsed, Some(&held), &current),
                    )
                }
            };
            assert_contract_shape(name, &actual, expected);
        }
        assert_eq!(executed_requests, 22, "every fixture request must execute");
    }
}
