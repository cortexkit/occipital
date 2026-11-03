//! Standard base64 with padding (RFC 4648 §4), the encoding the runner contract
//! uses for file bytes: in `session.upload_append`, which this module sends, and
//! in `session.media_read` replies, which it reads.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let word = (u32::from(group[0]) << 16)
            | (u32::from(*group.get(1).unwrap_or(&0)) << 8)
            | u32::from(*group.get(2).unwrap_or(&0));
        for position in 0..4 {
            if position <= group.len() {
                out.push(char::from(
                    ALPHABET[(word >> (18 - 6 * position) & 63) as usize],
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The bytes `text` encodes, or `None` unless it is canonical padded base64:
/// a multiple of four characters from the standard alphabet, `=` only as the
/// last one or two, and zero bits wherever padding leaves unused ones. A runner
/// sends exactly that, so anything else is a malformed reply.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let text = text.as_bytes();
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for (index, group) in text.chunks(4).enumerate() {
        let last = (index + 1) * 4 == text.len();
        let padding = group.iter().rev().take_while(|byte| **byte == b'=').count();
        if padding > 2 || (padding > 0 && !last) {
            return None;
        }
        let mut word = 0u32;
        for (position, byte) in group[..4 - padding].iter().enumerate() {
            let value = ALPHABET.iter().position(|known| known == byte)? as u32;
            word |= value << (18 - 6 * position);
        }
        let bytes = word.to_be_bytes();
        let kept = 3 - padding;
        // Bits a shorter final group leaves over must be zero, so each byte
        // string has one encoding.
        if bytes[1 + kept..].iter().any(|byte| *byte != 0) {
            return None;
        }
        out.extend_from_slice(&bytes[1..1 + kept]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rfc_4648_vectors_encode_and_decode() {
        for (bytes, text) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(bytes), text);
            assert_eq!(decode(text).as_deref(), Some(bytes));
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)), Some(all));
    }

    #[test]
    fn anything_but_canonical_padded_base64_is_refused() {
        for text in [
            "Zg", "Zg=", "Zm9v=", "Z===", "Zg==Zm8=", "Zh==", "Zm9=", "Zm-v", "Zm9v\n",
        ] {
            assert_eq!(decode(text), None, "{text:?}");
        }
    }
}
