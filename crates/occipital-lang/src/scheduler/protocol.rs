use crate::{Schema, SourceSpan, Value, ValueKind};

pub(super) fn decode(
    accounts: &crate::accounts::Accounts,
    bytes: &[u8],
    cap: usize,
) -> Result<Value, String> {
    if bytes.len() > cap {
        return Err("too_large".into());
    }
    let parsed = crate::data::prepare_accounted(
        accounts,
        crate::accounts::UnitKind::AnswerConversion,
        bytes,
        crate::data::Format::Json,
        &Default::default(),
        SourceSpan {
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
        },
    )
    .map_err(|_| "budget".to_owned())?;
    if matches!(parsed.value.kind(), ValueKind::Error(_)) {
        return Err("invalid_page".into());
    }
    Ok(parsed.value)
}

pub(super) fn accepts(schema: &Schema, value: &Value) -> bool {
    value
        .canonical_json()
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .is_some_and(|v| schema.accepts(&v))
}

pub(super) fn matches(query: &super::query::Query, epoch: super::Epoch) -> bool {
    query.request.as_ref().is_some_and(|(_, e)| *e == epoch)
}
