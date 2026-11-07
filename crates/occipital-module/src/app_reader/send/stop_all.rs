//! The apps' Stop everything: `session.stop_all {session, harness?, runId?, clientRequestId}`
//! ends the session's running turn and asks every tool provider in its plan to stop or
//! pause what it runs for the session: shell commands, remote builds, workers, consults.
//! This module calls the runner's `session.stop_all` as a sender the session's plan
//! grants `interrupt`, which a runner declaring `stop_all` admits as it admits Stop. The
//! runner does the fan-out within one 15-second budget and keeps its first reply under
//! the request id, so the app's retry after a lost reply gets that reply again and never
//! stops work begun since.
//!
//! The answer is the runner's reply as it is: `{providers: [{provider, outcome, items?,
//! more?}], runner: {items}}`. Every provider in the plan is listed, `answered` with
//! what it stopped, or `no_answer`, `not_declared` or `unreachable`, so the app can say
//! what may still be running. Item members are passed on unread.
use super::*;
use cortexkit_role_llm_runner::{capabilities, ops, stop_all::StopAllReply};

pub const OPERATION: &str = "session.stop_all";

struct StopAllRequest {
    session: String,
    harness: Option<String>,
    run_id: Option<String>,
    client_request_id: String,
}

impl StopAllRequest {
    fn parse(params: &Value) -> Result<Self, SendRefusal> {
        let invalid = |field: &str| SendRefusal {
            code: "invalid_request",
            message: format!("session.stop_all needs a valid {field}"),
            detail: Some(json!({"field": field})),
        };
        let object = params.as_object().ok_or_else(|| invalid("params"))?;
        if let Some(field) = object.keys().find(|key| {
            !matches!(
                key.as_str(),
                "session" | "harness" | "runId" | "clientRequestId"
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
        let optional = |field: &str| match object.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(_) => text(field).map(Some).ok_or_else(|| invalid(field)),
        };
        // The id goes to the runner as its `request_id`, which takes 1 to 128
        // printable ASCII characters.
        let client_request_id = text("clientRequestId")
            .filter(|id| id.len() <= 128 && id.bytes().all(|byte| (0x20..=0x7e).contains(&byte)))
            .ok_or_else(|| invalid("clientRequestId"))?;
        Ok(Self {
            session: text("session").ok_or_else(|| invalid("session"))?,
            harness: optional("harness")?,
            run_id: optional("runId")?,
            client_request_id,
        })
    }
}

impl Sender {
    pub async fn stop_all(&self, params: Value) -> Result<Value, SendRefusal> {
        let request = StopAllRequest::parse(&params)?;
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
        let serves = describe["capabilities"]
            .as_array()
            .is_some_and(|groups| groups.iter().any(|group| group == capabilities::STOP_ALL));
        if !serves {
            return Err(SendRefusal {
                code: codes::UNSUPPORTED,
                message: "this session's runner doesn't stop its tools' work on request".into(),
                detail: Some(json!({"field": OPERATION})),
            });
        }
        let mut body = json!({"request_id": request.client_request_id});
        if let Some(run_id) = &request.run_id {
            body["run_id"] = json!(run_id);
        }
        let reply = target
            .call(ops::SESSION_STOP_ALL, body)
            .await
            .map_err(runner_refusal)?;
        // Checked against the contract's shape, then passed on unchanged, so a provider's
        // item members and outcomes this module doesn't know still reach the app.
        let checked = serde_json::from_value::<StopAllReply>(reply.clone()).map_err(|error| {
            unreachable(format!(
                "the runner's session.stop_all reply is malformed: {error}"
            ))
        })?;
        let outcomes: Vec<String> = checked
            .providers
            .iter()
            .map(|provider| format!("{}={}", provider.provider, provider.outcome))
            .collect();
        crate::app_reader::observe::answer(outcomes.join(" "));
        Ok(reply)
    }
}
