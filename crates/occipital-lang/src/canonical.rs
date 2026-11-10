use serde::ser::Error;
use serde_json::Value;

/// A JSON document whose numbers are kept as their decimal text, so the
/// canonical form is exact without serde_json's global `arbitrary_precision`
/// feature, which this library leaves off so apps embedding it keep their own
/// number decoding. A language value's identity is built straight into this
/// form from its decimals (`value::codec`); a [`Value`] is converted with its
/// numbers as serde_json prints them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Exact {
    Null,
    Bool(bool),
    /// A JSON number token, such as `2.50` or `1e3`.
    Number(String),
    String(String),
    Array(Vec<Exact>),
    Object(Vec<(String, Exact)>),
}

impl From<&Value> for Exact {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(*value),
            Value::Number(value) => Self::Number(value.to_string()),
            Value::String(value) => Self::String(value.clone()),
            Value::Array(values) => Self::Array(values.iter().map(Self::from).collect()),
            Value::Object(values) => Self::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), Self::from(value)))
                    .collect(),
            ),
        }
    }
}

/// Identity/digest JSON (§14.3), not the order-preserving state/event codec.
pub(crate) fn json(value: &Value) -> Result<String, serde_json::Error> {
    exact(&Exact::from(value))
}

/// [`json`] of a document already in exact form.
pub(crate) fn exact(value: &Exact) -> Result<String, serde_json::Error> {
    let mut output = String::new();
    write(value, &mut output)?;
    Ok(output)
}

fn write(value: &Exact, output: &mut String) -> Result<(), serde_json::Error> {
    match value {
        Exact::Null => output.push_str("null"),
        Exact::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Exact::Number(token) => output.push_str(&number(token)?),
        Exact::String(value) => output.push_str(&serde_json::to_string(value)?),
        Exact::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write(value, output)?;
            }
            output.push(']');
        }
        Exact::Object(values) => {
            // UTF-8 lexical order equals Unicode scalar order, unlike UTF-16 order.
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_unstable_by_key(|(key, _)| key);
            output.push('{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key)?);
                output.push(':');
                write(value, output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

fn number(token: &str) -> Result<String, serde_json::Error> {
    let out_of_range =
        || serde_json::Error::custom("catalog number exceeds the §4.2 decimal range");
    let negative = token.starts_with('-');
    let unsigned = token.trim_start_matches('-');
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{integer}{fraction}");
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Ok("0".into());
    }
    // Work on decimal digits, never binary floats, including for large integer ids.
    let exponent: i64 = exponent.parse().map_err(|_| out_of_range())?;
    let leading_zeros = digits.len() - significant.len();
    let point = (integer.len() as i64 - leading_zeros as i64)
        .checked_add(exponent)
        .ok_or_else(out_of_range)?;
    let significant = significant.trim_end_matches('0');
    // §4.2 allows at most 20 integer places and 18 fraction places. Check before
    // expanding exponents so even invalid catalog input cannot request huge padding.
    if !(-18..=20).contains(&point) || significant.len() as i64 - point > 18 {
        return Err(out_of_range());
    }
    let mut output = String::new();
    if negative {
        output.push('-');
    }
    if point <= 0 {
        output.push_str("0.");
        output.extend(std::iter::repeat_n('0', (-point) as usize));
        output.push_str(significant);
    } else {
        let point = point as usize;
        if point >= significant.len() {
            output.push_str(significant);
            output.extend(std::iter::repeat_n('0', point - significant.len()));
        } else {
            output.push_str(&significant[..point]);
            output.push('.');
            output.push_str(&significant[point..]);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{exact, Exact};
    use serde_json::value::RawValue;

    /// `text` as an exact document, every number token as written, whatever
    /// serde_json's number features are.
    fn parse(text: &str) -> Exact {
        let raw: &RawValue = serde_json::from_str(text).unwrap();
        let text = raw.get().trim();
        match text.as_bytes()[0] {
            b'{' => {
                let entries: std::collections::BTreeMap<String, &RawValue> =
                    serde_json::from_str(text).unwrap();
                Exact::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key, parse(value.get())))
                        .collect(),
                )
            }
            b'[' => {
                let items: Vec<&RawValue> = serde_json::from_str(text).unwrap();
                Exact::Array(items.into_iter().map(|item| parse(item.get())).collect())
            }
            b'"' => Exact::String(serde_json::from_str(text).unwrap()),
            b't' => Exact::Bool(true),
            b'f' => Exact::Bool(false),
            b'n' => Exact::Null,
            _ => Exact::Number(text.to_owned()),
        }
    }

    fn json(exact_document: &Exact) -> Result<String, serde_json::Error> {
        exact(exact_document)
    }

    #[derive(serde::Deserialize)]
    struct Vector {
        name: String,
        source: String,
        canonical: String,
    }

    #[test]
    fn canonical_json_matches_hand_derived_vectors() {
        let vectors: Vec<Vector> =
            serde_json::from_str(include_str!("../tools/canonical-json-vectors.json")).unwrap();
        assert_eq!(vectors.len(), 6);
        for vector in vectors {
            let value = parse(&vector.source);
            assert_eq!(json(&value).unwrap(), vector.canonical, "{}", vector.name);
        }
    }

    #[test]
    fn canonical_json_rejects_numbers_outside_decimal_range() {
        for source in ["1e20", "1e-19", "1e999999999999999999999", "-1e20"] {
            let value = parse(source);
            assert!(json(&value).is_err(), "{source}");
        }
    }
}
