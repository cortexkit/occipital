//! Ordered admission for native UI calls. Preparing a call performs no I/O:
//! reference descriptors are metadata, not permission to fetch their records.

use std::collections::BTreeMap;

use occipital_lang::{
    CheckContext, ColumnType, DataFormat, Decimal, HostValues, Profile, ShowUiArguments,
};
use serde_json::{Map, Value};
use subc_protocol::{scope::ScopeStamp, Principal};

const MEMBERS: [&str; 7] = [
    "data",
    "data_columns",
    "data_format",
    "kind",
    "program",
    "summary",
    "title",
];
const REQUIRED: [&str; 4] = ["kind", "program", "summary", "title"];

/// Only daemon-stamped facts cross the native-execution boundary. In particular,
/// retaining no bind identity here prevents its client-supplied fields from
/// accidentally becoming persisted provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub owner: Principal,
    pub scope_ref: String,
    pub scope_epoch: u64,
    pub principal: Principal,
}

#[derive(Clone, Default)]
pub struct RouteStamp {
    pub scope: Option<ScopeStamp>,
    pub principal: Option<Principal>,
}

/// Validated native inputs; reference resolution requires carrier-authorized
/// paging, which is not implemented here.
pub struct PreparedCall {
    pub provenance: Provenance,
    pub source: String,
    pub context: CheckContext,
    pub host: HostValues,
    pub now_ms: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn member(code: &'static str, member: &str) -> Self {
        Self::new(code, format!("{member}: {code}"))
    }
}

fn invalid(member: &str) -> Refusal {
    Refusal::member("argument_invalid", member)
}

fn transcript_number(text: &str) -> Option<u64> {
    let digits = text.strip_prefix("$$tc(")?.strip_suffix(')')?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    digits.parse().ok()
}

/// The call carries a set descriptor, never the records themselves. Nonempty
/// sets require the carrier's paged metadata read, which is not served yet.
fn descriptor_count(raw: &[u8]) -> Result<u64, Refusal> {
    let value: Value = serde_json::from_slice(raw).map_err(|_| invalid("output_refs"))?;
    // Use the producer's compact-encoding cap, not the whitespace in the request.
    let descriptor =
        subc_protocol::tool_call::OutputRefs::new(value).map_err(|_| invalid("output_refs"))?;
    let map = descriptor.as_map();
    for name in ["call_key", "count", "dialect", "digest", "mid", "set_id"] {
        let valid = match name {
            "call_key" => map
                .get(name)
                .is_none_or(|value| value.as_str().is_some_and(|s| !s.is_empty())),
            "count" => map.get(name).and_then(Value::as_u64).is_some(),
            "digest" => map.get(name).and_then(Value::as_str).is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }),
            _ => map
                .get(name)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty()),
        };
        if !valid {
            return Err(invalid(&format!("output_refs.{name}")));
        }
    }
    map.get("count")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("output_refs.count"))
}

pub fn prepare(
    arguments: &Value,
    output_refs: Option<&[u8]>,
    stamp: &RouteStamp,
    now_ms: i64,
    now: Decimal,
) -> Result<PreparedCall, Refusal> {
    // Admission order is observable: a later malformed member must not mask an
    // earlier refusal, even when it lives outside the arguments object.
    if arguments.get("kind").and_then(Value::as_str) == Some("html") {
        return Err(Refusal::new("kind_unsupported", "HTML is not supported"));
    }
    let scope = stamp
        .scope
        .as_ref()
        .ok_or_else(|| Refusal::new("route_unscoped", "show_ui requires a route scope stamp"))?;
    let arguments = arguments.as_object().ok_or_else(|| invalid("arguments"))?;
    let mut names: Vec<_> = arguments.keys().collect();
    names.sort();
    for name in &names {
        if !MEMBERS.contains(&name.as_str()) {
            return Err(Refusal::member("unknown_argument", name));
        }
    }
    for name in REQUIRED {
        if !arguments.contains_key(name) {
            return Err(Refusal::member("argument_missing", name));
        }
    }

    let mut columns = BTreeMap::new();
    let mut count = 0;
    let descriptor_name = "output_refs".to_owned();
    if output_refs.is_some() {
        names.push(&descriptor_name);
        names.sort();
    }
    for name in names {
        if name == "output_refs" {
            if let Some(raw) = output_refs {
                count = descriptor_count(raw)?;
            }
            continue;
        }
        let value = &arguments[name];
        match name.as_str() {
            "data" if value.as_str().and_then(transcript_number).is_some() => (),
            "data_columns" => {
                let object = value.as_object().ok_or_else(|| invalid(name))?;
                for (key, value) in object {
                    let column = match value.as_str() {
                        Some("text") => ColumnType::Text,
                        Some("number") => ColumnType::Number,
                        Some("boolean") => ColumnType::Boolean,
                        _ => return Err(invalid(name)),
                    };
                    columns.insert(key.clone(), column);
                }
                let bytes = serde_json::to_vec(value).map_err(|_| invalid(name))?;
                occipital_registry::canonicalize(&bytes).map_err(|_| invalid(name))?;
            }
            "data_format" if matches!(value.as_str(), Some("json" | "csv")) => (),
            "kind" if value.as_str() == Some("native") => (),
            "program" | "summary" | "title" if value.is_string() => (),
            _ => return Err(invalid(name)),
        }
    }
    for (member, required) in [("data_columns", "data_format"), ("data_format", "data")] {
        if arguments.contains_key(member) && !arguments.contains_key(required) {
            return Err(Refusal::member("argument_forbidden", member));
        }
    }
    if count > 0 {
        return Err(Refusal::new(
            "references_unavailable",
            "Reference records cannot be read until the carrier is available",
        ));
    }
    if arguments.contains_key("data") {
        return Err(Refusal::member("data_reference_unknown", "data"));
    }

    let string = |name: &str| -> String {
        arguments
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let data_format = arguments.get("data_format").and_then(Value::as_str);
    let mut context = CheckContext::new(Profile::ShowUi);
    context.show_ui = ShowUiArguments {
        title: string("title"),
        summary: string("summary"),
        data_format: data_format.map(|value| {
            if value == "csv" {
                DataFormat::Csv
            } else {
                DataFormat::Json
            }
        }),
        data_columns: columns,
    };
    Ok(PreparedCall {
        provenance: Provenance {
            owner: scope.owner.clone(),
            scope_ref: scope.scope_ref.clone(),
            scope_epoch: scope.scope_epoch,
            principal: stamp.principal.clone().unwrap_or(Principal::Unverified),
        },
        source: string("program"),
        context,
        host: HostValues {
            network: false,
            now,
            zone: "UTC".into(),
            data: None,
            snapshots: BTreeMap::new(),
        },
        now_ms,
    })
}

pub fn refusal_content(refusal: Refusal) -> Map<String, Value> {
    Map::from_iter([
        ("schema".into(), Value::from("occipital.show_ui/1")),
        ("outcome".into(), Value::from("refused")),
        ("code".into(), Value::from(refusal.code)),
        ("message".into(), Value::from(refusal.message)),
    ])
}

#[cfg(test)]
pub(crate) mod tests;
