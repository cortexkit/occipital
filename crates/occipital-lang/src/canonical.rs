use serde::ser::Error;
use serde_json::Value;

/// Identity/digest JSON (§14.3), not the order-preserving state/event codec.
pub(crate) fn json(value: &Value) -> Result<String, serde_json::Error> {
    let mut output = String::new();
    write(value, &mut output)?;
    Ok(output)
}

fn write(value: &Value, output: &mut String) -> Result<(), serde_json::Error> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&number(&value.to_string())?),
        Value::String(value) => output.push_str(&serde_json::to_string(value)?),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write(value, output)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            // UTF-8 lexical order equals Unicode scalar order, unlike UTF-16 order.
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_unstable_by_key(|(key, _)| *key);
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
    use super::json;

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
            let value = serde_json::from_str(&vector.source).unwrap();
            assert_eq!(json(&value).unwrap(), vector.canonical, "{}", vector.name);
        }
    }

    #[test]
    fn canonical_json_rejects_numbers_outside_decimal_range() {
        for source in ["1e20", "1e-19", "1e999999999999999999999", "-1e20"] {
            let value = serde_json::from_str(source).unwrap();
            assert!(json(&value).is_err(), "{source}");
        }
    }
}
