//! Temporary Broca `session.read` projection until `session.history` serves display rows.
//! The llm-runner contract's display form will replace this decoder; its current
//! Anthropic/OpenCode projections do not cover Broca entries.
//! Ported from prefrontal-core at a13318fba1581db2776e91f4954744f35dbbbef6
//! (`llm_runner_consumer::{mod,runtime,full_message}`), without runner dependencies.
//!
//! Every display part built from a stored content block carries that block's
//! identity, so a consumer such as Magic Context can mark parts on the phone's
//! transcript without relying on the display order:
//! - `partKey`: the runner's own key for the block in its `session.read` entries.
//!   For Broca (and the Claude Code gateway, which Broca serves) that is the
//!   block's index in the stored message's `content`, written as a decimal string
//!   so the member has one type across runners (OpenCode's keys are `prt_…` ids).
//! - `partIndex`: the block's index in the stored message's `content`, counting
//!   every stored block, including the ones no display part shows.
//! - `callId`: on tool parts and `toolCalls` entries, the stored block's
//!   `tool_call_id`.
//!
//! A member is left out when the stored message has no value for it: the
//! placeholder row for an unreadable entry has no stored block, so its part
//! carries neither key, and a tool block without a `tool_call_id` has no
//! `callId`.

use std::collections::{HashMap, VecDeque};

use cortexkit_role_llm_runner::archive_prunes_reasoning::{kinds, PrunedReasoning};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::source::{ReadError, Row};

const ARGUMENT_BYTES: usize = 2_048;

/// The runner's key for a stored content block.
pub const PART_KEY: &str = "partKey";
/// A stored content block's position in its message, counting from 0.
pub const PART_INDEX: &str = "partIndex";
/// The stored call id of a tool call, or of the call a tool result answers.
pub const CALL_ID: &str = "callId";

#[derive(Deserialize)]
struct Entry {
    mid: String,
    ordinal: Option<u64>,
    message: Message,
    #[serde(
        default,
        alias = "createdAtMs",
        alias = "created_at",
        alias = "createdAt",
        alias = "timestampMs",
        alias = "timestamp_ms"
    )]
    created_at_ms: Option<i64>,
    #[serde(default)]
    written_at: Option<i64>,
    #[serde(default, alias = "completedAtMs", alias = "completed_at")]
    completed_at_ms: Option<i64>,
    #[serde(default)]
    client_message_id: Option<String>,
    /// The run's rich UI stamp, which a runner declaring `rich_ui_stamp` adds to an
    /// assistant entry when the read asks for it. Kept raw so a malformed stamp drops
    /// only itself, never the page.
    #[serde(default)]
    rich_ui: Option<Value>,
    /// The calls' start and answer times, which a runner declaring
    /// `read_display_facts` adds when the read asks for them. Kept raw, as
    /// [`crate::tool_times`] reads it.
    #[serde(default)]
    tool_times: Option<Value>,
}

#[derive(Deserialize)]
struct Message {
    role: String,
    #[serde(default)]
    content: Vec<Value>,
    #[serde(
        default,
        alias = "createdAtMs",
        alias = "created_at",
        alias = "createdAt",
        alias = "timestampMs",
        alias = "timestamp_ms"
    )]
    created_at_ms: Option<i64>,
    #[serde(default, alias = "completedAtMs", alias = "completed_at")]
    completed_at_ms: Option<i64>,
    #[serde(default)]
    client_message_id: Option<String>,
}

enum Part {
    Text(String),
    Reasoning(String),
    Injected(String),
    Call { name: String, input: Value },
    Result { content: Value, is_error: bool },
    File { raw: Value },
    Unknown(Value),
}

/// A decoded content block and where it sits in the stored message. Some stored
/// blocks decode to nothing (encrypted reasoning, turn ends), so the position in
/// `Parsed::parts` is not the stored position.
struct Decoded {
    /// The block's index in the stored message's `content`.
    stored: usize,
    /// The stored `tool_call_id`, kept for tool calls and results only.
    call_id: Option<String>,
    part: Part,
}

struct Parsed {
    entry: Entry,
    parts: Vec<Decoded>,
}

/// Decode a complete observed range before selecting app pages, so a tool result
/// on the following source page can still decorate its issuing call.
pub fn rows(entries: &[Value], full: bool) -> Result<Vec<Row>, ReadError> {
    rows_with_pruned_reasoning(entries, full, &[])
}

/// Project stored entries with their page's pruning metadata. Only fully pruned
/// reasoning is hidden; readable summaries and signature-only changes stay visible.
pub fn rows_with_pruned_reasoning(
    entries: &[Value],
    full: bool,
    pruned_reasoning: &[PrunedReasoning],
) -> Result<Vec<Row>, ReadError> {
    let mut parsed = Vec::with_capacity(entries.len());
    let mut failures = HashMap::new();
    for (index, raw) in entries.iter().enumerate() {
        match serde_json::from_value::<Entry>(raw.clone()) {
            Ok(entry) if !entry.mid.is_empty() => {
                let mut parts = Vec::new();
                for (stored, value) in entry.message.content.iter().cloned().enumerate() {
                    // Match stored positions before filtering, so later parts retain their keys.
                    if entry.ordinal.is_some_and(|ordinal| {
                        pruned_reasoning.iter().any(|pruned| {
                            pruned.kind == kinds::REASONING
                                && pruned.mid == entry.mid
                                && pruned.part == format!("m{ordinal}#{stored}")
                        })
                    }) {
                        continue;
                    }
                    let call_id = tool_call_id(&value);
                    if let Some(part) = decode_part(value) {
                        let call_id = call_id
                            .filter(|_| matches!(part, Part::Call { .. } | Part::Result { .. }));
                        parts.push(Decoded {
                            stored,
                            call_id,
                            part,
                        });
                    }
                }
                parsed.push(Some(Parsed { entry, parts }));
            }
            _ => {
                // A broken payload must remain visible, but an identity cannot be invented.
                // Its placeholder part stands for no stored block, so it carries no
                // `partKey` or `partIndex`.
                let mid = raw["mid"]
                    .as_str()
                    .filter(|mid| !mid.is_empty())
                    .ok_or_else(|| {
                        ReadError::new("identity_unsupported", "Broca transcript entry has no mid")
                    })?;
                let ordinal = match raw.get("ordinal") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(value.as_u64().ok_or_else(|| {
                        ReadError::new(
                            "identity_unsupported",
                            format!("Broca entry {mid} has invalid ordinal {value}; expected u64"),
                        )
                    })?),
                };
                failures.insert(index, Row {
                    mid: mid.into(), ordinal, written_at: timestamp(raw),
                    display: json!({"role":"unknown", "text":"[Unreadable transcript entry]", "unreadable":true,
                        "blockSummaries":[], "parts":[{"type":"text", "text":"[Unreadable transcript entry]"}]}),
                });
                parsed.push(None);
            }
        }
    }
    let outcomes = tool_outcomes(&parsed, full);
    Ok(parsed
        .into_iter()
        .enumerate()
        .map(|(index, message)| match message {
            Some(message) => project(message, index, &outcomes, full),
            None => failures.remove(&index).expect("unreadable row retained"),
        })
        .collect())
}

/// Remove the part identity members (`partKey`, `partIndex`, `callId`) from every
/// row under `value`, giving the bytes a reader served before it carried them.
/// File parts keep `partIndex`, which they carried before the other parts did.
/// Tests use this to hold new replies to sample sets recorded without the members.
pub fn without_part_identity(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(Value::Array(parts)) = object.get_mut("parts") {
                for part in parts.iter_mut().filter_map(Value::as_object_mut) {
                    part.remove(PART_KEY);
                    part.remove(CALL_ID);
                    if part.get("type").and_then(Value::as_str) != Some("file") {
                        part.remove(PART_INDEX);
                    }
                }
            }
            if let Some(Value::Array(calls)) = object.get_mut("toolCalls") {
                for call in calls.iter_mut().filter_map(Value::as_object_mut) {
                    call.remove(CALL_ID);
                }
            }
            // Rows sit under `messages`, `newTurns` or a `resync` page. A part's own
            // members (a tool's arguments, say) are message content, not rows.
            for (key, child) in object.iter_mut() {
                if key != "parts" && key != "toolCalls" {
                    without_part_identity(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(without_part_identity),
        _ => {}
    }
}

fn tool_call_id(value: &Value) -> Option<String> {
    value
        .get("tool_call_id")
        .or_else(|| value.get("toolCallId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn decode_part(value: Value) -> Option<Part> {
    let text_part = |text: String| {
        if text.trim_start().starts_with("<system-reminder>") {
            Part::Injected(text)
        } else {
            Part::Text(text)
        }
    };
    match value {
        Value::String(text) => Some(text_part(text)),
        Value::Object(map) => {
            let kind = map
                .get("type")
                .or_else(|| map.get("kind"))
                .and_then(Value::as_str);
            let part = match kind {
                Some("text") | None => map
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| text_part(text.into()))
                    .unwrap_or_else(|| Part::Unknown(Value::Object(map.clone()))),
                Some("reasoning") => map
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| Part::Reasoning(text.into()))
                    .unwrap_or_else(|| Part::Unknown(Value::Object(map.clone()))),
                // Encrypted reasoning is private metadata, never a text fallback.
                Some("redacted_reasoning") => return None,
                Some("injected" | "system-reminder") => Part::Injected(
                    map.get("text")
                        .or_else(|| map.get("content"))
                        .map(body)
                        .unwrap_or_else(|| body(&Value::Object(map.clone()))),
                ),
                Some("file" | "artifact") => Part::File {
                    raw: Value::Object(map),
                },
                Some("tool_call") => Part::Call {
                    name: map
                        .get("name")
                        .or_else(|| map.get("tool_name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    input: map.get("input").cloned().unwrap_or(Value::Null),
                },
                Some("tool_result") => Part::Result {
                    content: map
                        .get("content")
                        .or_else(|| map.get("output"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    is_error: map
                        .get("is_error")
                        .or_else(|| map.get("isError"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                },
                Some("turn_end") => return None,
                // The provider handing a reply from one model to another: bookkeeping
                // Broca echoes back to the provider, with no text for the model or the
                // person. Shown as an unknown part, its JSON (the provider's block
                // included) would sit among the reply's text.
                Some("provider_fallback") => return None,
                Some(_) => Part::Unknown(Value::Object(map)),
            };
            Some(part)
        }
        other => Some(Part::Unknown(other)),
    }
}

struct Outcome {
    content: Value,
    is_error: bool,
}

fn tool_outcomes(messages: &[Option<Parsed>], full: bool) -> HashMap<(usize, usize), Outcome> {
    let mut unanswered: VecDeque<(usize, usize, Option<&str>)> = VecDeque::new();
    let mut outcomes = HashMap::new();
    for (index, message) in messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| m.as_ref().map(|m| (i, m)))
    {
        for (part_index, decoded) in message.parts.iter().enumerate() {
            let id = decoded.call_id.as_deref();
            match &decoded.part {
                Part::Call { .. } => unanswered.push_back((index, part_index, id)),
                Part::Result { content, is_error } => {
                    let slot = match id {
                        Some(id) => unanswered
                            .iter()
                            .position(|(_, _, call)| *call == Some(id))
                            .or_else(|| {
                                (!full)
                                    .then(|| {
                                        unanswered.iter().position(|(_, _, call)| call.is_none())
                                    })
                                    .flatten()
                            }),
                        None => (!unanswered.is_empty()).then_some(0),
                    };
                    if let Some(slot) = slot {
                        let (call_message, call_part, _) =
                            unanswered.remove(slot).expect("pending call");
                        outcomes.insert(
                            (call_message, call_part),
                            Outcome {
                                content: content.clone(),
                                is_error: *is_error,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
    }
    outcomes
}

fn project(
    message: Parsed,
    index: usize,
    outcomes: &HashMap<(usize, usize), Outcome>,
    full: bool,
) -> Row {
    let text = message
        .parts
        .iter()
        .filter_map(|decoded| match &decoded.part {
            Part::Text(text) | Part::Injected(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    let mut parts = Vec::new();
    let mut calls = Vec::new();
    let mut summaries = Vec::new();
    let mut incomplete = Vec::new();
    for (part_index, decoded) in message.parts.iter().enumerate() {
        let stored = decoded.stored;
        let mut value = match &decoded.part {
            Part::Text(text) => json!({"type":"text", "text":text}),
            Part::Reasoning(text) => json!({"type":"reasoning", "text":text}),
            Part::Injected(text) => json!({"type":"injected", "text":text}),
            Part::Call { name, input } => {
                let name = if name.is_empty() && !full {
                    "unknown"
                } else {
                    name
                };
                let serialized = if input.is_null() {
                    "{}".into()
                } else {
                    body(input)
                };
                let arguments = if full {
                    serialized.as_str()
                } else {
                    truncate(&serialized, ARGUMENT_BYTES)
                };
                let truncated = arguments.len() < serialized.len();
                let mut value = json!({"type":"tool", "tool":name, "arguments":arguments, "argumentsTruncated":truncated});
                if full {
                    value["arguments"] = input.clone();
                }
                let mut call =
                    json!({"name":name, "arguments":arguments, "argumentsTruncated":truncated});
                if let Some(outcome) = outcomes.get(&(index, part_index)) {
                    value["outputBytes"] = json!(body(&outcome.content).len());
                    call["outputBytes"] = value["outputBytes"].clone();
                    if outcome.is_error {
                        value["isError"] = json!(true);
                    }
                    if full {
                        value["content"] = json!(result_text(&outcome.content));
                        value["isError"] = json!(outcome.is_error);
                    }
                } else if full {
                    incomplete.push(name.to_string());
                }
                if let Some(id) = &decoded.call_id {
                    call[CALL_ID] = json!(id);
                    if let Some(items) = crate::tool_times::items(message.entry.tool_times.as_ref())
                    {
                        crate::tool_times::apply(&mut value, items, id);
                    }
                }
                crate::tool_input::attach(&mut value, Some(input), full);
                calls.push(call);
                summaries.push(name.to_string());
                value
            }
            Part::Result { content, is_error } if full => {
                json!({"type":"tool", "content":result_text(content), "isError":is_error})
            }
            Part::Result { .. } => continue,
            Part::File { .. } | Part::Unknown(_) if full => continue,
            Part::File { raw } => file_part(stored, raw),
            Part::Unknown(raw) => json!({"type":"text", "text":body(raw)}),
        };
        // The stored block's identity, never the part's position in this row.
        value[PART_KEY] = json!(stored.to_string());
        value[PART_INDEX] = json!(stored);
        if let Some(id) = &decoded.call_id {
            value[CALL_ID] = json!(id);
        }
        parts.push(value);
    }
    let files: Vec<Value> = parts
        .iter()
        .filter(|part| part["type"] == "file")
        .map(|part| {
            let mut descriptor = part.clone();
            let descriptor_fields = descriptor.as_object_mut().expect("file object");
            descriptor_fields.remove("type");
            // `files` already named each file's `partIndex`; it stays as it was.
            descriptor_fields.remove(PART_KEY);
            descriptor
        })
        .collect();
    let mut display = json!({"role":message.entry.message.role, "text":text, "blockSummaries":summaries, "parts":parts});
    if !calls.is_empty() {
        display["toolCalls"] = json!(calls);
    }
    if !files.is_empty() {
        display["files"] = json!(files);
    }
    if !incomplete.is_empty() {
        display["fullContentUnavailable"] = json!(format!(
            "matching tool result not retained or not yet recorded for {}",
            incomplete.join(", ")
        ));
    }
    if let Some(at) = message
        .entry
        .completed_at_ms
        .or(message.entry.message.completed_at_ms)
    {
        display["completedAtMs"] = json!(at);
    } else if message.entry.message.role == "assistant" {
        // Broca writes an assistant message only after its step completes.
        if let Some(at) = message.entry.written_at {
            display["completedAtMs"] = json!(at);
        } else {
            display["completion"] = json!("unknown");
        }
    }
    if let Some(id) = message
        .entry
        .client_message_id
        .or(message.entry.message.client_message_id)
    {
        display["clientMessageId"] = json!(id);
    }
    // Only assistant rows carry the stamp (llm-runner §9.17); the phone draws rich UI
    // only for a stamped row and plain Markdown otherwise.
    if message.entry.message.role == "assistant" {
        if let Some(stamp) = message.entry.rich_ui.as_ref().and_then(rich_ui) {
            display["richUi"] = stamp;
        }
    }
    Row {
        mid: message.entry.mid,
        ordinal: message.entry.ordinal,
        written_at: message
            .entry
            .created_at_ms
            .or(message.entry.message.created_at_ms)
            .or(message.entry.written_at),
        display,
    }
}

/// A runner's rich UI stamp as the phone reads it, or `None` when the stamp is
/// malformed (the contract's own validation), so the row is drawn without one.
pub fn rich_ui(raw: &Value) -> Option<Value> {
    let stamp =
        serde_json::from_value::<cortexkit_role_llm_runner::rich_ui::RichUiStamp>(raw.clone())
            .ok()?;
    Some(json!({
        "dialect": stamp.dialect(),
        "catalogDigest": stamp.catalog_digest(),
    }))
}

fn file_part(index: usize, raw: &Value) -> Value {
    let pointer = raw
        .get("artifact")
        .filter(|value| value.is_object())
        .unwrap_or(raw);
    let mut descriptor = json!({"type":"file", "partIndex":index});
    for (field, snake) in [
        ("mime", "mime"),
        ("filename", "filename"),
        ("attachmentId", "attachment_id"),
        ("artifactId", "artifact_id"),
        ("title", "title"),
    ] {
        if let Some(value) = pointer
            .get(field)
            .or_else(|| pointer.get(snake))
            .or_else(|| raw.get(field))
            .or_else(|| raw.get(snake))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            descriptor[field] = json!(value);
        }
    }
    for (field, snake) in [
        ("byteCount", "byte_count"),
        ("width", "width"),
        ("height", "height"),
    ] {
        if let Some(value) = pointer
            .get(field)
            .or_else(|| pointer.get(snake))
            .or_else(|| raw.get(field))
            .or_else(|| raw.get(snake))
            .and_then(Value::as_u64)
        {
            descriptor[field] = json!(value);
        }
    }
    if let Some(value) = pointer
        .get("sealed")
        .or_else(|| raw.get("sealed"))
        .and_then(Value::as_bool)
    {
        descriptor["sealed"] = json!(value);
    }
    descriptor
}

fn timestamp(raw: &Value) -> Option<i64> {
    [
        "created_at_ms",
        "createdAtMs",
        "created_at",
        "createdAt",
        "timestampMs",
        "timestamp_ms",
        "written_at",
    ]
    .into_iter()
    .find_map(|key| raw.get(key).and_then(Value::as_i64))
}

fn body(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn result_text(value: &Value) -> String {
    match value {
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect(),
        other => body(other),
    }
}

fn truncate(value: &str, cap: usize) -> &str {
    let mut end = value.len().min(cap);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_pruned_redacted_reasoning_keeps_its_existing_omission() {
        // The runner contract's opaque-reasoning example keeps a summary inside
        // redacted provider data. Pruning must not turn private payloads into display parts.
        let page = json!({
            "messages":[{"mid":"message-2", "ordinal":2, "message":{"role":"assistant", "content":[{
                "type":"redacted_reasoning",
                "data":"{\"id\":\"item-example\",\"summary\":[{\"text\":\"Consider the blue door.\",\"type\":\"summary_text\"}],\"type\":\"reasoning\"}",
                "provider_format":"example-item"
            }]}}],
            "pruned_reasoning":[{
                "mid":"message-2", "part":"m2#0", "kind":"opaque_reasoning", "bytes":245,
                "sha256":"203f324596cf074ace9b366366d9ae49d079a8a62bf3a3fbe35f4fe8f888b53b"
            }]
        });
        let entries = page["messages"].as_array().unwrap();
        let pruning =
            serde_json::from_value::<Vec<PrunedReasoning>>(page["pruned_reasoning"].clone())
                .unwrap();
        for full in [false, true] {
            let ordinary = rows(entries, full).unwrap();
            let pruned = rows_with_pruned_reasoning(entries, full, &pruning).unwrap();
            assert_eq!(pruned[0].display["parts"], json!([]));
            assert_eq!(pruned[0].display["text"], "");
            assert_eq!(
                serde_json::to_vec(&pruned[0].display).unwrap(),
                serde_json::to_vec(&ordinary[0].display).unwrap()
            );
        }
    }

    fn entry(content: Value) -> Value {
        json!({"mid":"m12", "ordinal":12, "message":{"role":"assistant", "content":content}})
    }

    #[test]
    fn broca_tool_call_page_vector_matches_opencode_shape_without_result_bodies() {
        let vector: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/broca/broca-tool-calls-v1.json"
        ))
        .unwrap();
        let mapped = rows(
            vector["brocaSessionRead"]["messages"].as_array().unwrap(),
            false,
        )
        .unwrap();
        let actual: Vec<Value> = mapped
            .into_iter()
            .map(|row| {
                let mut value = row.display;
                value["messageId"] = json!(row.mid);
                value["ordinal"] = json!(row.ordinal);
                value
            })
            .collect();
        // The vector predates part identities and completion metadata. Keep the
        // published vector intact and add the new completion expectation here.
        let mut actual = json!(actual);
        assert_eq!(actual[1]["parts"][1][CALL_ID], "call_read_1");
        without_part_identity(&mut actual);
        let mut expected = vector["reply"]["messages"].clone();
        for row in expected.as_array_mut().unwrap() {
            if row["role"] == "assistant" {
                row["completion"] = json!("unknown");
            }
        }
        assert_eq!(actual, expected);
        let serialized = actual.to_string();
        assert!(!serialized.contains("RESULT-BODY"));
        assert!(!serialized.contains("\"content\""));
    }

    #[test]
    fn captured_broca_tool_blocks_serve_name_args_and_error_flag_without_body() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/broca/broca-session-read-tool-blocks-live.json"
        ))
        .unwrap();
        let mapped = rows(fixture["response"]["messages"].as_array().unwrap(), false).unwrap();
        let turn = &mapped[0].display;
        assert_eq!(mapped[0].ordinal, Some(8));
        assert_eq!(turn["blockSummaries"], json!(["read"]));
        let part = &turn["parts"][0];
        assert_eq!(part["tool"], "read");
        assert_eq!(part["argumentsTruncated"], false);
        let arguments: Value = serde_json::from_str(part["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(arguments["filePath"], ".");
        assert_eq!(part["isError"], true);
        assert_eq!(
            part["outputBytes"],
            json!({"kind":"error", "message":"tool plane unavailable"})
                .to_string()
                .len()
        );
        assert_eq!(turn["toolCalls"][0]["outputBytes"], part["outputBytes"]);
        assert_eq!(mapped[1].display["parts"], json!([]));
        assert!(
            !json!(mapped.iter().map(|row| &row.display).collect::<Vec<_>>())
                .to_string()
                .contains("tool plane unavailable")
        );
    }

    #[test]
    fn runner_parts_cap_arguments_drop_result_bodies_and_name_unknown_raw_kinds() {
        let large_body = "é".repeat(9_000);
        let mapped = rows(
            &[entry(json!([
                {"type":"tool_call", "name":"search", "input":{"query":"é".repeat(3_000)}},
                {"type":"tool_result", "content":large_body},
                {"type":"tool_result", "content":large_body, "is_error":true},
                {"type":"provider_trace", "trace":"opaque"},
            ]))],
            false,
        )
        .unwrap();
        let turn = &mapped[0].display;
        assert_eq!(turn["blockSummaries"], json!(["search"]));
        let parts = turn["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert!(parts[0]["arguments"].as_str().unwrap().len() <= ARGUMENT_BYTES);
        assert_eq!(parts[0]["argumentsTruncated"], true);
        assert_eq!(parts[0]["outputBytes"], large_body.len());
        assert!(parts[0].get("isError").is_none());
        assert_eq!(turn["toolCalls"][0]["arguments"], parts[0]["arguments"]);
        assert_eq!(
            parts[1],
            json!({"type":"text", "text":"{\"trace\":\"opaque\",\"type\":\"provider_trace\"}",
                PART_KEY:"3", PART_INDEX:3})
        );
        assert!(!turn.to_string().contains(&large_body));
    }

    #[test]
    fn unified_runner_projection_carries_reasoning_tool_file_and_injected_parts() {
        let mapped = rows(&[entry(json!([
            {"type":"text", "text":"visible"}, {"type":"reasoning", "text":"because"},
            {"type":"tool_call", "name":"read", "input":{"filePath":"README.md"}},
            {"type":"tool_result", "content":"ok"},
            {"type":"injected", "text":"<system-reminder>hide me</system-reminder>"},
            {"type":"file", "mime":"image/png", "filename":"proof.png", "byteCount":12, "width":3, "height":2},
            {"type":"redacted_reasoning", "data":"encrypted"},
        ]))], false).unwrap();
        let parts = mapped[0].display["parts"].as_array().unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|part| part["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["text", "reasoning", "tool", "injected", "file"]
        );
        assert_eq!(parts[1]["text"], "because");
        assert_eq!(parts[2]["outputBytes"], 2);
        assert_eq!(parts[4]["width"], 3);
        assert_eq!(mapped[0].display["files"][0]["partIndex"], 5);
        assert!(!mapped[0].display.to_string().contains("encrypted"));
    }

    #[test]
    fn timestamps_client_ids_unreadable_entries_and_missing_identity_are_explicit() {
        let mut raw = entry(json!(["answer"]));
        raw["message"]["createdAtMs"] = json!(700);
        raw["created_at_ms"] = json!(710);
        raw["message"]["completed_at"] = json!(720);
        raw["written_at"] = json!(730);
        raw["client_message_id"] = json!("phone-id");
        let mapped = rows(&[raw.clone()], false).unwrap();
        assert_eq!(mapped[0].written_at, Some(710));
        assert_eq!(mapped[0].display["completedAtMs"], 720);
        assert!(mapped[0].display.get("completion").is_none());
        assert_eq!(mapped[0].display["clientMessageId"], "phone-id");
        raw["completed_at_ms"] = json!(740);
        let mapped = rows(&[raw], false).unwrap();
        assert_eq!(mapped[0].written_at, Some(710));
        assert_eq!(mapped[0].display["completedAtMs"], 740);
        assert!(mapped[0].display.get("completion").is_none());
        let broken = json!({"mid":"broken", "ordinal":13, "message":{"content":false}});
        let mapped = rows(&[entry(json!([])), broken], false).unwrap();
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[1].mid, "broken");
        assert_eq!(mapped[1].display["unreadable"], true);
        assert_eq!(
            rows(&[json!({"ordinal":14})], false).unwrap_err().code,
            "identity_unsupported"
        );
        let error = rows(&[json!({"mid":"m", "ordinal":"not-an-ordinal"})], false).unwrap_err();
        assert!(error.message.contains("not-an-ordinal") && error.message.contains("u64"));
    }

    #[test]
    fn exact_content_is_not_capped_and_pairs_results_by_id() {
        let input = json!({"query":"é".repeat(3_000)});
        let result = "result".repeat(4_000);
        let mapped = rows(
            &[entry(json!([
                {"type":"tool_call", "tool_call_id":"a", "name":"read", "input":input},
                {"type":"tool_result", "tool_call_id":"a", "output":result, "is_error":true},
            ]))],
            true,
        )
        .unwrap();
        assert_eq!(mapped[0].display["parts"][0]["arguments"], input);
        assert_eq!(mapped[0].display["parts"][0]["argumentsTruncated"], false);
        assert_eq!(mapped[0].display["parts"][0]["content"], result);
        assert_eq!(mapped[0].display["parts"][0]["isError"], true);
    }

    #[test]
    fn exact_arguments_keep_json_null_and_array_results_keep_only_text_blocks() {
        let mapped = rows(&[entry(json!([
            {"type":"tool_call", "name":"read", "input":null},
            {"type":"tool_result", "content":[{"type":"text", "text":"first"}, {"type":"image", "data":"private"}, {"type":"text", "text":"second"}]},
            {"type":"provider_trace", "trace":"unknown"},
            {"type":"file", "artifact_id":"artifact"},
        ]))], true).unwrap();
        let parts = mapped[0].display["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert!(parts[0]["arguments"].is_null());
        assert_eq!(parts[0]["content"], "firstsecond");
        assert_eq!(parts[1]["content"], "firstsecond");
        assert!(mapped[0].display.get("fullContentUnavailable").is_none());
        let missing = rows(&[entry(json!([{"type":"tool_call", "name":"read"}]))], true).unwrap();
        assert!(missing[0].display["fullContentUnavailable"]
            .as_str()
            .unwrap()
            .ends_with("for read"));
    }

    /// (row position, `partKey`, `partIndex`, `callId`) for each display part.
    fn identities(display: &Value) -> Vec<(usize, Value, Value, Value)> {
        display["parts"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(at, part)| {
                (
                    at,
                    part[PART_KEY].clone(),
                    part[PART_INDEX].clone(),
                    part[CALL_ID].clone(),
                )
            })
            .collect()
    }

    /// Encrypted reasoning and a turn end are stored but never shown, and a page read
    /// folds a tool result into its call (a full read shows it as its own part), so
    /// shown parts sit earlier in the row than in the stored message. The
    /// identities follow the stored message.
    #[test]
    fn part_identity_names_the_stored_block_not_the_row_position() {
        let content = json!([
            {"type":"redacted_reasoning", "data":"encrypted"},
            {"type":"reasoning", "text":"check the year"},
            {"type":"text", "text":""},
            {"type":"tool_call", "tool_call_id":"call_a", "name":"bash", "input":{"command":"date"}},
            {"type":"tool_result", "tool_call_id":"call_a", "content":"2026\n"},
            {"type":"turn_end"},
            {"type":"text", "text":"It is 2026."},
            {"type":"tool_call", "name":"read", "input":{"filePath":"x"}},
            {"type":"file", "mime":"image/png", "filename":"proof.png"},
        ]);
        let page = rows(&[entry(content.clone())], false).unwrap();
        let display = &page[0].display;
        let null = Value::Null;
        assert_eq!(
            identities(display),
            [
                (0, json!("1"), json!(1), null.clone()),
                (1, json!("2"), json!(2), null.clone()),
                (2, json!("3"), json!(3), json!("call_a")),
                (3, json!("6"), json!(6), null.clone()),
                // A call stored without an id has no `callId`, never an invented one.
                (4, json!("7"), json!(7), null.clone()),
                (5, json!("8"), json!(8), null.clone()),
            ]
        );
        assert_eq!(display["parts"][2]["outputBytes"], 5);
        assert_eq!(display["toolCalls"][0][CALL_ID], "call_a");
        assert!(display["toolCalls"][1].get(CALL_ID).is_none());
        // `files` keeps the shape it had: a `partIndex` and no `partKey`.
        assert_eq!(
            display["files"],
            json!([{"partIndex":8, "mime":"image/png", "filename":"proof.png"}])
        );

        // The full read shows the result as its own part, which lands on its call.
        let full = rows(&[entry(content)], true).unwrap();
        assert_eq!(
            identities(&full[0].display),
            [
                (0, json!("1"), json!(1), null.clone()),
                (1, json!("2"), json!(2), null.clone()),
                (2, json!("3"), json!(3), json!("call_a")),
                (3, json!("4"), json!(4), json!("call_a")),
                (4, json!("6"), json!(6), null.clone()),
                (5, json!("7"), json!(7), null.clone()),
            ]
        );
    }

    #[test]
    fn a_tool_result_in_a_later_message_lands_on_its_call_by_call_id() {
        let call = entry(json!([
            {"type":"text", "text":"Looking."},
            {"type":"tool_call", "tool_call_id":"call_b", "name":"read", "input":{}},
        ]));
        let result = json!({"mid":"m13", "ordinal":13, "message":{"role":"tool", "content":[
            {"type":"tool_result", "tool_call_id":"call_b", "output":"ok"}]}});
        let page = rows(&[call, result], false).unwrap();
        let tool = &page[0].display["parts"][1];
        assert_eq!(
            (&tool[CALL_ID], &tool[PART_KEY], &tool[PART_INDEX]),
            (&json!("call_b"), &json!("1"), &json!(1))
        );
        assert_eq!(page[0].display["toolCalls"][0][CALL_ID], "call_b");
        assert_eq!(page[1].display["parts"], json!([]));
    }

    #[test]
    fn the_unreadable_placeholder_part_carries_no_part_identity() {
        let broken = json!({"mid":"broken", "ordinal":13, "message":{"content":false}});
        let page = rows(&[broken], false).unwrap();
        let part = &page[0].display["parts"][0];
        assert_eq!(part["text"], "[Unreadable transcript entry]");
        for member in [PART_KEY, PART_INDEX, CALL_ID] {
            assert!(part.get(member).is_none(), "{member}: {part}");
        }
    }

    #[test]
    fn removing_part_identity_leaves_file_indexes_and_tool_arguments_alone() {
        let mut reply = json!({"messages":[{"parts":[
            {"type":"text", "text":"a", PART_KEY:"0", PART_INDEX:0},
            {"type":"file", PART_KEY:"1", PART_INDEX:1},
            {"type":"tool", CALL_ID:"c", "arguments":{"parts":[{PART_KEY:"kept"}]}},
        ], "toolCalls":[{"name":"read", CALL_ID:"c"}]}]});
        without_part_identity(&mut reply);
        assert_eq!(
            reply,
            json!({"messages":[{"parts":[
                {"type":"text", "text":"a"},
                {"type":"file", PART_INDEX:1},
                {"type":"tool", "arguments":{"parts":[{PART_KEY:"kept"}]}},
            ], "toolCalls":[{"name":"read"}]}]})
        );
    }
}
