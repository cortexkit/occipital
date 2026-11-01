//! Pi journal entries projected by the runner contract for app reads.

use cortexkit_role_llm_runner::display::{self, Limits};
use occipital_app_reader::source::{ReadError, Row};
use serde_json::Value;

// Match the phone's tool payload bounds; exact message reads keep the full payload.
const APP_LIMITS: Limits = Limits::new(2_048, 16_384, 65_536);

pub(super) fn rows(entries: &[Value], full: bool) -> Result<Vec<Row>, ReadError> {
    entries
        .iter()
        .map(|entry| {
            let mid = entry["mid"]
                .as_str()
                .filter(|mid| !mid.is_empty())
                .ok_or_else(|| ReadError::new("identity_unsupported", "Pi entry has no mid"))?;
            let ordinal = match entry.get("ordinal") {
                None | Some(Value::Null) => None,
                Some(value) => Some(value.as_u64().ok_or_else(|| {
                    ReadError::new(
                        "identity_unsupported",
                        format!("Pi entry {mid} has invalid ordinal {value}; expected u64"),
                    )
                })?),
            };
            let row = display::pi(&entry["message"], (!full).then_some(&APP_LIMITS))
                .map_err(|error| ReadError::new("provider_invalid_response", error.to_string()))?;
            Ok(Row {
                mid: mid.into(),
                ordinal,
                written_at: entry["written_at"].as_i64(),
                display: serde_json::to_value(row).map_err(|error| {
                    ReadError::new("internal", format!("cannot encode a Pi row: {error}"))
                })?,
            })
        })
        .collect()
}
