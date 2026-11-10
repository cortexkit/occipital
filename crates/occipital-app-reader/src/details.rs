//! `session.details`: what the session's runner says about the session now, for the
//! apps' session screen. One head read answers it; no transcript is read.
//!
//! The request is `{session, harness?}`, the same session selection a transcript
//! page uses. The reply is `{model?, usage?, incarnation?}` plus the send grant
//! (see [`crate::source::SendGrant`]). An absent `model`, `usage` or `incarnation`
//! means unknown, never zero or none. For the grant, `senderUnknown: true` means
//! unknown, and a reply with neither `sender` nor `senderUnknown` means the app may
//! not send.

use serde_json::{json, Value};

use crate::page::{self, PageRefusal};
use crate::source::{Head, ReadError, TranscriptSource};

pub const OPERATION: &str = "session.details";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailsRequest {
    pub session: String,
    pub harness: Option<String>,
}

impl DetailsRequest {
    /// Unknown members are ignored, as the transcript page ignores them.
    pub fn parse(params: &Value) -> Result<Self, PageRefusal> {
        let harness = page::optional_transcript_harness(params)?.map(str::to_string);
        let session = params
            .get("session")
            .and_then(Value::as_str)
            .filter(|session| !session.trim().is_empty())
            .ok_or_else(|| PageRefusal {
                code: "invalid_request",
                message: "session.details requires a non-empty string 'session'".into(),
            })?;
        Ok(Self {
            session: session.into(),
            harness,
        })
    }
}

/// Answer from exactly one `head()` call on `source`. `None` is a session the
/// resolver does not know.
pub async fn read(
    source: Option<&dyn TranscriptSource>,
    request: &DetailsRequest,
) -> Result<Value, ReadError> {
    read_with_head(source, request)
        .await
        .map(|(reply, _)| reply)
}

/// [`read`], with the head the reply was built from, for a caller that adds
/// facts of its own from the same head.
pub async fn read_with_head(
    source: Option<&dyn TranscriptSource>,
    request: &DetailsRequest,
) -> Result<(Value, Head), ReadError> {
    let Some(source) = source else {
        return Err(ReadError::new(
            "session_unknown",
            format!("no session '{}' is known to this daemon", request.session),
        ));
    };
    page::assert_transcript_harness(request.harness.as_deref(), source.harness())?;
    let head = source.head().await?;
    let mut reply = json!({});
    if let Some(usage) = &head.usage {
        reply["usage"] = usage.clone();
    }
    if let Some(incarnation) = &head.incarnation {
        reply["incarnation"] = json!(incarnation);
    }
    head.describe_session(&mut reply);
    Ok((reply, head))
}

#[cfg(test)]
mod tests;
