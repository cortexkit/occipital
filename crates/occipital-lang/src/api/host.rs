//! Provisional host values and their preparation into evaluation inputs.
//!
//! These types model the host-provided values this crate converts into
//! evaluation inputs (§2.3, §4.8, §10.2). The spec marks the underlying host
//! contracts as provisional, so these shapes will change with them.

use std::collections::BTreeMap;

use serde_json::Value as Json;

use crate::accounts::{Accounts, Refused, UnitKind};
use crate::data::{self, Format, Options};
use crate::eval::snapshot::{self, ParseKey, SnapshotSource};
use crate::eval::Inputs;
use crate::prepare::resolve::Resolved;
use crate::{
    CheckContext, ColumnType, DataFormat, Decimal, Diagnostic, HandleRecord, Profile,
    ReferenceRecord, ReferenceState, ScalarString, ShowUiArguments, SourceSpan, ToolInputs, Value,
};

/// The values a host supplies at load besides the source and its check
/// context: the recorded network policy, the clock, and fetched snapshot
/// bytes. `input`/`view` come from the check context's tool projection and
/// reference records from its stamped records.
#[derive(Clone, Debug)]
pub struct HostValues {
    /// The recorded network policy. When false, address activation is
    /// refused (§7.5).
    pub network: bool,
    /// `now`: milliseconds since the Unix epoch, an explicit host input
    /// rather than a clock read (§19 item 11).
    pub now: Decimal,
    /// `zone`: the IANA time zone name formatting functions use (§6.4).
    pub zone: String,
    /// The `show_ui` call's `data` snapshot, read as `data` (§4.8). Other
    /// profiles ignore it.
    pub data: Option<DataSnapshot>,
    /// Fetched bytes for `$$tc(N)` references, keyed by N. A reference whose
    /// record is resolved but whose bytes are absent shows as loading.
    pub snapshots: BTreeMap<u64, Vec<u8>>,
}

impl Default for HostValues {
    fn default() -> Self {
        Self {
            network: true,
            now: Decimal::ZERO,
            zone: "UTC".into(),
            data: None,
            snapshots: BTreeMap::new(),
        }
    }
}

/// The `show_ui` call's `data` argument: its stamped record and, when
/// fetched, its bytes. The call's `data_format` and `data_columns` hint how
/// the bytes are parsed (§10.2).
#[derive(Clone, Debug)]
pub struct DataSnapshot {
    /// The record id the tree names this snapshot by.
    pub record_id: String,
    /// The record stamped when the call was stored (§4.8).
    pub record: ReferenceRecord,
    /// The fetched snapshot bytes, if any.
    pub bytes: Option<Vec<u8>>,
}

/// Why the host's request could not be served. In each case the host shows
/// the program's fallback; no effect has run (§13.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The program has error diagnostics, so it never runs (§13.4). They are
    /// the check's own diagnostics, as `check_program` would report them.
    Rejected(Vec<Diagnostic>),
    /// An account refused the work before it started (§9.1).
    Refused(Refused),
    /// A host value cannot become a language value, for example a
    /// projection string longer than the string limit (§9.4). It names the
    /// host value.
    InvalidHostValue(&'static str),
    /// A broken internal invariant, reported instead of a panic.
    Internal(&'static str),
}

impl From<Refused> for HostError {
    fn from(refused: Refused) -> Self {
        Self::Refused(refused)
    }
}

/// A host JSON value as a language value. Object keys arrive sorted (the
/// JSON map is ordered by key), so the record order is the same on every
/// run.
pub(crate) fn json_value(json: &Json) -> Result<Value, HostError> {
    let invalid = |_| HostError::InvalidHostValue("input or view");
    Ok(match json {
        Json::Null => Value::null(),
        Json::Bool(value) => Value::boolean(*value),
        Json::Number(number) => Value::number(
            Decimal::from_literal(&number.to_string())
                .map_err(|_| HostError::InvalidHostValue("input or view"))?,
        ),
        Json::String(text) => Value::string(text.as_str()).map_err(invalid)?,
        Json::Array(items) => Value::list(
            items
                .iter()
                .map(json_value)
                .collect::<Result<Vec<_>, _>>()?,
        )
        .map_err(invalid)?,
        Json::Object(fields) => Value::record(
            fields
                .iter()
                .map(|(key, value)| Ok((ScalarString::new(key.as_str()), json_value(value)?)))
                .collect::<Result<Vec<_>, HostError>>()?,
        )
        .map_err(invalid)?,
    })
}

/// What a load needs from the check context and the checked program, kept
/// by a session so each load prepares its inputs the same way.
pub(super) struct HostContext {
    profile: Profile,
    tool: Option<ToolInputs>,
    show_ui: ShowUiArguments,
    references: BTreeMap<u64, ReferenceRecord>,
    handles: Vec<HandleRecord>,
    /// Each distinct `Parse` call and its source position (§10.2).
    parse_keys: Vec<(ParseKey, SourceSpan)>,
}

impl HostContext {
    pub(super) fn new(context: &CheckContext, program: &Resolved) -> Self {
        Self {
            profile: context.profile,
            tool: context.tool.clone(),
            show_ui: context.show_ui.clone(),
            references: context.references.clone(),
            handles: context.handles.clone(),
            parse_keys: snapshot::parse_keys(program)
                .into_iter()
                .filter_map(|key| {
                    let span = crate::reply::parse_span(program, &key)?;
                    Some((key, span))
                })
                .collect(),
        }
    }

    /// Evaluation inputs for one load. Snapshot parsing is charged to the
    /// data preparation account of `accounts` (§9.1, §10.2).
    pub(super) fn inputs(
        &self,
        host: &HostValues,
        accounts: &Accounts,
    ) -> Result<Inputs, HostError> {
        let mut inputs = Inputs::new(self.profile);
        inputs.network = host.network;
        inputs.now = host.now;
        inputs.zone =
            Value::string(host.zone.as_str()).map_err(|_| HostError::InvalidHostValue("zone"))?;
        if let Some(tool) = self
            .tool
            .as_ref()
            .filter(|_| self.profile.supplies("input"))
        {
            inputs.input = json_value(&tool.input.value)?;
            inputs.view = json_value(&tool.view.value)?;
        }
        inputs.handles = self.handles.clone();
        for (n, record) in &self.references {
            let bytes = host.snapshots.get(n).map(Vec::as_slice);
            inputs.references.insert(
                n.to_string(),
                snapshot::snapshot(&n.to_string(), record, bytes, None),
            );
        }
        let data = host.data.as_ref().filter(|_| self.profile.supplies("data"));
        if let Some(data) = data {
            let prepared = match (self.show_ui.data_format, resolved_bytes(data)) {
                (Some(format), Some(bytes)) => Some(prepare(
                    accounts,
                    bytes,
                    match format {
                        DataFormat::Json => Format::Json,
                        DataFormat::Csv => Format::Csv,
                    },
                    &self.data_options(),
                    // A format hint has no source position of its own; its
                    // diagnostics point into the snapshot (§12.1).
                    SourceSpan {
                        line: 1,
                        column: 1,
                        end_line: 1,
                        end_column: 1,
                    },
                )?),
                _ => None,
            };
            inputs.data = snapshot::snapshot(
                &data.record_id,
                &data.record,
                data.bytes.as_deref(),
                prepared.as_ref(),
            );
        }
        for (key, span) in &self.parse_keys {
            let bytes = match &key.source {
                SnapshotSource::Data => data.and_then(resolved_bytes),
                SnapshotSource::Reference(n) => n
                    .parse::<u64>()
                    .ok()
                    .filter(|n| {
                        self.references
                            .get(n)
                            .is_some_and(|record| record.state == ReferenceState::Resolved)
                    })
                    .and_then(|n| host.snapshots.get(&n))
                    .map(Vec::as_slice),
            };
            if let Some(bytes) = bytes {
                let prepared = prepare(accounts, bytes, key.format, &key.options, *span)?;
                inputs.prepared.insert(key.clone(), prepared);
            }
        }
        Ok(inputs)
    }

    /// The `show_ui` call's `data_columns`, as the conversions of §10.2.
    fn data_options(&self) -> Options {
        Options {
            columns: self
                .show_ui
                .data_columns
                .iter()
                .map(|(name, kind)| {
                    let kind = match kind {
                        ColumnType::Text => data::ColumnType::Text,
                        ColumnType::Number => data::ColumnType::Number,
                        ColumnType::Boolean => data::ColumnType::Boolean,
                    };
                    (name.clone(), kind)
                })
                .collect(),
            ..Options::default()
        }
    }
}

/// Only a resolved record's bytes are parsed (§4.8); other states keep their
/// own snapshot value.
fn resolved_bytes(data: &DataSnapshot) -> Option<&[u8]> {
    data.bytes
        .as_deref()
        .filter(|_| data.record.state == ReferenceState::Resolved)
}

fn prepare(
    accounts: &Accounts,
    bytes: &[u8],
    format: Format,
    options: &Options,
    span: SourceSpan,
) -> Result<data::Prepared, HostError> {
    Ok(data::prepare_accounted(
        accounts,
        UnitKind::DataPreparation,
        bytes,
        format,
        options,
        span,
    )?)
}
