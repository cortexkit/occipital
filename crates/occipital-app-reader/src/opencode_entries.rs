//! OpenCode `session.read` entries as the phone's transcript rows.
//!
//! An entry's `message` is OpenCode's own `{info, parts}` message. The row shape is
//! ported from the OpenCode plugin that served these rows to the phone before Thalamus
//! did: prefrontal `9b411a17b`, `packages/opencode/src/features/host-bridge/`
//! (`canonical-adapter.ts`, the `transcriptMessage*` helpers, and
//! `host-provider-deps.ts`, `transcriptPageFromOrdinalMessages`). So a session read
//! through this reader shows the rows it showed when the plugin served it. Two
//! differences:
//! - Like the Broca rows, every part built from a stored part also carries the
//!   stored part's identity: `partKey` (OpenCode's `prt_…` id), `partIndex` (its
//!   position in the stored `parts`) and, on tool parts and `toolCalls`, `callId`.
//! - JSON the row shows as text (a tool call's `arguments`, and a part kind the row
//!   doesn't know) has its object members in sorted order, since the reader holds the
//!   message as a parsed value; the plugin kept the host's member order.
use serde_json::{json, Map, Value};

use crate::source::{ReadError, Row};

/// Per-call argument cap on list pages, the plugin's
/// `TRANSCRIPT_TOOL_ARGUMENT_CAP_BYTES`.
const ARGUMENT_BYTES: usize = 2048;
const TOOL_RESULT_BYTES: usize = 16_384;
const ERROR_RESULT_BYTES: usize = 64 * 1024;
const PART_KEY: &str = "partKey";
const PART_INDEX: &str = "partIndex";
const CALL_ID: &str = "callId";

/// Rows for stored entries, in the order given. `full` is an exact message read: tool
/// calls keep their whole input and their output.
pub fn rows(entries: &[Value], full: bool) -> Result<Vec<Row>, ReadError> {
    entries.iter().map(|entry| row(entry, full)).collect()
}

fn row(entry: &Value, full: bool) -> Result<Row, ReadError> {
    let mid = entry["mid"]
        .as_str()
        .filter(|mid| !mid.is_empty())
        .ok_or_else(|| ReadError::new("identity_unsupported", "OpenCode entry has no mid"))?;
    let ordinal = match entry.get("ordinal") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| {
            ReadError::new(
                "identity_unsupported",
                format!("OpenCode entry {mid} has invalid ordinal {value}; expected u64"),
            )
        })?),
    };
    let message = &entry["message"];
    let (Some(info), Some(parts)) = (message["info"].as_object(), message["parts"].as_array())
    else {
        // A broken payload stays visible, but no identity is invented for its parts.
        return Ok(Row {
            mid: mid.into(),
            ordinal,
            written_at: entry["written_at"].as_i64(),
            display: json!({"role":"unknown", "text":"[Unreadable transcript entry]", "unreadable":true,
                "blockSummaries":[], "parts":[{"type":"text", "text":"[Unreadable transcript entry]"}]}),
        });
    };
    let role = info.get("role").and_then(Value::as_str).unwrap_or_default();
    let text: String = parts
        .iter()
        .filter(|part| part["type"] == "text")
        .filter_map(|part| part["text"].as_str())
        .collect();
    let summaries: Vec<String> = parts
        .iter()
        .filter(|part| part["type"] == "tool")
        .map(tool_name)
        .collect();
    let calls: Vec<Value> = parts
        .iter()
        .filter(|part| part["type"] == "tool")
        .map(|part| {
            let mut call = tool_call(part, false);
            if let Some(id) = call_id(part) {
                call[CALL_ID] = json!(id);
            }
            call
        })
        .collect();
    let files: Vec<Value> = parts
        .iter()
        .enumerate()
        .filter_map(|(index, part)| file(index, part))
        .collect();
    let mut display = json!({
        "role": role,
        "text": text,
        "blockSummaries": summaries,
        "parts": display_parts(parts, &files, full, crate::tool_times::items(entry.get("tool_times"))),
    });
    if !calls.is_empty() {
        display["toolCalls"] = json!(calls);
    }
    if !files.is_empty() {
        display["files"] = json!(files);
    }
    let time = info.get("time");
    if let Some(completed) = time.and_then(|time| time["completed"].as_i64()) {
        display["completedAtMs"] = json!(completed);
    }
    if let Some(model) = model(info) {
        display["model"] = model;
    }
    // The phone's own id for a user message it sent, echoed onto the entry from the
    // runner's display history (`client_message_id` on that row).
    if role == "user" {
        if let Some(id) = entry["client_message_id"]
            .as_str()
            .filter(|id| !id.is_empty())
        {
            display["clientMessageId"] = json!(id);
        }
    }
    // Only assistant rows carry the stamp the runner serves on an opted-in read
    // (llm-runner §9.17), as on Broca rows.
    if role == "assistant" {
        if let Some(stamp) = entry.get("rich_ui").and_then(crate::broca_entries::rich_ui) {
            display["richUi"] = stamp;
        }
    }
    Ok(Row {
        mid: mid.into(),
        ordinal,
        written_at: time
            .and_then(|time| time["created"].as_i64())
            .or_else(|| entry["written_at"].as_i64()),
        display,
    })
}

/// An assistant message's model, only when both halves are known: a partial or user
/// record stays absent, so an unknown model never reads as a known one.
fn model(info: &Map<String, Value>) -> Option<Value> {
    if info.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let field = |nested: &str, flat: &str| {
        info.get("model")
            .and_then(|model| model.get(nested))
            .or_else(|| info.get(flat))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let provider = field("providerID", "providerID")?;
    let model = field("modelID", "modelID")?;
    Some(json!({"providerID": provider, "modelID": model}))
}

fn tool_name(part: &Value) -> String {
    part["tool"]
        .as_str()
        .or_else(|| part["name"].as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_owned()
}

fn call_id(part: &Value) -> Option<&str> {
    part["callID"].as_str().filter(|id| !id.is_empty())
}

/// A tool call as a list page shows it: the compact JSON of its input as a string,
/// cut at the argument cap on a character boundary unless `full`.
fn tool_call(part: &Value, full: bool) -> Value {
    let input = part["state"].get("input").filter(|input| !input.is_null());
    let serialized = input.map_or_else(|| "{}".to_owned(), Value::to_string);
    let (arguments, truncated) = if full {
        (serialized.as_str(), false)
    } else {
        let cut = truncate(&serialized, ARGUMENT_BYTES);
        (cut, cut.len() < serialized.len())
    };
    let mut call =
        json!({"name": tool_name(part), "arguments": arguments, "argumentsTruncated": truncated});
    if let Some(output) = part["state"]["output"].as_str() {
        call["outputBytes"] = json!(output.len());
    }
    call
}

/// `times` is what the runner served beside the entry when the read asked for
/// display facts; without it, a tool part takes the times OpenCode stored on it.
fn display_parts(
    parts: &[Value],
    files: &[Value],
    full: bool,
    times: Option<&[Value]>,
) -> Vec<Value> {
    let mut shown = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let kind = part["type"].as_str();
        let mut value = match kind {
            None | Some("text") if part["text"].is_string() => {
                let text = part["text"].as_str().unwrap_or_default();
                let kind = if text.trim_start().starts_with("<system-reminder>") {
                    "injected"
                } else {
                    "text"
                };
                json!({"type": kind, "text": text})
            }
            Some("reasoning") => {
                json!({"type":"reasoning", "text": part["text"].as_str().unwrap_or_default()})
            }
            Some("tool") => tool_part(part, full, times),
            Some("tool_result") => tool_result(part, full),
            Some("file" | "artifact") => {
                match files.iter().find(|file| file[PART_INDEX] == json!(index)) {
                    Some(file) => {
                        let mut file = file.clone();
                        file["type"] = json!("file");
                        file
                    }
                    None => unknown(part),
                }
            }
            Some("injected" | "system-reminder") => {
                let body = part
                    .get("text")
                    .filter(|value| !value.is_null())
                    .or_else(|| part.get("content").filter(|value| !value.is_null()))
                    .unwrap_or(part);
                json!({"type":"injected", "text": text_of(body)})
            }
            // OpenCode's control markers carry no renderable body.
            Some("step-start" | "step-finish" | "redacted_reasoning") => continue,
            _ => unknown(part),
        };
        if let Some(id) = part["id"].as_str().filter(|id| !id.is_empty()) {
            value[PART_KEY] = json!(id);
        }
        value[PART_INDEX] = json!(index);
        if matches!(kind, Some("tool" | "tool_result")) {
            if let Some(id) = call_id(part) {
                value[CALL_ID] = json!(id);
            }
        }
        shown.push(value);
    }
    shown
}

/// A tool call part with the host's own state for it. An errored call has no output,
/// so without `isError` and `error` it would read as one still waiting. A list page
/// shows only the error's first line, as a page does for every tool output it doesn't
/// carry; an exact message read carries the whole error text.
fn tool_part(part: &Value, full: bool, times: Option<&[Value]>) -> Value {
    let call = tool_call(part, full);
    let mut value = json!({
        "type": "tool",
        "tool": call["name"],
        "arguments": call["arguments"],
        "argumentsTruncated": call["argumentsTruncated"],
    });
    if let Some(bytes) = call.get("outputBytes") {
        value["outputBytes"] = bytes.clone();
    }
    let state = &part["state"];
    if let Some(status) = state["status"].as_str().filter(|status| !status.is_empty()) {
        value["status"] = json!(status);
    }
    match (times, call_id(part)) {
        (Some(times), Some(id)) => crate::tool_times::apply(&mut value, times, id),
        (Some(_), None) => {}
        // A runner that doesn't serve display facts: OpenCode stores the call's own
        // times, in Unix milliseconds, `start` when the tool began running and `end`
        // when it answered. A missing or zero time is left out, never filled in
        // from another clock or from the message's time.
        (None, _) => {
            for (stored, member) in [("start", "startedAtMs"), ("end", "finishedAtMs")] {
                if let Some(at) = state["time"][stored].as_u64().filter(|at| *at > 0) {
                    value[member] = json!(at);
                }
            }
        }
    }
    crate::tool_input::attach(&mut value, state.get("input"), full);
    if let Some(error) = state["error"].as_str() {
        value["isError"] = json!(true);
        value["error"] = json!(error
            .split('\n')
            .next()
            .unwrap_or_default()
            .trim_end_matches('\r'));
    }
    if full {
        if let Some(output) = state["output"].as_str().or_else(|| state["error"].as_str()) {
            value["content"] = json!(output);
        }
        if let Some(error) = state["error"].as_str() {
            value["error"] = json!(error);
        }
    }
    value
}

fn tool_result(part: &Value, full: bool) -> Value {
    let state = &part["state"];
    let body = text_of(
        part.get("content")
            .filter(|value| !value.is_null())
            .or_else(|| state.get("output").filter(|value| !value.is_null()))
            .unwrap_or(&Value::Null),
    );
    let is_error =
        state["error"].is_string() || part["isError"] == true || part["is_error"] == true;
    let cap = if is_error {
        ERROR_RESULT_BYTES
    } else {
        TOOL_RESULT_BYTES
    };
    let visible = truncate(&body, cap);
    let mut value = json!({"type":"tool", "content": visible, "isError": is_error,
        "byteCount": body.len(), "truncated": visible.len() < body.len()});
    if full {
        let whole = text_of(
            part.get("content")
                .filter(|value| !value.is_null())
                .or_else(|| state.get("output").filter(|value| !value.is_null()))
                .or_else(|| state.get("error").filter(|value| !value.is_null()))
                .unwrap_or(&Value::Null),
        );
        value["content"] = json!(whole);
        value["truncated"] = json!(false);
    }
    value
}

/// A file or artifact part as a pointer-only descriptor: never its bytes, and only a
/// filename's last path segment, since the machine path is private.
fn file(index: usize, part: &Value) -> Option<Value> {
    let pointer = part
        .get("artifact")
        .filter(|value| value.is_object())
        .unwrap_or(part);
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let artifact = text(pointer.get("artifactId")).or_else(|| text(pointer.get("artifact_id")));
    if part["type"] != "file" && artifact.is_none() {
        return None;
    }
    let mut descriptor = json!({"partIndex": index});
    if let Some(mime) = text(pointer.get("mime")).or_else(|| text(part.get("mime"))) {
        descriptor["mime"] = json!(mime);
    }
    if let Some(name) = text(part.get("filename")) {
        let base = name.rsplit('/').next().unwrap_or(&name);
        descriptor["filename"] = json!(base);
    }
    let count = pointer
        .get("byteCount")
        .or_else(|| pointer.get("byte_count"))
        .and_then(Value::as_u64)
        .or_else(|| {
            artifact
                .is_none()
                .then(|| part["url"].as_str().and_then(data_url_bytes))
                .flatten()
        });
    if let Some(count) = count {
        descriptor["byteCount"] = json!(count);
    }
    if let Some(id) = text(part.get("attachmentId")) {
        descriptor["attachmentId"] = json!(id);
    }
    if let Some(id) = &artifact {
        descriptor["artifactId"] = json!(id);
        if let Some(title) = text(pointer.get("title")) {
            descriptor["title"] = json!(title);
        }
        if let Some(sealed) = pointer.get("sealed").and_then(Value::as_bool) {
            descriptor["sealed"] = json!(sealed);
        }
    }
    Some(descriptor)
}

/// The decoded size of a data URL's payload, counted without decoding: base64 from its
/// length, otherwise one byte per percent escape and each character's UTF-8 length.
fn data_url_bytes(url: &str) -> Option<u64> {
    let (header, payload) = url.strip_prefix("data:")?.split_once(',')?;
    if header.contains(";base64") {
        let padding = if url.ends_with("==") {
            2
        } else if url.ends_with('=') {
            1
        } else {
            0
        };
        return Some(((payload.len() as u64 * 3) / 4).saturating_sub(padding));
    }
    let bytes = payload.as_bytes();
    let (mut count, mut at) = (0u64, 0usize);
    while at < bytes.len() {
        if bytes[at] == b'%'
            && bytes.len() > at + 2
            && bytes[at + 1].is_ascii_hexdigit()
            && bytes[at + 2].is_ascii_hexdigit()
        {
            count += 1;
            at += 3;
        } else {
            count += 1;
            at += 1;
        }
    }
    Some(count)
}

/// An unknown part kind as text: its JSON, so its bytes stay visible.
fn unknown(part: &Value) -> Value {
    json!({"type":"text", "text": part.to_string()})
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
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

    fn entry(message: Value) -> Value {
        json!({"mid": message["info"]["id"], "ordinal": 3, "message": message})
    }

    #[test]
    fn an_assistant_turn_shows_text_reasoning_and_a_compact_tool_call() {
        let long = "x".repeat(ARGUMENT_BYTES + 10);
        let message = json!({
        "info": {"id":"msg_a", "role":"assistant", "time":{"created": 1000, "completed": 1500},
            "providerID":"anthropic", "modelID":"claude-opus-5-5"},
        "parts": [
            {"id":"prt_0", "type":"step-start"},
            {"id":"prt_1", "type":"reasoning", "text":"thinking"},
            {"id":"prt_2", "type":"text", "text":"Hello "},
            {"id":"prt_3", "type":"tool", "tool":"bash", "callID":"call_1",
                "state":{"status":"completed", "input":{"command": long}, "output":"done\n"}},
            {"id":"prt_4", "type":"tool", "tool":"read", "callID":"call_2",
                "state":{"status":"error", "input":{}, "error":"no such file\nstack"}},
            {"id":"prt_5", "type":"text", "text":"<system-reminder>note</system-reminder>"},
            {"id":"prt_6", "type":"step-finish"}
        ]});
        let row = &rows(&[entry(message.clone())], false).unwrap()[0];
        assert_eq!(row.mid, "msg_a");
        assert_eq!(row.ordinal, Some(3));
        assert_eq!(row.written_at, Some(1000));
        let display = &row.display;
        assert_eq!(display["role"], "assistant");
        assert_eq!(
            display["text"],
            "Hello <system-reminder>note</system-reminder>"
        );
        assert_eq!(display["blockSummaries"], json!(["bash", "read"]));
        assert_eq!(display["completedAtMs"], 1500);
        assert_eq!(
            display["model"],
            json!({"providerID":"anthropic","modelID":"claude-opus-5-5"})
        );
        let parts = display["parts"].as_array().unwrap();
        // Step markers show nothing; every shown part keeps its stored identity.
        assert_eq!(
            parts
                .iter()
                .map(|p| p[PART_INDEX].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        assert_eq!(
            parts[0],
            json!({"type":"reasoning","text":"thinking","partKey":"prt_1","partIndex":1})
        );
        let bash = &parts[2];
        assert_eq!(bash["tool"], "bash");
        assert_eq!(bash["argumentsTruncated"], true);
        assert_eq!(bash["arguments"].as_str().unwrap().len(), ARGUMENT_BYTES);
        assert_eq!(bash["outputBytes"], 5);
        assert_eq!(bash["status"], "completed");
        assert_eq!(bash[CALL_ID], "call_1");
        assert!(
            bash.get("content").is_none(),
            "a list page carries no output"
        );
        let read = &parts[3];
        assert_eq!(read["isError"], true);
        assert_eq!(read["error"], "no such file");
        assert_eq!(parts[4]["type"], "injected");
        assert_eq!(display["toolCalls"][0]["callId"], "call_1");
        assert_eq!(display["toolCalls"][1]["arguments"], "{}");

        // An exact read keeps the whole input, the output and the whole error.
        let full = &rows(&[entry(message)], true).unwrap()[0].display["parts"];
        assert_eq!(full[2]["argumentsTruncated"], false);
        assert_eq!(full[2]["content"], "done\n");
        assert_eq!(full[3]["error"], "no such file\nstack");
        assert_eq!(full[3]["content"], "no such file\nstack");
    }

    /// A tool part carries the call's own stored times on list pages and exact
    /// reads alike. A call without them, or with a zero, carries neither member.
    #[test]
    fn a_tool_part_carries_the_stored_call_times_and_never_invents_one() {
        let message = json!({
        "info": {"id":"msg_a", "role":"assistant", "time":{"created": 1000, "completed": 9000}},
        "parts": [
            {"id":"prt_1", "type":"tool", "tool":"bash", "callID":"call_1",
                "state":{"status":"completed", "input":{}, "output":"",
                    "time":{"start": 2000, "end": 3500}}},
            {"id":"prt_2", "type":"tool", "tool":"bash", "callID":"call_2",
                "state":{"status":"running", "input":{}, "time":{"start": 4000}}},
            {"id":"prt_3", "type":"tool", "tool":"read", "callID":"call_3",
                "state":{"status":"error", "input":{}, "error":"gone",
                    "time":{"start": 0, "end": 5000}}},
            {"id":"prt_4", "type":"tool", "tool":"read", "callID":"call_4",
                "state":{"status":"pending", "input":{}}}
        ]});
        for full in [false, true] {
            let parts = rows(&[entry(message.clone())], full).unwrap()[0].display["parts"].clone();
            let times: Vec<_> = parts
                .as_array()
                .unwrap()
                .iter()
                .map(|part| {
                    (
                        part.get("startedAtMs").cloned(),
                        part.get("finishedAtMs").cloned(),
                    )
                })
                .collect();
            assert_eq!(
                times,
                [
                    (Some(json!(2000)), Some(json!(3500))),
                    (Some(json!(4000)), None),
                    (None, Some(json!(5000))),
                    (None, None),
                ],
                "full {full}"
            );
            // OpenCode records no reason a call went unanswered.
            assert!(parts
                .as_array()
                .unwrap()
                .iter()
                .all(|part| part.get("unanswered").is_none()));
        }
    }

    /// When the runner serves the calls' times beside the entry, they are the only
    /// source: the stored part's times are not read, so a call the runner names no
    /// time for carries none.
    #[test]
    fn served_tool_times_replace_the_stored_part_times() {
        let message = json!({
        "info": {"id":"msg_a", "role":"assistant", "time":{"created": 1000}},
        "parts": [
            {"id":"prt_1", "type":"tool", "tool":"bash", "callID":"call_1",
                "state":{"status":"completed", "input":{}, "output":"", "time":{"start": 1, "end": 2}}},
            {"id":"prt_2", "type":"tool", "tool":"bash", "callID":"call_2",
                "state":{"status":"error", "input":{}, "error":"aborted", "time":{"start": 3, "end": 4}}}
        ]});
        let mut served = entry(message);
        served["tool_times"] = json!([
            {"tool_call_id": "call_2", "started_at": 3000, "unanswered": "interrupted"}
        ]);
        let parts = rows(&[served], false).unwrap()[0].display["parts"].clone();
        let first = &parts[0];
        assert!(
            first.get("startedAtMs").is_none() && first.get("finishedAtMs").is_none(),
            "{first}"
        );
        assert_eq!(parts[1]["startedAtMs"], 3000);
        assert!(parts[1].get("finishedAtMs").is_none());
        assert_eq!(parts[1]["unanswered"], "interrupted");
    }

    #[test]
    fn files_are_pointers_with_a_basename_and_unknown_parts_stay_visible() {
        let message = json!({
        "info": {"id":"msg_u", "role":"user", "time":{"created": 7}},
        "parts": [
            {"id":"prt_0", "type":"text", "text":"see"},
            {"id":"prt_1", "type":"file", "mime":"image/png", "filename":"/Users/someone/shot.png",
                "url":"data:image/png;base64,AAAA"},
            {"id":"prt_2", "type":"compaction", "auto":true}
        ]});
        let row = &rows(&[entry(message)], false).unwrap()[0];
        let display = &row.display;
        assert_eq!(
            display["files"],
            json!([{"partIndex":1, "mime":"image/png", "filename":"shot.png", "byteCount":3}])
        );
        assert_eq!(display["parts"][1]["type"], "file");
        assert_eq!(display["parts"][1]["partKey"], "prt_1");
        assert_eq!(display["parts"][2]["type"], "text");
        assert!(display["parts"][2]["text"]
            .as_str()
            .unwrap()
            .contains("compaction"));
        assert!(display.get("model").is_none(), "a user row names no model");
        assert!(display.get("completedAtMs").is_none());
    }

    /// `test-vectors/opencode-rows-v1/rows.json` is the plugin's own output for the
    /// same messages, produced by running its helpers (`generate.ts`). Without the part
    /// identity members this port adds, the rows match it exactly, apart from member
    /// order in JSON shown as text, which `canonical_text` sets aside.
    #[test]
    fn rows_match_what_the_plugin_built_for_the_same_messages() {
        let vector: Value =
            serde_json::from_str(include_str!("../test-vectors/opencode-rows-v1/rows.json"))
                .unwrap();
        let mut expected = vector["rows"].as_array().unwrap().iter();
        for message in vector["messages"].as_array().unwrap() {
            for full in [false, true] {
                let want = expected.next().unwrap();
                assert_eq!(want["full"], full);
                let mut got = rows(&[entry(message.clone())], full).unwrap()[0]
                    .display
                    .clone();
                crate::broca_entries::without_part_identity(&mut got);
                canonical_text(&mut got);
                // The page builder's members, which the helpers don't produce.
                for member in ["completedAtMs", "model"] {
                    got.as_object_mut().unwrap().remove(member);
                }
                let mut want = want.clone();
                want.as_object_mut().unwrap().remove("full");
                canonical_text(&mut want);
                assert_eq!(got, want, "{} full={full}", message["info"]["id"]);
            }
        }
    }

    /// A part whose text is a JSON object, re-serialized so member order doesn't count.
    fn canonical_text(row: &mut Value) {
        for part in row["parts"].as_array_mut().unwrap() {
            let parsed = part["text"]
                .as_str()
                .and_then(|text| serde_json::from_str::<Map<String, Value>>(text).ok());
            if let Some(object) = parsed {
                part["text"] = json!(Value::Object(object).to_string());
            }
        }
    }

    #[test]
    fn a_user_row_carries_the_echoed_client_message_id_and_an_assistant_row_never_does() {
        let user = json!({"mid":"msg_u", "ordinal":4, "client_message_id":"phone-1",
            "message":{"info":{"id":"msg_u", "role":"user", "time":{"created":1}}, "parts":[]}});
        let assistant = json!({"mid":"msg_a", "ordinal":5, "client_message_id":"phone-1",
            "message":{"info":{"id":"msg_a", "role":"assistant", "time":{"created":2}}, "parts":[]}});
        let rows = rows(&[user, assistant], false).unwrap();
        assert_eq!(rows[0].display["clientMessageId"], "phone-1");
        assert!(rows[1].display.get("clientMessageId").is_none());
    }

    /// The runner's stamp reaches an assistant row only, and a malformed one draws the row
    /// without it.
    #[test]
    fn only_an_assistant_row_takes_the_runners_rich_ui_stamp() {
        let stamp = json!({"dialect":"1.0", "catalog_digest":"8".repeat(64)});
        let entry = |mid: &str, role: &str, stamp: &Value| {
            json!({"mid":mid, "ordinal":1, "rich_ui":stamp,
                "message":{"info":{"id":mid, "role":role, "time":{"created":1}}, "parts":[]}})
        };
        let rows = rows(
            &[
                entry("msg_a", "assistant", &stamp),
                entry("msg_u", "user", &stamp),
                entry(
                    "msg_b",
                    "assistant",
                    &json!({"dialect":"1.0", "catalog_digest":"x"}),
                ),
            ],
            false,
        )
        .unwrap();
        assert_eq!(
            rows[0].display["richUi"],
            json!({"dialect":"1.0", "catalogDigest":"8".repeat(64)})
        );
        assert!(rows[1].display.get("richUi").is_none());
        assert!(rows[2].display.get("richUi").is_none());
    }

    #[test]
    fn a_data_url_is_counted_by_its_decoded_size() {
        assert_eq!(data_url_bytes("data:text/plain,%41%42%43"), Some(3));
        assert_eq!(data_url_bytes("data:text/plain,\u{1F600}"), Some(4));
        assert_eq!(data_url_bytes("data:;base64,AAA="), Some(2));
        assert_eq!(data_url_bytes("https://x"), None);
    }

    #[test]
    fn an_entry_without_a_message_is_an_unreadable_row_and_one_without_a_mid_is_refused() {
        let row = &rows(
            &[json!({"mid":"msg_x", "ordinal":1, "message":"broken"})],
            false,
        )
        .unwrap()[0];
        assert_eq!(row.display["unreadable"], true);
        assert_eq!(
            rows(&[json!({"ordinal":1, "message":{}})], false)
                .unwrap_err()
                .code,
            "identity_unsupported"
        );
    }
}
