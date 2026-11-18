//! `say` calls as the agent's messages to the person.
//!
//! An agent given prefrontal's `say` tool talks to the person through it: each
//! call's `text` is a message, drawn as a chat bubble rather than a tool card.
//! Whether a call named `say` is that tool isn't in the stored call (it names
//! no provider), only in the session's plan, which the module reads. So row
//! building marks each `say` call's tool part with its whole text under
//! [`SAY_TEXT`], before arguments are cut to the page's size, and the module
//! then settles every marked part with [`resolve`]: into a `message` part for
//! a chat whose plan gives the agent `say`, or back to a plain tool part.

use serde_json::{json, Map, Value};

/// The tool's name.
pub const TOOL: &str = "say";

/// A marked part's whole message text. [`resolve`] always removes it, so it
/// never reaches the apps.
pub const SAY_TEXT: &str = "sayText";

/// Prefrontal's message record for a marked call, attached before
/// [`resolve`]: the message it answers, the verified quote, media and how the
/// message alerted. [`resolve`] always removes it too.
pub const SAY_RECORD: &str = "sayRecord";

/// The members of a message the record decides when there is one, in place
/// of what the call's input wrote.
const RECORD_SETTINGS: [&str; 4] = ["level", "key", "done", "deliver"];

/// The alert levels `say` takes. Any other value is left out of the message.
const LEVELS: [&str; 4] = ["chat", "info", "attention", "urgent"];

/// The longest `key` a `say` call may give, in bytes. A call's `key` names a
/// standing condition the message reports (a later message with the same key
/// updates it); a longer one is left out of the message.
const KEY_BYTES: usize = 64;

/// Mark the tool part `part` of a call named `name` with `input`, when it's a
/// `say` with a string `text`.
pub fn mark(part: &mut Value, name: &str, input: &Value) {
    if name != TOOL {
        return;
    }
    if let Some(text) = input.get("text").and_then(Value::as_str) {
        part[SAY_TEXT] = json!(text);
        if let Some(fields) = settings(input) {
            part["saySettings"] = Value::Object(fields);
        }
    }
}

/// The call key and, for a call moved here from another runner, the runner
/// that minted it (`callCarrier`), of each marked part of one row: where
/// prefrontal filed each call's message record.
pub fn marked_calls(display: &Value) -> Vec<(String, Option<String>)> {
    display["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part.get(SAY_TEXT).is_some())
        .filter_map(|part| {
            let key = part.get("callKey")?.as_str()?.to_owned();
            let carrier = part
                .get("callCarrier")
                .and_then(Value::as_str)
                .map(str::to_owned);
            Some((key, carrier))
        })
        .collect()
}

/// Attach `record`, prefrontal's checked message record, to the marked part of
/// one row whose call key is `call_key`.
pub fn attach_record(display: &mut Value, call_key: &str, record: Value) {
    for part in display["parts"].as_array_mut().into_iter().flatten() {
        if part.get(SAY_TEXT).is_some()
            && part.get("callKey").and_then(Value::as_str) == Some(call_key)
        {
            part[SAY_RECORD] = record;
            return;
        }
    }
}

/// Whether any part of the rows' displays is marked.
pub fn any_marked<'a>(displays: impl IntoIterator<Item = &'a Value>) -> bool {
    displays.into_iter().any(|display| {
        display["parts"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| part.get(SAY_TEXT).is_some()))
    })
}

/// Settle every marked part of one row's `display`.
///
/// When `say_is_the_message_tool`, a marked call becomes a `message` part once
/// it's answered without an error: the person got a message. While it runs,
/// it's left out of the row, since the message isn't sent yet and its row
/// would only flash a tool card. A call that failed or got no answer stays a
/// tool card, so the full view shows what went wrong. Otherwise, or when the
/// chat's agent doesn't have prefrontal's `say`, every marked part is a plain
/// tool part again.
pub fn resolve(display: &mut Value, say_is_the_message_tool: bool) {
    let Some(parts) = display.get_mut("parts").and_then(Value::as_array_mut) else {
        return;
    };
    let mut sent = Vec::new();
    let mut running = Vec::new();
    for part in parts.iter_mut() {
        let Some(fields) = part.as_object_mut() else {
            continue;
        };
        let Some(Value::String(text)) = fields.remove(SAY_TEXT) else {
            continue;
        };
        let settings = fields.remove("saySettings");
        let record = fields.remove(SAY_RECORD);
        if !say_is_the_message_tool {
            continue;
        }
        let call_id = fields.get("callId").cloned();
        let answered = fields.contains_key("outputBytes") || fields.contains_key("content");
        let failed =
            fields.get("isError") == Some(&json!(true)) || fields.contains_key("unanswered");
        if failed {
            continue;
        }
        if !answered {
            running.push(call_id);
            continue;
        }
        let mut message = Map::new();
        message.insert("type".into(), json!("message"));
        for kept in ["partIndex", "partKey", "callId", "callKey"] {
            if let Some(value) = fields.get(kept) {
                message.insert(kept.into(), value.clone());
            }
        }
        message.insert("text".into(), json!(text));
        match record {
            Some(Value::Object(record)) => add_record(&mut message, &record),
            _ => {
                if let Some(Value::Object(settings)) = settings {
                    message.extend(settings);
                }
            }
        }
        *part = Value::Object(message);
        sent.push(call_id);
    }
    if !running.is_empty() {
        parts.retain(|part| {
            !(part["type"] == "tool"
                && part.get("tool").and_then(Value::as_str) == Some(TOOL)
                && running.contains(&part.get("callId").cloned()))
        });
    }
    // A message, sent or still being written, is no tool call of the row.
    let gone: Vec<Option<Value>> = sent.into_iter().chain(running).collect();
    if gone.is_empty() {
        return;
    }
    if let Some(calls) = display.get_mut("toolCalls").and_then(Value::as_array_mut) {
        calls.retain(|call| !gone.contains(&call.get("callId").cloned()));
        if calls.is_empty() {
            display
                .as_object_mut()
                .map(|fields| fields.remove("toolCalls"));
        }
    }
    if let Some(summaries) = display
        .get_mut("blockSummaries")
        .and_then(Value::as_array_mut)
    {
        for _ in 0..gone.len() {
            if let Some(at) = summaries.iter().position(|name| name == TOOL) {
                summaries.remove(at);
            }
        }
    }
}

/// Add what prefrontal's `record` says to `message`: the message it answers
/// (`replyTo`, with the quote prefrontal verified against the person's own
/// words), its media, and how it alerted, which the record decides instead of
/// the call's input since it says what prefrontal delivered.
fn add_record(message: &mut Map<String, Value>, record: &Map<String, Value>) {
    if let Some(id) = record.get("reply_to").and_then(Value::as_str) {
        let mut reply = json!({"messageId": id});
        if let Some(quote) = record.get("quote").and_then(Value::as_str) {
            reply["quote"] = json!(quote);
            reply["verified"] = json!(true);
        }
        message.insert("replyTo".into(), reply);
    }
    if let Some(media) = record
        .get("media")
        .filter(|media| media.as_array().is_some_and(|items| !items.is_empty()))
    {
        message.insert("media".into(), media.clone());
    }
    for setting in RECORD_SETTINGS {
        if let Some(value) = record.get(setting) {
            message.insert(setting.into(), value.clone());
        }
    }
}

/// `level`, `key` and `done` as the call wrote them, each left out when it
/// isn't what `say` takes.
fn settings(input: &Value) -> Option<Map<String, Value>> {
    let mut fields = Map::new();
    if let Some(level) = input
        .get("level")
        .and_then(Value::as_str)
        .filter(|level| LEVELS.contains(level))
    {
        fields.insert("level".into(), json!(level));
    }
    if let Some(key) = input
        .get("key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty() && key.len() <= KEY_BYTES)
    {
        fields.insert("key".into(), json!(key));
        if let Some(done) = input.get("done").and_then(Value::as_bool) {
            fields.insert("done".into(), json!(done));
        }
    }
    (!fields.is_empty()).then_some(fields)
}

#[cfg(test)]
mod tests;
