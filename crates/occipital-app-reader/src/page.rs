//! Typed transcript-page request validation, independent of session resolution and transport.

pub use crate::claim::ClaimRefusal as PageRefusal;
use crate::claim::{Claim, TranscriptClaim};
use serde_json::Value;

/// Leave space for framing below the transport's four MiB ceiling.
pub const MAX_REPLY_BODY_BYTES: usize = 4_194_304 - 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub session_id: String,
    pub requested_harness: Option<String>,
    pub message_ordinal: Option<i64>,
    pub activity_token: Option<String>,
    pub before_ordinal: Option<i64>,
    pub before_cursor: Option<String>,
    pub limit: i64,
    pub claim: Option<TranscriptClaim>,
    /// The version of an empty held page: a claim sent with `heldVersion` alone
    /// ([`Claim::Empty`]). Never set together with `claim`.
    pub empty_claim: Option<String>,
    pub accepts_outside_window: bool,
    /// A page around this message: it and up to `limit` numbered messages before and
    /// after it.
    pub around_message_id: Option<String>,
    /// A page of up to `limit` numbered messages after this ordinal, oldest first.
    pub after_ordinal: Option<i64>,
}

impl PageRequest {
    pub fn parse(params: &Value) -> Result<Self, PageRefusal> {
        let (claim, empty_claim) = match crate::claim::parse_claim(params)? {
            Some(Claim::Held(claim)) => (Some(claim), None),
            Some(Claim::Empty { held_version }) => (None, Some(held_version)),
            None => (None, None),
        };
        let claimed = claim.is_some() || empty_claim.is_some();
        let accepts_outside_window = crate::claim::accepts_outside_window(params)?;
        let requested_harness = optional_transcript_harness(params)?;
        let session_id = required_session(params)?;
        if session_id.trim().is_empty() {
            return Err(invalid("session must not be empty"));
        }
        let message_ordinal = params
            .get("messageOrdinal")
            .map(|value| {
                value
                    .as_i64()
                    .filter(|ordinal| *ordinal >= 0 && *ordinal < i64::MAX)
                    .ok_or_else(|| invalid("messageOrdinal must be an integer >= 0 and < i64::MAX"))
            })
            .transpose()?;
        let activity_token = match params.get("activityToken") {
            None => None,
            Some(Value::String(token)) => Some(token.as_str()),
            Some(_) => {
                return Err(invalid(
                    "activityToken must be an opaque string when present",
                ));
            }
        };
        let requested_before_ordinal = match params.get("beforeOrdinal") {
            None => None,
            Some(Value::Null) => {
                return Err(invalid(
                    "beforeOrdinal must be absent or a non-negative integer, not null",
                ));
            }
            Some(value) => Some(
                value
                    .as_i64()
                    .filter(|ordinal| *ordinal >= 0)
                    .ok_or_else(|| invalid("beforeOrdinal must be a non-negative integer"))?,
            ),
        };
        let before_cursor = match params.get("beforeCursor") {
            None => None,
            Some(Value::String(cursor)) if !cursor.is_empty() => Some(cursor.as_str()),
            Some(_) => {
                return Err(invalid(
                    "beforeCursor must be a non-empty opaque string when present",
                ))
            }
        };
        if requested_before_ordinal.is_some() && before_cursor.is_some() {
            return Err(invalid(
                "beforeOrdinal cannot be combined with beforeCursor",
            ));
        }
        if message_ordinal.is_some()
            && (requested_before_ordinal.is_some() || before_cursor.is_some() || claimed)
        {
            return Err(invalid(
                "messageOrdinal cannot be combined with beforeOrdinal, beforeCursor or claim",
            ));
        }
        let around_message_id = match params.get("aroundMessageId") {
            None => None,
            Some(Value::String(mid)) if !mid.is_empty() => Some(mid.clone()),
            Some(_) => return Err(invalid("aroundMessageId must be a non-empty string")),
        };
        let after_ordinal = match params.get("afterOrdinal") {
            None => None,
            Some(value) => Some(
                value
                    .as_i64()
                    .filter(|ordinal| *ordinal >= 0 && *ordinal < i64::MAX)
                    .ok_or_else(|| {
                        invalid("afterOrdinal must be an integer >= 0 and < i64::MAX")
                    })?,
            ),
        };
        // Each position form names one page: a read around a message or after an ordinal
        // takes no other position, no claim and no activity token.
        let others = message_ordinal.is_some()
            || requested_before_ordinal.is_some()
            || before_cursor.is_some()
            || claimed
            || activity_token.is_some();
        if around_message_id.is_some() && (others || after_ordinal.is_some()) {
            return Err(invalid(
                "aroundMessageId cannot be combined with messageOrdinal, beforeOrdinal, beforeCursor, afterOrdinal, claim or activityToken",
            ));
        }
        if after_ordinal.is_some() && others {
            return Err(invalid(
                "afterOrdinal cannot be combined with messageOrdinal, beforeOrdinal, beforeCursor, claim or activityToken",
            ));
        }
        let before_ordinal =
            transcript_page_before_ordinal(message_ordinal, requested_before_ordinal);
        let limit = if message_ordinal.is_some() {
            1
        } else {
            params
                .get("limit")
                .map(|value| {
                    value
                        .as_i64()
                        .filter(|limit| *limit >= 0)
                        .ok_or_else(|| invalid("limit must be a non-negative integer"))
                })
                .transpose()?
                .unwrap_or(100)
                .min(100)
        };
        if claimed && limit == 0 {
            return Err(PageRefusal {
                code: "claim_limit_zero",
                message: "a claim requires a usable page: limit must be at least 1".to_string(),
            });
        }

        Ok(Self {
            session_id,
            requested_harness: requested_harness.map(str::to_string),
            message_ordinal,
            activity_token: activity_token.map(str::to_string),
            before_ordinal,
            before_cursor: before_cursor.map(str::to_string),
            limit,
            claim,
            empty_claim,
            accepts_outside_window,
            around_message_id,
            after_ordinal,
        })
    }
}

fn invalid(message: impl Into<String>) -> PageRefusal {
    PageRefusal {
        code: "invalid_request",
        message: message.into(),
    }
}

fn required_session(params: &Value) -> Result<String, PageRefusal> {
    params
        .get("session")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid("missing string param 'session'"))
}

pub fn optional_transcript_harness(params: &Value) -> Result<Option<&str>, PageRefusal> {
    match params.get("harness") {
        None => Ok(None),
        Some(Value::String(harness))
            if matches!(harness.as_str(), "opencode2" | "broca" | "pi") =>
        {
            Ok(Some(harness))
        }
        Some(Value::String(harness)) => Err(invalid(format!(
            "transcript harness '{harness}' is not served here; the served harnesses are 'opencode2', 'broca' and 'pi'"
        ))),
        Some(_) => Err(invalid("transcript harness must be a string when present")),
    }
}

pub fn assert_transcript_harness(
    requested_harness: Option<&str>,
    actual_harness: &str,
) -> Result<(), PageRefusal> {
    if let Some(requested_harness) = requested_harness {
        if requested_harness != actual_harness {
            return Err(invalid(format!(
                "transcript harness mismatch: you said {requested_harness}, the session is {actual_harness}"
            )));
        }
    }
    Ok(())
}

/// Convert an exact message ordinal into the exclusive cursor used by transcript pages.
pub fn transcript_page_before_ordinal(
    message_ordinal: Option<i64>,
    before_ordinal: Option<i64>,
) -> Option<i64> {
    message_ordinal
        .map(|ordinal| ordinal + 1)
        .or(before_ordinal)
}

/// Check the serialized reply body, not a row count or an estimate of its JSON size.
pub fn check_reply_size(bytes: usize) -> Result<(), PageRefusal> {
    if bytes > MAX_REPLY_BODY_BYTES {
        return Err(PageRefusal {
            code: "reply_too_large",
            message: format!("session.transcript_page ran, but its reply was {bytes} bytes, above the 4 MiB transport limit, so the result was not sent; check its effect before retrying, and narrow the request (fewer paths, a smaller page or a lower max_bytes)"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claim_request() -> Value {
        json!({"session":"s", "claim":{
            "heldFloorMessageId":"floor", "sinceMessageId":"tail", "heldVersion":"v"
        }})
    }

    #[test]
    fn page_request_defaults_limits_and_exact_ordinals() {
        let defaults = PageRequest::parse(&json!({"session":"s", "mode":"ignored"})).unwrap();
        assert_eq!(defaults.limit, 100);
        assert_eq!(defaults.before_ordinal, None);
        assert_eq!(defaults.before_cursor, None);
        assert_eq!(
            PageRequest::parse(&json!({"session":"s", "limit":1000}))
                .unwrap()
                .limit,
            100
        );
        assert_eq!(
            PageRequest::parse(&json!({"session":"s", "limit":0}))
                .unwrap()
                .limit,
            0
        );
        let exact =
            PageRequest::parse(&json!({"session":"s", "messageOrdinal":10, "limit":"ignored"}))
                .unwrap();
        assert_eq!(exact.limit, 1);
        assert_eq!(exact.before_ordinal, Some(11));
        assert_eq!(transcript_page_before_ordinal(None, Some(11)), Some(11));
        assert_eq!(
            PageRequest::parse(&json!({"session":"s", "messageOrdinal":i64::MAX-1}))
                .unwrap()
                .before_ordinal,
            Some(i64::MAX)
        );
    }

    #[test]
    fn page_request_rejects_invalid_types_and_ordinal_ranges() {
        for request in [
            json!(null),
            json!({}),
            json!({"session":null}),
            json!({"session":"  "}),
        ] {
            assert_eq!(
                PageRequest::parse(&request).unwrap_err().code,
                "invalid_request"
            );
        }
        for key in ["messageOrdinal", "beforeOrdinal", "limit"] {
            for value in [
                json!(null),
                json!(-1),
                json!(1.5),
                json!("1"),
                json!(true),
                json!(u64::MAX),
            ] {
                let mut request = json!({"session":"s"});
                request[key] = value;
                assert_eq!(
                    PageRequest::parse(&request).unwrap_err().code,
                    "invalid_request",
                    "{request}"
                );
            }
        }
        assert!(PageRequest::parse(&json!({"session":"s", "messageOrdinal":i64::MAX})).is_err());
        assert!(PageRequest::parse(&json!({"session":"s", "beforeOrdinal":i64::MAX})).is_ok());
        assert!(PageRequest::parse(&json!({"session":"s", "activityToken":null})).is_err());
        assert_eq!(
            PageRequest::parse(&json!({"session":"s", "activityToken":""}))
                .unwrap()
                .activity_token
                .as_deref(),
            Some("")
        );
    }

    #[test]
    fn page_history_cursors_are_opaque_and_exclusive() {
        let request =
            PageRequest::parse(&json!({"session":"s", "beforeCursor":"mid:opaque"})).unwrap();
        assert_eq!(request.before_cursor.as_deref(), Some("mid:opaque"));
        assert_eq!(request.before_ordinal, None);
        for request in [
            json!({"session":"s", "beforeCursor":"mid", "beforeOrdinal":0}),
            json!({"session":"s", "beforeCursor":"mid", "messageOrdinal":0}),
            json!({"session":"s", "beforeOrdinal":0, "messageOrdinal":0}),
            json!({"session":"s", "beforeCursor":""}),
            json!({"session":"s", "beforeCursor":null}),
            json!({"session":"s", "beforeCursor":1}),
        ] {
            assert_eq!(
                PageRequest::parse(&request).unwrap_err().code,
                "invalid_request",
                "{request}"
            );
        }
    }

    #[test]
    fn claims_refuse_zero_limits_and_exact_or_history_reads() {
        let mut request = claim_request();
        request["limit"] = json!(0);
        assert_eq!(
            PageRequest::parse(&request).unwrap_err().code,
            "claim_limit_zero"
        );
        request.as_object_mut().unwrap().remove("limit");
        request["messageOrdinal"] = json!(0);
        assert_eq!(
            PageRequest::parse(&request).unwrap_err().code,
            "invalid_request"
        );
        request.as_object_mut().unwrap().remove("messageOrdinal");
        request["beforeCursor"] = json!("opaque");
        assert_eq!(
            PageRequest::parse(&request).unwrap_err().code,
            "claim_conflicts_with_history_cursor"
        );
    }

    #[test]
    fn transcript_harness_is_optional_and_assertions_must_match() {
        assert_eq!(optional_transcript_harness(&json!({})).unwrap(), None);
        for harness in ["opencode2", "pi", "broca"] {
            let params = json!({"harness":harness});
            assert_eq!(optional_transcript_harness(&params).unwrap(), Some(harness));
            assert!(assert_transcript_harness(Some(harness), harness).is_ok());
        }
        assert!(assert_transcript_harness(None, "broca").is_ok());
        let error = assert_transcript_harness(Some("pi"), "broca").unwrap_err();
        assert!(error.message.contains("pi") && error.message.contains("broca"));
        for harness in [json!(null), json!("claude"), json!(1)] {
            assert_eq!(
                optional_transcript_harness(&json!({"harness":harness}))
                    .unwrap_err()
                    .code,
                "invalid_request"
            );
        }
    }

    #[test]
    fn reply_size_guard_checks_the_exact_transport_boundary() {
        assert_eq!(MAX_REPLY_BODY_BYTES, 4_128_768);
        assert!(check_reply_size(MAX_REPLY_BODY_BYTES).is_ok());
        assert_eq!(
            check_reply_size(MAX_REPLY_BODY_BYTES + 1).unwrap_err().code,
            "reply_too_large"
        );
    }
}
