//! A tool call's start and answer times, as a runner serves them beside a read
//! entry's `mid` when the read asks for display facts (llm-runner §4.7.1,
//! `session.read` with `include_display_facts`):
//! `tool_times: [{tool_call_id, started_at?, finished_at?, unanswered?,
//! call_key?, call_carrier?, schema_pin?}]`.
//!
//! The app's tool part carries them as `startedAtMs`, `finishedAtMs`,
//! `unanswered`, `callKey` and `schemaPin`, each only when known. A runner
//! sends the key and pin only when it also declares `call_identity`: `callKey`
//! is the key a provider's display records are filed under, and `schemaPin`
//! names the tool version whose view draws the call.
//!
//! `call_carrier` names another runner that minted the call's key, for a call
//! moved here from that runner with its history. The call's records and
//! notices are filed under that runner and the key, so the part carries it as
//! [`CALL_CARRIER`] for the module to read, which takes it off before the app
//! sees the part. The items stay raw JSON, so a malformed one drops only
//! itself and never the row.

use serde_json::{json, Value};

/// The tool part member that carries a moved call's `call_carrier` from the
/// row builder to the module, which reads it with [`call_identities`] and
/// removes it with [`take_carriers`] before serving the row.
pub const CALL_CARRIER: &str = "callCarrier";

/// The items an entry's `tool_times` member holds, or `None` when the entry
/// carries none: a read without the opt-in, or a runner that recorded nothing.
pub fn items(tool_times: Option<&Value>) -> Option<&[Value]> {
    tool_times
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .filter(|items| !items.is_empty())
}

/// Copy the item for call `call_id` onto an app tool part. A time is copied only
/// as a positive integer, and `unanswered`, `call_key` and `schema_pin` only as
/// non-empty strings, kept as the runner wrote them: anything else is left out,
/// never guessed.
pub fn apply(part: &mut Value, items: &[Value], call_id: &str) {
    let Some(item) = items.iter().find(|item| item["tool_call_id"] == call_id) else {
        return;
    };
    for (served, member) in [
        ("started_at", "startedAtMs"),
        ("finished_at", "finishedAtMs"),
    ] {
        if let Some(at) = item[served].as_u64().filter(|at| *at > 0) {
            part[member] = json!(at);
        }
    }
    for (served, member) in [
        ("unanswered", "unanswered"),
        ("call_key", "callKey"),
        ("call_carrier", CALL_CARRIER),
        ("schema_pin", "schemaPin"),
    ] {
        if let Some(text) = item[served].as_str().filter(|text| !text.is_empty()) {
            part[member] = json!(text);
        }
    }
}

/// A tool call as a page names it: its key, its tool version, and for a call
/// moved from another runner, the runner that minted the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallIdentity {
    pub call_key: String,
    pub schema_pin: String,
    pub carrier: Option<String>,
}

/// The identity of every tool part in a row's display form that carries both
/// a key and a pin, in part order.
pub fn call_identities(display: &Value) -> Vec<CallIdentity> {
    display["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["type"] == "tool")
        .filter_map(|part| {
            Some(CallIdentity {
                call_key: part["callKey"].as_str()?.to_owned(),
                schema_pin: part["schemaPin"].as_str()?.to_owned(),
                carrier: part[CALL_CARRIER].as_str().map(str::to_owned),
            })
        })
        .collect()
}

/// Remove [`CALL_CARRIER`] from every part of a row's display form. The apps
/// ask for a call's records and notices by its key alone, and this module
/// finds the runner they're filed under.
pub fn take_carriers(display: &mut Value) {
    for part in display["parts"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object_mut)
    {
        part.remove(CALL_CARRIER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_identities_come_only_from_tool_parts_carrying_both() {
        let display = json!({"parts": [
            {"type": "tool", "callKey": "ck_1", "schemaPin": "tp1:a:00:1"},
            {"type": "tool", "callKey": "ck_2"},
            {"type": "text", "callKey": "ck_3", "schemaPin": "tp1:b:00:1"},
            {"type": "tool", "callKey": "ck_4", "schemaPin": "tp1:c:00:1"}
        ]});
        let identity = |key: &str, pin: &str, carrier: Option<&str>| CallIdentity {
            call_key: key.into(),
            schema_pin: pin.into(),
            carrier: carrier.map(str::to_owned),
        };
        assert_eq!(
            call_identities(&display),
            [
                identity("ck_1", "tp1:a:00:1", None),
                identity("ck_4", "tp1:c:00:1", None)
            ]
        );
        assert!(call_identities(&json!({})).is_empty());
        let mut moved = json!({"parts": [
            {"type": "tool", "callKey": "ck_1", "schemaPin": "tp1:a:00:1", "callCarrier": "thalamus-pi"},
            {"type": "text", "text": "hi"}
        ]});
        assert_eq!(
            call_identities(&moved),
            [identity("ck_1", "tp1:a:00:1", Some("thalamus-pi"))]
        );
        take_carriers(&mut moved);
        assert_eq!(
            moved,
            json!({"parts": [
                {"type": "tool", "callKey": "ck_1", "schemaPin": "tp1:a:00:1"},
                {"type": "text", "text": "hi"}
            ]})
        );
    }

    #[test]
    fn only_known_facts_for_the_named_call_are_copied() {
        let served = json!([
            {"tool_call_id": "a", "started_at": 10, "finished_at": 25},
            {"tool_call_id": "b", "started_at": 0, "finished_at": "late", "unanswered": "interrupted"},
            {"tool_call_id": "c", "unanswered": "", "call_key": "", "schema_pin": 7},
            {"tool_call_id": "d", "call_key": "ck_1", "schema_pin": "tp1:bash:00:1"},
            {"tool_call_id": "e", "call_key": "ck_2", "call_carrier": "thalamus-pi"},
            "not an item"
        ]);
        let items = items(Some(&served)).unwrap();
        let mut part = json!({"type": "tool"});
        apply(&mut part, items, "a");
        assert_eq!(
            part,
            json!({"type": "tool", "startedAtMs": 10, "finishedAtMs": 25})
        );
        let mut part = json!({"type": "tool"});
        apply(&mut part, items, "b");
        assert_eq!(part, json!({"type": "tool", "unanswered": "interrupted"}));
        let mut part = json!({"type": "tool"});
        apply(&mut part, items, "d");
        assert_eq!(
            part,
            json!({"type": "tool", "callKey": "ck_1", "schemaPin": "tp1:bash:00:1"})
        );
        let mut part = json!({"type": "tool"});
        apply(&mut part, items, "e");
        assert_eq!(
            part,
            json!({"type": "tool", "callKey": "ck_2", "callCarrier": "thalamus-pi"})
        );
        for call in ["c", "missing"] {
            let mut part = json!({"type": "tool"});
            apply(&mut part, items, call);
            assert_eq!(part, json!({"type": "tool"}), "{call}");
        }
        assert!(super::items(None).is_none());
        assert!(super::items(Some(&json!([]))).is_none());
        assert!(super::items(Some(&json!({"a": 1}))).is_none());
    }
}
