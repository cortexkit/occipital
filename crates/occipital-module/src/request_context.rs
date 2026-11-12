//! What a session's model is given before the conversation, as the apps show it.
//!
//! `session.request_context {session, harness?}` answers the system text and the
//! tools of the plan in force on the session, from its runner's
//! `session.plan_view`. It is the request as the plan built it, not a copy of a
//! request sent to a provider: a login plugin or gateway may put other text in
//! front of it. Magic Context's memories, summaries, user profile and project
//! docs reach the model as the first messages of the conversation, which Magic
//! Context serves to the apps itself.
//!
//! The reply keeps the runner's text, tool schemas and byte offsets unchanged and
//! only spells the members the apps' way:
//!
//! - `generation`: the plan generation in force; `appliedAtMs`, when the runner
//!   recorded when it was applied.
//! - `newestStep {atMs, replyMessageId?}`: the newest request built under it,
//!   with the `messageId` of its reply on the chat's pages when known.
//! - `system {text, items?}`: each item `{provider, preset?, start, end}` names
//!   the UTF-8 byte range of one provider's part of `text`, `end` exclusive.
//! - `tools [{name, description, inputSchema, provider, tokens? {count, exact}}]`,
//!   in the order the model is offered them; `exact: false` is an estimate.
//! - `pending {generation, note?}`: a newer plan not yet in force, and what the
//!   model was told about it, when it was.
//! - `asPlanned: true`, always.
//!
//! Refusals the app must not retry keep their own code: `unsupported` (the runner
//! doesn't serve plan views), `no_plan` and `too_large`. The runner refuses
//! `too_large` for a plan above its 4 MiB frame; this module refuses the same for
//! a reply above the app reply limit, which is a little lower, rather than cut it.

use cortexkit_role_llm_runner::plan_view::PlanView;
use occipital_app_reader::source::ReadError;
use serde_json::{json, Map, Value};

use crate::app_reader::SourceResolver;

pub const OPERATION: &str = "session.request_context";

/// Answer `session.request_context` from the session's runner.
pub async fn answer(params: &Value, resolver: &dyn SourceResolver) -> Result<Value, ReadError> {
    let session = params["session"]
        .as_str()
        .filter(|session| !session.is_empty())
        .ok_or_else(|| {
            ReadError::new("invalid_params", "session is missing or malformed")
                .with_detail(json!({"field": "session"}))
        })?;
    let harness = occipital_app_reader::page::optional_transcript_harness(params)
        .map_err(|error| ReadError::new(error.code, error.message))?;
    let view = resolver.plan_view(session, harness).await?;
    let reply = app_view(&view);
    // The reply goes out as `{"result": reply}`, eleven bytes more.
    let bytes = serde_json::to_vec(&reply).map_or(usize::MAX, |body| body.len() + 11);
    if bytes > occipital_app_reader::page::MAX_REPLY_BODY_BYTES {
        return Err(ReadError::new(
            "too_large",
            format!(
                "the plan's system text and tools are {bytes} bytes, above the app reply limit"
            ),
        ));
    }
    Ok(reply)
}

/// The runner's plan view in the apps' spelling.
pub fn app_view(view: &PlanView) -> Value {
    let mut reply = Map::new();
    reply.insert("generation".into(), json!(view.generation));
    if let Some(at) = view.applied_at_ms {
        reply.insert("appliedAtMs".into(), json!(at));
    }
    if let Some(step) = &view.newest_step {
        let mut newest = Map::new();
        newest.insert("atMs".into(), json!(step.at_ms));
        if let Some(mid) = &step.reply_mid {
            newest.insert("replyMessageId".into(), json!(mid));
        }
        reply.insert("newestStep".into(), Value::Object(newest));
    }
    let mut system = Map::new();
    system.insert("text".into(), json!(view.system.text));
    if let Some(items) = &view.system.items {
        let items: Vec<Value> = items
            .iter()
            .map(|item| {
                let mut value = Map::new();
                value.insert("provider".into(), json!(item.provider));
                if let Some(preset) = &item.preset {
                    value.insert("preset".into(), json!(preset));
                }
                value.insert("start".into(), json!(item.start));
                value.insert("end".into(), json!(item.end));
                Value::Object(value)
            })
            .collect();
        system.insert("items".into(), Value::Array(items));
    }
    reply.insert("system".into(), Value::Object(system));
    let tools: Vec<Value> = view
        .tools
        .iter()
        .map(|tool| {
            let mut value = Map::new();
            value.insert("name".into(), json!(tool.name));
            value.insert("description".into(), json!(tool.description));
            value.insert("inputSchema".into(), tool.input_schema.clone());
            value.insert("provider".into(), json!(tool.provider));
            if let Some(tokens) = &tool.tokens {
                value.insert(
                    "tokens".into(),
                    json!({"count": tokens.count, "exact": tokens.exact}),
                );
            }
            Value::Object(value)
        })
        .collect();
    reply.insert("tools".into(), Value::Array(tools));
    if let Some(pending) = &view.pending {
        let mut value = Map::new();
        value.insert("generation".into(), json!(pending.generation));
        if let Some(note) = &pending.note {
            value.insert("note".into(), json!(note));
        }
        reply.insert("pending".into(), Value::Object(value));
    }
    reply.insert("asPlanned".into(), json!(true));
    Value::Object(reply)
}
