//! The apps' "Send now": `session.promote_send {session, harness?, clientMessageId,
//! clientRequestId}` moves a message the person sent, still waiting for the running
//! turn to end, into that turn as a steer.
//!
//! Only the runner can decide that safely: it holds the queue, and the turn may end
//! while the person presses. So this module asks the runner's `session.promote`
//! (runner contract §9.24) about the message's own send, by the `send_id` its receipt
//! recorded, and the runner moves it or says why not, in one step with its queue:
//! `promoted` (it joins the running turn at its next step), `delivered` (the turn
//! ended first and the message already started or joined one, so nothing runs
//! twice), `not_promoted` with a reason (still queued, but no run is active), or
//! `not_queued` (no such send, or it ended otherwise). The send keeps its `send_id`,
//! so its receipt and its `queuedSends` entry go on as before: the entry stays
//! `queued` until this module's poll of `session.send_status` sees it delivered.
//!
//! `clientRequestId` names the press. The runner keeps its first answer under it, so
//! the app's retry after a lost reply gets that answer again, while a later press
//! under a new id is decided afresh. The answer is the runner's reply as it is.
use super::*;
use cortexkit_role_llm_runner::{
    capabilities, ops,
    promote::{PromoteReply, PromoteRequest},
};

pub const OPERATION: &str = "session.promote_send";

struct PromoteSendRequest {
    session: String,
    harness: Option<String>,
    client_message_id: String,
    client_request_id: String,
}

impl PromoteSendRequest {
    fn parse(params: &Value) -> Result<Self, SendRefusal> {
        let invalid = |field: &str| SendRefusal {
            code: "invalid_request",
            message: format!("session.promote_send needs a valid {field}"),
            detail: Some(json!({"field": field})),
        };
        let object = params.as_object().ok_or_else(|| invalid("params"))?;
        if let Some(field) = object.keys().find(|key| {
            !matches!(
                key.as_str(),
                "session" | "harness" | "clientMessageId" | "clientRequestId"
            )
        }) {
            return Err(invalid(field));
        }
        let text = |field: &str| {
            object
                .get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let harness = match object.get("harness") {
            None | Some(Value::Null) => None,
            Some(_) => Some(text("harness").ok_or_else(|| invalid("harness"))?),
        };
        // The id goes to the runner as its `request_id`, which takes 1 to 128
        // printable ASCII characters.
        let client_request_id = text("clientRequestId")
            .filter(|id| id.len() <= 128 && id.bytes().all(|byte| (0x20..=0x7e).contains(&byte)))
            .ok_or_else(|| invalid("clientRequestId"))?;
        Ok(Self {
            session: text("session").ok_or_else(|| invalid("session"))?,
            harness,
            client_message_id: text("clientMessageId").ok_or_else(|| invalid("clientMessageId"))?,
            client_request_id,
        })
    }
}

impl Sender {
    pub async fn promote(&self, params: Value) -> Result<Value, SendRefusal> {
        let request = PromoteSendRequest::parse(&params)?;
        // The runner knows the message only by the send id it went out under,
        // which its receipt here recorded. A message this module never sent, or
        // one whose receipt has expired, can't be named to the runner.
        let Some(stored) = self.stored(&request.session, &request.client_message_id)? else {
            return Err(SendRefusal {
                code: codes::NOT_FOUND,
                message: "no message sent through this module has this clientMessageId".into(),
                detail: Some(json!({"field": "clientMessageId"})),
            });
        };
        let target = self
            .targets
            .open(&request.session, request.harness.as_deref())
            .await
            .map_err(|error| match error.code {
                // As Stop does: `detail.runner` names the runner Entorhinal records.
                "session_unknown" => SendRefusal {
                    code: codes::NOT_FOUND,
                    message: format!(
                        "no runner this module sends to hosts session {}",
                        request.session
                    ),
                    detail: error.detail,
                },
                code => SendRefusal {
                    code,
                    message: error.message,
                    detail: error.detail,
                },
            })?;
        let unreachable = |message: String| SendRefusal::new("provider_unavailable", message);
        let runner_refusal = |error: RunnerError| match error {
            RunnerError::Unreachable(message) => unreachable(message),
            // The app used this press's id for another message.
            RunnerError::Refused { code, message, .. } if code == "request_id_reuse" => {
                SendRefusal {
                    code: "client_request_id_conflict",
                    message: format!("the runner answered {code}: {message}"),
                    detail: Some(json!({"field": "clientRequestId"})),
                }
            }
            // The runner declares the group, but this session's harness takes no
            // steer (Claude Code), so nothing can join its turn.
            RunnerError::Refused { code, message, .. } if code == "unsupported" => SendRefusal {
                code: codes::UNSUPPORTED,
                message: format!("the runner answered {code}: {message}"),
                detail: Some(json!({"field": OPERATION})),
            },
            RunnerError::Refused {
                code,
                message,
                detail,
            } => refused(&code, &message, detail.as_ref()),
        };
        let describe = target
            .call(ops::ROLE_DESCRIBE, json!({}))
            .await
            .map_err(runner_refusal)?;
        let serves = describe["capabilities"].as_array().is_some_and(|groups| {
            groups
                .iter()
                .any(|group| group == capabilities::SEND_PROMOTE)
        });
        if !serves {
            return Err(SendRefusal {
                code: codes::UNSUPPORTED,
                message: "this session's runner can't move a waiting message into its running turn"
                    .into(),
                detail: Some(json!({"field": OPERATION})),
            });
        }
        let body = serde_json::to_value(PromoteRequest::new(
            stored.send_id,
            request.client_request_id,
        ))
        .map_err(|error| {
            SendRefusal::new("internal", format!("cannot encode the promote: {error}"))
        })?;
        let reply = target
            .call(ops::SESSION_PROMOTE, body)
            .await
            .map_err(runner_refusal)?;
        // Checked against the contract's shape, then passed on unchanged, so an
        // outcome or reason this module doesn't know still reaches the app, which
        // never takes one for a promotion.
        let checked = serde_json::from_value::<PromoteReply>(reply.clone()).map_err(|error| {
            unreachable(format!(
                "the runner's session.promote reply is malformed: {error}"
            ))
        })?;
        let outcome = match checked {
            PromoteReply::Promoted => "promoted".to_owned(),
            PromoteReply::Delivered => "delivered".to_owned(),
            PromoteReply::NotPromoted { reason } => {
                format!("not_promoted reason={}", String::from(reason))
            }
            PromoteReply::NotQueued => "not_queued".to_owned(),
            PromoteReply::Unknown(outcome) => outcome,
            _ => "other".to_owned(),
        };
        crate::app_reader::observe::answer(format!("outcome={outcome}"));
        Ok(reply)
    }
}
