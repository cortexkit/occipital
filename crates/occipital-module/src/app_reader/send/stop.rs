//! The apps' Stop: `session.stop {session, harness?, runId, clientRequestId}` stops the
//! session's running turn. This module calls the runner's `run.cancel` as a sender the
//! session's plan grants `interrupt`, immediately, which a runner declaring
//! `run_cancel_by_interrupt` admits. It adds no authority: that sender could already end
//! the turn with an interrupting send; Stop ends it without starting another.
//!
//! The answer is `{state, runId}`: `stopped` once the run has ended cancelled, or
//! `not_active` when it is not running (it ended another way, or the runner doesn't know
//! it). Naming the run means a late press can never stop the next turn. A repeat with the
//! same `clientRequestId` gets the first answer.
use super::*;
use cortexkit_role_llm_runner::{capabilities, ops};

pub const OPERATION: &str = "session.stop";

/// Stop answers kept for repeats. An app retries a press within seconds, and a person
/// presses Stop a handful of times a day per session, so 512 entries hold every retry
/// still coming while keeping the map a few tens of kilobytes. Past that the map starts
/// again, which costs a late repeat only the runner round trip: the runner's answer for a
/// run already cancelled is still `stopped`.
const REMEMBERED_STOPS: usize = 512;

struct StopRequest {
    session: String,
    harness: Option<String>,
    run_id: String,
    client_request_id: String,
}

impl StopRequest {
    fn parse(params: &Value) -> Result<Self, SendRefusal> {
        let invalid = |field: &str| SendRefusal {
            code: "invalid_request",
            message: format!("session.stop needs a valid {field}"),
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
        let harness = match object.get("harness") {
            None | Some(Value::Null) => None,
            Some(_) => Some(text("harness").ok_or_else(|| invalid("harness"))?),
        };
        Ok(Self {
            session: text("session").ok_or_else(|| invalid("session"))?,
            harness,
            run_id: text("runId").ok_or_else(|| invalid("runId"))?,
            client_request_id: text("clientRequestId").ok_or_else(|| invalid("clientRequestId"))?,
        })
    }
}

impl Sender {
    pub async fn stop(&self, params: Value) -> Result<Value, SendRefusal> {
        let request = StopRequest::parse(&params)?;
        let key = (request.session.clone(), request.client_request_id.clone());
        if let Some(answer) = self.stops.guard().get(&key).cloned() {
            crate::app_reader::observe::answer(format!(
                "{} run={} (repeat)",
                answer["state"].as_str().unwrap_or_default(),
                request.run_id
            ));
            return Ok(answer);
        }
        let target = self
            .targets
            .open(&request.session, request.harness.as_deref())
            .await
            .map_err(|error| match error.code {
                // As sends do: `detail.runner` names the runner Entorhinal records, so the
                // app can route the session to whoever serves it.
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
        let describe =
            target
                .call(ops::ROLE_DESCRIBE, json!({}))
                .await
                .map_err(|error| match error {
                    RunnerError::Unreachable(message) => unreachable(message),
                    RunnerError::Refused {
                        code,
                        message,
                        detail,
                    } => refused(&code, &message, detail.as_ref()),
                })?;
        let stoppable = describe["capabilities"].as_array().is_some_and(|groups| {
            groups
                .iter()
                .any(|group| group == capabilities::RUN_CANCEL_BY_INTERRUPT)
        });
        if !stoppable {
            return Err(SendRefusal {
                code: codes::UNSUPPORTED,
                message: "this session's runner doesn't let a sender stop its run".into(),
                detail: Some(json!({"field": OPERATION})),
            });
        }
        let state = match target
            .call(
                ops::RUN_CANCEL,
                json!({"run_id": request.run_id, "at": "now"}),
            )
            .await
        {
            Ok(reply) if reply == "ack" => "stopped",
            // Not running: stopped only if it already ended cancelled, as after a first
            // stop whose answer this module no longer remembers.
            Ok(_) => match target
                .call(ops::RUN_STATUS, json!({"run_id": request.run_id}))
                .await
            {
                Ok(status) if status["state"] == "cancelled" => "stopped",
                _ => "not_active",
            },
            Err(RunnerError::Unreachable(message)) => return Err(unreachable(message)),
            Err(RunnerError::Refused {
                code,
                message,
                detail,
            }) => return Err(refused(&code, &message, detail.as_ref())),
        };
        crate::app_reader::observe::answer(format!("{state} run={}", request.run_id));
        let answer = json!({"state": state, "runId": request.run_id});
        let mut stops = self.stops.guard();
        if stops.len() >= REMEMBERED_STOPS {
            stops.clear();
        }
        stops.insert(key, answer.clone());
        Ok(answer)
    }
}
