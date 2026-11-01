//! The RFC 8785 serializer. Every canonical byte the crate produces, whether
//! from parsed input or from a typed object built in code, comes from here.

use super::value::Value;

/// Serializes `value` as RFC 8785 canonical JSON: no whitespace, object
/// members in UTF-16 key order, integers in plain decimal, and strings
/// escaped as §3.2.2.2 states.
pub(super) fn write_value(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        // Integers within ±(2^53 − 1) print the same as ECMAScript's
        // Number-to-string, which RFC 8785 §3.2.2.3 specifies.
        Value::Integer(integer) => out.extend_from_slice(integer.get().to_string().as_bytes()),
        Value::String(string) => write_string(string, out),
        Value::Array(elements) => {
            out.push(b'[');
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_value(element, out);
            }
            out.push(b']');
        }
        Value::Object(object) => {
            out.push(b'{');
            for (index, (key, member)) in object.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_string(key, out);
                out.push(b':');
                write_value(member, out);
            }
            out.push(b'}');
        }
    }
}

/// Writes a string as RFC 8785 §3.2.2.2 states: `"` and `\` escaped, the five
/// controls with short forms written `\b \t \n \f \r`, every other control
/// below U+0020 written `\u00xx` in lowercase hex, and everything else,
/// including U+007F, `/`, U+2028 and U+2029, written as raw UTF-8.
fn write_string(string: &str, out: &mut Vec<u8>) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    for &byte in string.as_bytes() {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x09 => out.extend_from_slice(b"\\t"),
            0x0a => out.extend_from_slice(b"\\n"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x0d => out.extend_from_slice(b"\\r"),
            0x00..=0x1f => {
                out.extend_from_slice(b"\\u00");
                out.push(HEX[usize::from(byte >> 4)]);
                out.push(HEX[usize::from(byte & 0x0f)]);
            }
            // Bytes of multi-byte UTF-8 sequences are all 0x80 or above, so
            // copying them one by one keeps each character intact.
            _ => out.push(byte),
        }
    }
    out.push(b'"');
}
