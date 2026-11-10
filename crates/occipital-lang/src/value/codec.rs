use serde_json::{json, Value as Json};

use super::{ScalarString, SourceSpan, Value, ValueBuildError, ValueError, ValueKind, ValueType};
use crate::{canonical, Decimal};

/// Events can carry local errors; saved state cannot (§14.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecMode {
    /// Saved state: errors are not allowed.
    State,
    /// Event values: local errors are allowed.
    Event,
}

/// Unsupported value kinds are rejected explicitly, never assigned guessed tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// The kind has no ordered encoding.
    UnsupportedKind(ValueType),
    /// An error value in saved state.
    ErrorInState,
    /// The JSON is not a tagged ordered value.
    InvalidEncoding,
    /// A number coefficient is not a valid decimal.
    InvalidNumber,
    /// The decoded value breaks a size rule.
    Build(ValueBuildError),
    /// The bytes are not JSON.
    Json(String),
}

impl From<ValueBuildError> for CodecError {
    fn from(value: ValueBuildError) -> Self {
        Self::Build(value)
    }
}

impl From<serde_json::Error> for CodecError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}

impl Value {
    /// Tagged ordered JSON (§14.3). Coefficients are decimal strings, and record
    /// fields are pair arrays, so JSON object ordering cannot affect restoration.
    pub fn encode_ordered(&self, mode: CodecMode) -> Result<String, CodecError> {
        Ok(serde_json::to_string(&self.ordered_json(mode)?)?)
    }

    /// Decodes the tagged ordered JSON of §14.3, keeping record order.
    pub fn decode_ordered(bytes: &[u8], mode: CodecMode) -> Result<Self, CodecError> {
        Self::from_ordered_json(&serde_json::from_slice(bytes)?, mode)
    }

    fn ordered_json(&self, mode: CodecMode) -> Result<Json, CodecError> {
        Ok(match &self.kind {
            ValueKind::Number(value) => json!(["number", value.coefficient().to_string()]),
            ValueKind::String(value) => json!(["string", value.as_str()]),
            ValueKind::Boolean(value) => json!(["boolean", value]),
            ValueKind::Null => json!(["null"]),
            ValueKind::Missing => json!(["missing"]),
            ValueKind::Error(error) => {
                if mode == CodecMode::State {
                    return Err(CodecError::ErrorInState);
                }
                json!(["error", error.code.as_ref(), error.span])
            }
            ValueKind::List(values) => {
                let values: Result<Vec<_>, _> = values
                    .iter()
                    .map(|value| value.ordered_json(mode))
                    .collect();
                json!(["list", values?])
            }
            ValueKind::Record(record) => {
                let fields: Result<Vec<_>, CodecError> = record
                    .fields
                    .iter()
                    .map(|(key, value)| Ok(json!([key.as_str(), value.ordered_json(mode)?])))
                    .collect();
                json!(["record", fields?])
            }
            _ => return Err(CodecError::UnsupportedKind(self.value_type())),
        })
    }

    fn from_ordered_json(json: &Json, mode: CodecMode) -> Result<Self, CodecError> {
        let values = json.as_array().ok_or(CodecError::InvalidEncoding)?;
        let tag = values
            .first()
            .and_then(Json::as_str)
            .ok_or(CodecError::InvalidEncoding)?;
        let arity = |n| {
            if values.len() == n {
                Ok(())
            } else {
                Err(CodecError::InvalidEncoding)
            }
        };
        match tag {
            "number" => {
                arity(2)?;
                let token = values[1].as_str().ok_or(CodecError::InvalidNumber)?;
                let coefficient: i128 = token.parse().map_err(|_| CodecError::InvalidNumber)?;
                if coefficient.to_string() != token {
                    return Err(CodecError::InvalidNumber);
                }
                Ok(Self::number(
                    Decimal::from_coefficient(coefficient)
                        .map_err(|_| CodecError::InvalidNumber)?,
                ))
            }
            "string" => {
                arity(2)?;
                Ok(Self::string(
                    values[1].as_str().ok_or(CodecError::InvalidEncoding)?,
                )?)
            }
            "boolean" => {
                arity(2)?;
                Ok(Self::boolean(
                    values[1].as_bool().ok_or(CodecError::InvalidEncoding)?,
                ))
            }
            "null" => {
                arity(1)?;
                Ok(Self::null())
            }
            "missing" => {
                arity(1)?;
                Ok(Self::missing())
            }
            "error" => {
                if mode == CodecMode::State {
                    return Err(CodecError::ErrorInState);
                }
                arity(3)?;
                let code = values[1].as_str().ok_or(CodecError::InvalidEncoding)?;
                let span: SourceSpan = serde_json::from_value(values[2].clone())?;
                if !span.is_valid() {
                    return Err(CodecError::InvalidEncoding);
                }
                Ok(Self::error(ValueError::new(code, span)))
            }
            "list" => {
                arity(2)?;
                let values = values[1].as_array().ok_or(CodecError::InvalidEncoding)?;
                let elements: Result<Vec<_>, _> = values
                    .iter()
                    .map(|v| Self::from_ordered_json(v, mode))
                    .collect();
                Ok(Self::list(elements?)?)
            }
            "record" => {
                arity(2)?;
                let fields = values[1].as_array().ok_or(CodecError::InvalidEncoding)?;
                let fields: Result<Vec<_>, CodecError> = fields
                    .iter()
                    .map(|field| {
                        let pair = field.as_array().ok_or(CodecError::InvalidEncoding)?;
                        if pair.len() != 2 {
                            return Err(CodecError::InvalidEncoding);
                        }
                        let key = pair[0].as_str().ok_or(CodecError::InvalidEncoding)?;
                        Ok((
                            ScalarString::new(key),
                            Self::from_ordered_json(&pair[1], mode)?,
                        ))
                    })
                    .collect();
                Ok(Self::record(fields?)?)
            }
            "component" => Err(CodecError::UnsupportedKind(ValueType::Component)),
            "action" => Err(CodecError::UnsupportedKind(ValueType::Action)),
            "snapshot" => Err(CodecError::UnsupportedKind(ValueType::Snapshot)),
            "handle" => Err(CodecError::UnsupportedKind(ValueType::Handle)),
            "siteEnvelope" => Err(CodecError::UnsupportedKind(ValueType::SiteEnvelope)),
            _ => Err(CodecError::InvalidEncoding),
        }
    }

    /// Identity JSON is deliberately separate from ordered persistence (§14.3).
    /// The final emitter is shared with catalog digest calculation, including
    /// exact numbers, scalar-sorted keys and minimal string escaping.
    pub fn canonical_json(&self) -> Result<String, CodecError> {
        Ok(canonical::exact(&self.identity_json()?)?)
    }

    /// Numbers go in as their decimal text, never through a binary float.
    fn identity_json(&self) -> Result<canonical::Exact, CodecError> {
        use canonical::Exact;
        Ok(match &self.kind {
            ValueKind::Number(number) => Exact::Number(number.to_string()),
            ValueKind::String(text) => Exact::String(text.as_str().into()),
            ValueKind::Boolean(value) => Exact::Bool(*value),
            ValueKind::Null | ValueKind::Missing => Exact::Null,
            ValueKind::List(values) => Exact::Array(
                values
                    .iter()
                    .map(Self::identity_json)
                    .collect::<Result<_, _>>()?,
            ),
            ValueKind::Record(record) => {
                let fields: Result<_, CodecError> = record
                    .fields
                    .iter()
                    .filter(|(_, value)| value.value_type() != ValueType::Missing)
                    .map(|(key, value)| Ok((key.as_str().into(), value.identity_json()?)))
                    .collect();
                Exact::Object(fields?)
            }
            _ => return Err(CodecError::UnsupportedKind(self.value_type())),
        })
    }
}
