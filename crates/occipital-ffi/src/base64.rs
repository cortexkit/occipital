//! Byte strings in JSON envelopes are standard base64 (RFC 4648 §4) with
//! padding. Decoding accepts only the canonical form: a length that is a
//! multiple of four, `=` only as final padding, and zero unused bits, so
//! every byte string has exactly one accepted text.

/// The value of one base64 alphabet byte.
fn sextet(byte: u8) -> Option<u32> {
    Some(u32::from(match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    }))
}

/// The bytes `text` encodes, or `None` when it is not canonical base64.
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let (quads, rest) = text.as_bytes().as_chunks::<4>();
    if !rest.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(quads.len() * 3);
    for (index, quad) in quads.iter().enumerate() {
        let padding = quad.iter().rev().take_while(|&&b| b == b'=').count();
        if padding > 2 || (padding > 0 && index + 1 != quads.len()) {
            return None;
        }
        let mut word = 0u32;
        for &byte in &quad[..4 - padding] {
            word = (word << 6) | sextet(byte)?;
        }
        word <<= 6 * padding as u32;
        let [_, a, b, c] = word.to_be_bytes();
        match padding {
            0 => out.extend_from_slice(&[a, b, c]),
            // The unused low bits of the last sextet must be zero.
            1 if c == 0 => out.extend_from_slice(&[a, b]),
            2 if b == 0 && c == 0 => out.push(a),
            _ => return None,
        }
    }
    Some(out)
}

/// The canonical text of `bytes`. Only tests encode: they write the base64
/// members of the requests recorded in the FFI vectors.
#[cfg(test)]
pub(crate) fn encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let word = group
            .iter()
            .enumerate()
            .fold(0u32, |word, (i, &b)| word | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= group.len() {
                out.push(char::from(ALPHABET[(word >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_round_trips() {
        for bytes in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            &[0xfb, 0xff],
            &[0, 1, 2, 0xff],
        ] {
            assert_eq!(decode(&encode(bytes)).as_deref(), Some(bytes));
        }
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
    }

    #[test]
    fn decodes_canonical_text() {
        for (text, bytes) in [
            ("", &b""[..]),
            ("Zg==", b"f"),
            ("Zm8=", b"fo"),
            ("Zm9v", b"foo"),
            ("Zm9vYg==", b"foob"),
            ("Zm9vYmE=", b"fooba"),
            ("Zm9vYmFy", b"foobar"),
            ("+/8=", &[0xfb, 0xff]),
            ("AAEC/w==", &[0, 1, 2, 0xff]),
        ] {
            assert_eq!(decode(text).as_deref(), Some(bytes), "{text}");
        }
    }

    #[test]
    fn refuses_text_that_is_not_canonical() {
        for text in [
            "Zg", "Zg=", "Zg===", "Z===", "====", "Zh==", "Zm9=", "Zg==Zg==", "Zm9v\n", "Zm-v",
            "Zm_v", " Zm9v", "=Zm9",
        ] {
            assert_eq!(decode(text), None, "{text:?}");
        }
    }
}
