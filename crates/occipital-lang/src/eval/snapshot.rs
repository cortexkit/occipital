//! Snapshot values (§4.8) and the prepared results `Parse` reads (§10.2).

use cortexkit_rich_ui_syntax::{Expression as E, Kind, Node};

use crate::data::{ColumnType, Delimiter, Format, Options, Prepared};
use crate::prepare::resolve::Resolved;
use crate::{Decimal, ReferenceRecord, ReferenceState, ScalarString, Snapshot, Value, ValueKind};

/// Where a parsed snapshot comes from in the source: the `show_ui` call's
/// `data`, or the reference `$$tc(N)` with N as written.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SnapshotSource {
    Data,
    Reference(String),
}

/// One distinct (snapshot, format, options) of §10.2. Every key is known
/// from the source alone, so the host prepares each one once per load.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ParseKey {
    pub source: SnapshotSource,
    pub format: Format,
    pub options: Options,
}

/// The snapshot value for a stored record (§4.8, §10.1).
///
/// `bytes` are the fetched output; `text` exists only when the record is
/// resolved and the bytes are present. `value` and `rows` exist only with
/// a format hint, given here as the prepared parse. `reason` exists only
/// for an unresolved record.
pub(crate) fn snapshot(
    record_id: &str,
    record: &ReferenceRecord,
    bytes: Option<&[u8]>,
    prepared: Option<&Prepared>,
) -> Value {
    let resolved = record.state == ReferenceState::Resolved;
    let text = bytes
        .filter(|_| resolved)
        .map(|bytes| host_string(&crate::data::snapshot_text(bytes)));
    let state = match record.state {
        ReferenceState::Resolved => "resolved",
        ReferenceState::Unresolved => "unresolved",
        ReferenceState::TooLarge => "too_large",
        ReferenceState::Held => "held",
    };
    let reason = match record.state {
        ReferenceState::Unresolved => record.reason.as_deref().map(host_string),
        _ => None,
    };
    let parsed = prepared.filter(|_| text.is_some());
    members(
        record_id,
        Members {
            text,
            value: parsed.map(|prepared| prepared.value.clone()),
            rows: parsed.map(|prepared| prepared.rows.clone()),
            state: host_string(state),
            reason,
            tool: record.tool.as_deref().map(host_string),
            bytes: record
                .bytes
                .map(|bytes| Value::number(Decimal::from_u64(bytes))),
        },
    )
}

/// A reference with no stored record: unresolved, with nothing to read.
pub(super) fn unresolved(n: &str) -> Value {
    members(
        n,
        Members {
            text: None,
            value: None,
            rows: None,
            state: host_string("unresolved"),
            reason: None,
            tool: None,
            bytes: None,
        },
    )
}

/// `Parse` returns the snapshot with the same `text` and with `value` and
/// `rows` from the prepared result (§10.2).
pub(super) fn with_prepared(snapshot: &Snapshot, prepared: &Prepared) -> Value {
    let get = |key: &str| Some(snapshot.members.get(key)).filter(|value| !is_missing(value));
    let text = get("text");
    let resolved = text.is_some();
    members(
        &snapshot.record_id,
        Members {
            value: resolved.then(|| prepared.value.clone()),
            rows: resolved.then(|| prepared.rows.clone()),
            text,
            state: snapshot.members.get("state"),
            reason: get("reason"),
            tool: get("tool"),
            bytes: get("bytes"),
        },
    )
}

struct Members {
    text: Option<Value>,
    value: Option<Value>,
    rows: Option<Value>,
    state: Value,
    reason: Option<Value>,
    tool: Option<Value>,
    bytes: Option<Value>,
}

/// Members in the order of the §4.8 table; an absent member is missing.
fn members(record_id: &str, members: Members) -> Value {
    let fields = [
        ("text", members.text),
        ("value", members.value),
        ("rows", members.rows),
        ("state", Some(members.state)),
        ("reason", members.reason),
        ("tool", members.tool),
        ("bytes", members.bytes),
    ]
    .into_iter()
    .filter_map(|(key, value)| Some((ScalarString::new(key), value?)))
    .collect();
    Value::snapshot(Snapshot {
        record_id: record_id.into(),
        members: crate::Record::members(fields),
    })
}

fn is_missing(value: &Value) -> bool {
    matches!(value.kind(), ValueKind::Missing)
}

/// Host strings may be longer than a created string (§4.3); they are only
/// bounded by the data limits of §10.
fn host_string(text: &str) -> Value {
    Value::text(text)
}

/// The key of one `Parse` call. Its format and options are literals (§10.2);
/// anything else is the check error `E_LITERAL_REQUIRED`, and an unknown
/// format, option or option value is `E_ARG`.
pub(crate) fn parse_key(call: &Node) -> Result<ParseKey, &'static str> {
    let source = match &call.children[0].kind {
        Kind::Reference { n } => SnapshotSource::Reference(n.clone()),
        Kind::Expression(E::Name(name)) if name == "data" => SnapshotSource::Data,
        _ => return Err("E_LITERAL_REQUIRED"),
    };
    let format = match &call.children[1].kind {
        Kind::Expression(E::String(format)) if format == "json" => Format::Json,
        Kind::Expression(E::String(format)) if format == "csv" => Format::Csv,
        Kind::Expression(E::String(_)) => return Err("E_ARG"),
        _ => return Err("E_LITERAL_REQUIRED"),
    };
    let mut options = Options::default();
    if let Some(node) = call.children.get(2) {
        let Kind::Expression(E::Record(keys)) = &node.kind else {
            return Err("E_LITERAL_REQUIRED");
        };
        for (key, value) in keys.iter().zip(&node.children) {
            match (key.as_str(), &value.kind) {
                ("columns", Kind::Expression(E::Record(columns))) => {
                    for (column, kind) in columns.iter().zip(&value.children) {
                        let kind = match &kind.kind {
                            Kind::Expression(E::String(kind)) => match kind.as_str() {
                                "text" => ColumnType::Text,
                                "number" => ColumnType::Number,
                                "boolean" => ColumnType::Boolean,
                                _ => return Err("E_ARG"),
                            },
                            _ => return Err("E_LITERAL_REQUIRED"),
                        };
                        options.columns.push((column.clone(), kind));
                    }
                }
                ("delimiter", Kind::Expression(E::String(delimiter))) if format == Format::Csv => {
                    options.delimiter = match delimiter.as_str() {
                        "," => Delimiter::Comma,
                        ";" => Delimiter::Semicolon,
                        "\t" => Delimiter::Tab,
                        "|" => Delimiter::Pipe,
                        _ => return Err("E_ARG"),
                    };
                }
                ("header", Kind::Expression(E::Boolean(header))) if format == Format::Csv => {
                    options.header = *header;
                }
                (_, Kind::Expression(E::Number(_) | E::String(_) | E::Boolean(_) | E::Null))
                | (_, Kind::Expression(E::List | E::Record(_))) => return Err("E_ARG"),
                _ => return Err("E_LITERAL_REQUIRED"),
            }
        }
    }
    Ok(ParseKey {
        source,
        format,
        options,
    })
}

/// Every distinct well-formed `Parse` in the source, in source order. The
/// host prepares exactly these before evaluating (§10.2).
pub(crate) fn parse_keys(program: &Resolved) -> Vec<ParseKey> {
    fn visit(node: &Node, keys: &mut Vec<ParseKey>) {
        if let Kind::Expression(E::Call(name)) = &node.kind {
            if name == "Parse" && matches!(node.children.len(), 2 | 3) {
                if let Ok(key) = parse_key(node) {
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
            }
        }
        if let Kind::Tag { props, .. } = &node.kind {
            for prop in props {
                visit(&prop.value, keys);
            }
        }
        for child in &node.children {
            visit(child, keys);
        }
    }
    let mut keys = Vec::new();
    for node in &program.parsed.document.nodes {
        visit(node, &mut keys);
    }
    keys
}
