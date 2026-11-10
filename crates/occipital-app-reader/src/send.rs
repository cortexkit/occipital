//! The apps' send request: `session.enqueue_user_message`, ported from prefrontal-core.
//!
//! Ported from prefrontal commit `6b36afc1456483b0f94ea87769526ac92276fdc6`:
//! `crates/prefrontal-core-module/src/enqueue_gate.rs` (`validate_client_message_id`,
//! `validate_enqueue_send_id`, the `parts` shape check) and
//! `crates/prefrontal-core-module/src/task_lifecycle/runtime.rs`
//! (`enqueue_user_message_inner`: the order of the checks, their codes and messages,
//! and the content the duplicate check compares). The request keeps core's shape,
//! `{session, text, clientMessageId?, send_id?, parts?}`, plus the optional `mode`
//! the apps now name. Unknown top-level fields are ignored, as core ignores them.
//!
//! Who may send is decided by the route before this parser runs; the caller fields
//! core's route handler stamped (`callerPairedPhone` and the like) are not read here.

use serde_json::{json, Value};

pub const OPERATION: &str = "session.enqueue_user_message";

/// Maximum number of ASCII characters in a client-chosen `send_id` (core's
/// `ENQUEUE_SEND_ID_MAX_CHARS`).
pub const ENQUEUE_SEND_ID_MAX_CHARS: usize = 128;

/// A named refusal for a malformed send. Nothing is resolved or sent for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRefusal {
    pub code: &'static str,
    pub message: String,
    pub detail: Option<Value>,
}

impl SendRefusal {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }
}

/// How the app wants its message delivered. Core had no modes: the host decided,
/// and the apps only queued. An absent mode therefore means `queue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The next turn after the running one, or a new turn on an idle session.
    Queue,
    /// Into the running turn at its next step, or a new turn on an idle session.
    Steer,
    /// Cancel the running turn, then start this message.
    Interrupt,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Steer => "steer",
            Self::Interrupt => "interrupt",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queue" => Some(Self::Queue),
            "steer" => Some(Self::Steer),
            "interrupt" => Some(Self::Interrupt),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SendRequest {
    pub session: String,
    /// The session's harness when it isn't a Broca session (`opencode`), as reads take
    /// it: such a session is routed by the runner Entorhinal records for it.
    pub harness: Option<String>,
    pub text: String,
    pub client_message_id: Option<String>,
    pub send_id: Option<String>,
    /// `None` when the request names no mode.
    pub mode: Option<Mode>,
    /// The `parts` value exactly as sent, or `[]` when absent: the duplicate check
    /// compares it byte for byte, as core's digest does.
    parts: Value,
    /// The `attachment_id` each part names, in order.
    attachments: Vec<String>,
}

impl SendRequest {
    /// Parse in core's order, so the same malformed request gets the same refusal.
    pub fn parse(params: &Value) -> Result<Self, SendRefusal> {
        validate_client_message_id(params.get("clientMessageId"))?;
        let session = required_str(params, "session")?;
        let harness = params
            .get("harness")
            .and_then(Value::as_str)
            .filter(|harness| !harness.is_empty())
            .map(str::to_owned);
        let text = required_str(params, "text")?;
        let client_message_id = validate_client_message_id(params.get("clientMessageId"))?;
        let send_id = validate_enqueue_send_id(params.get("send_id"))?;
        if session.trim().is_empty() {
            return Err(SendRefusal::invalid("session must not be empty"));
        }
        let mode = match params.get("mode") {
            None => None,
            Some(Value::String(mode)) => Some(Mode::parse(mode).ok_or_else(|| {
                SendRefusal::invalid(format!(
                    "mode '{mode}' is unknown; expected queue, steer or interrupt"
                ))
            })?),
            Some(_) => {
                return Err(SendRefusal::invalid(
                    "mode must be a string when present: queue, steer or interrupt",
                ))
            }
        };
        let parts = params.get("parts").cloned().unwrap_or_else(|| json!([]));
        let attachments = attachment_ids(&parts)?;
        if text.trim().is_empty() && attachments.is_empty() {
            return Err(SendRefusal::invalid(
                "message must include non-empty text or at least one file part",
            ));
        }
        Ok(Self {
            session,
            harness,
            text,
            client_message_id,
            send_id,
            mode,
            parts,
            attachments,
        })
    }

    /// The uploads the message names, in its order: each an `attachment_id` from
    /// an `upload.finish` that put the file in the prompt.
    pub fn attachments(&self) -> &[String] {
        &self.attachments
    }

    /// The bytes a repeat must match: core hashes exactly this JSON,
    /// `{"text": text, "parts": parts}`, to tell a duplicate from a conflict.
    pub fn content(&self) -> Value {
        json!({"text": self.text, "parts": self.parts})
    }

    /// [`Self::content`] with each part's `attachment_id` replaced by the SHA-256
    /// of the file it names, `sha256[i]` for the i-th part, as core's digest does:
    /// a retry that uploaded the same bytes again, under a new upload id, is the
    /// same message. Filenames, media types and part order still count.
    pub fn content_by_bytes(&self, sha256: &[String]) -> Value {
        let mut parts = self.parts.clone();
        if let Value::Array(parts) = &mut parts {
            for (part, sha256) in parts.iter_mut().zip(sha256) {
                part["attachment_id"] = json!(sha256);
            }
        }
        json!({"text": self.text, "parts": parts})
    }
}

/// The `attachment_id` of each file part, in order. Absent, `null` and `[]` carry
/// nothing. Each part is `{type: "file", attachment_id, mime?, filename?}`, the
/// shape core took; `mime` and `filename` are the app's own labels and are only
/// compared by the duplicate check, never trusted, since the upload records both.
fn attachment_ids(parts: &Value) -> Result<Vec<String>, SendRefusal> {
    let parts = match parts {
        Value::Null => return Ok(Vec::new()),
        Value::Array(parts) => parts,
        _ => return Err(SendRefusal::invalid("parts must be an array of file parts")),
    };
    let mut ids = Vec::with_capacity(parts.len());
    for part in parts {
        let id = (part["type"] == "file")
            .then(|| part["attachment_id"].as_str())
            .flatten()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                SendRefusal::invalid(
                    "each part must be {type: \"file\", attachment_id} naming a finished upload",
                )
            })?;
        if ids.iter().any(|seen| seen == id) {
            return Err(SendRefusal::invalid(format!(
                "attachment {id} is named twice"
            )));
        }
        ids.push(id.to_owned());
    }
    Ok(ids)
}

fn required_str(params: &Value, field: &str) -> Result<String, SendRefusal> {
    params
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| SendRefusal::invalid(format!("missing string param '{field}'")))
}

/// 1-64 ASCII letters, digits, `-` or `_`; scoped to the target session.
pub fn validate_client_message_id(value: Option<&Value>) -> Result<Option<String>, SendRefusal> {
    let invalid = |message: &str| SendRefusal::new("invalid_client_message_id", message);
    let Some(value) = value else { return Ok(None) };
    let id = value
        .as_str()
        .ok_or_else(|| invalid("clientMessageId must be a string"))?;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(invalid(
            "clientMessageId must contain 1-64 ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(Some(id.to_owned()))
}

/// The older retry identity: a compact ASCII token, so the runner key derived from
/// it stays unambiguous in logs and durable keys.
pub fn validate_enqueue_send_id(value: Option<&Value>) -> Result<Option<String>, SendRefusal> {
    let invalid = |message: String| SendRefusal::new("invalid_send_id", message);
    let Some(value) = value else {
        return Ok(None);
    };
    let send_id = value
        .as_str()
        .ok_or_else(|| invalid("send_id must be a string when present".into()))?;
    if send_id.is_empty() {
        return Err(invalid("send_id must not be empty".into()));
    }
    if send_id.chars().count() > ENQUEUE_SEND_ID_MAX_CHARS {
        return Err(invalid(format!(
            "send_id must be at most {ENQUEUE_SEND_ID_MAX_CHARS} characters"
        )));
    }
    if !send_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(invalid(
            "send_id may contain only ASCII letters, digits, '.', '_', and '-'".into(),
        ));
    }
    Ok(Some(send_id.to_string()))
}

/// Receipt codes the apps decode. A receipt is read by its code, never its text.
pub mod codes {
    /// Never left the computer (or a resend is provably harmless): safe to resend.
    pub const NOT_SENT: &str = "not_sent";
    /// This device may not send to this session.
    pub const NOT_AUTHORIZED: &str = "enqueue_not_authorized";
    /// The same `clientMessageId` was used for different text, parts or mode.
    pub const CONFLICT: &str = "client_message_id_conflict";
    /// The worker has no run in progress, so a steer has nothing to join.
    pub const TASK_PAUSED: &str = "task_paused";
    /// No session by that name is known.
    pub const NOT_FOUND: &str = "not_found";
    /// The send asked for something this path cannot carry (an attachment, a mode).
    pub const UNSUPPORTED: &str = "unsupported";
    /// A worker steer the run ended before taking: it expired and was never written.
    pub const STEER_EXPIRED: &str = "steer_expired";
}

#[cfg(test)]
mod tests;
