//! A byte-level RFC 8259 parser restricted to the registry's JSON profile.
//!
//! It reads the input once from left to right, validating UTF-8 as it goes,
//! and stops at the first fault it meets. A number token is read to its end
//! before its grammar is checked, but it holds only ASCII bytes and cannot
//! contain another kind of fault. Everything else is judged byte by byte, so
//! the fault reported is always the one at the lowest offset.

use std::collections::HashSet;

use super::value::{Integer, Object, Value};
use super::{CanonError, CanonErrorKind, MAX_DEPTH};

/// Parses a whole document: one value, optionally surrounded by whitespace.
pub(super) fn parse_document(input: &[u8]) -> Result<Value, CanonError> {
    let mut parser = Parser { input, pos: 0 };
    parser.skip_whitespace();
    let value = parser.parse_value(0)?;
    parser.skip_whitespace();
    if parser.pos < input.len() {
        return Err(parser.syntax_error(parser.pos));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    /// The fault for a byte the grammar cannot accept. When that byte also
    /// starts an ill-formed UTF-8 sequence, the encoding fault is reported
    /// instead: at one offset `invalid_utf8` wins over `json_syntax`.
    fn syntax_error(&self, offset: usize) -> CanonError {
        let kind = if offset < self.input.len() && utf8_sequence_len(self.input, offset).is_none() {
            CanonErrorKind::InvalidUtf8
        } else {
            CanonErrorKind::JsonSyntax
        };
        CanonError::new(kind, offset)
    }

    /// Parses one value at the current position. `depth` is the number of
    /// arrays and objects already open around it.
    fn parse_value(&mut self, depth: usize) -> Result<Value, CanonError> {
        match self.peek() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => self.parse_string().map(Value::String),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(b't') => self.parse_literal(b"true", Value::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", Value::Bool(false)),
            Some(b'n') => self.parse_literal(b"null", Value::Null),
            _ => Err(self.syntax_error(self.pos)),
        }
    }

    fn parse_literal(&mut self, literal: &[u8], value: Value) -> Result<Value, CanonError> {
        for &expected in literal {
            if self.peek() != Some(expected) {
                return Err(self.syntax_error(self.pos));
            }
            self.pos += 1;
        }
        Ok(value)
    }

    /// Refuses a container that would open deeper than [`MAX_DEPTH`], at its
    /// opening bracket and before anything inside it is read.
    fn open_container(&mut self, depth: usize) -> Result<usize, CanonError> {
        if depth >= MAX_DEPTH {
            return Err(CanonError::new(CanonErrorKind::DepthExceeded, self.pos));
        }
        self.pos += 1;
        Ok(depth + 1)
    }

    fn parse_array(&mut self, depth: usize) -> Result<Value, CanonError> {
        let depth = self.open_container(depth)?;
        let mut elements = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(elements));
        }
        loop {
            self.skip_whitespace();
            elements.push(self.parse_value(depth)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(elements));
                }
                _ => return Err(self.syntax_error(self.pos)),
            }
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Value, CanonError> {
        let depth = self.open_container(depth)?;
        let mut members = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(Object::new()));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.syntax_error(self.pos));
            }
            let key_offset = self.pos;
            let key = self.parse_string()?;
            // Keys are compared after their escapes are decoded, so `"a"` and
            // `"\u0061"` collide. The check runs as soon as the key is read,
            // before anything after it, because its offset is the second
            // key's opening quote.
            if !seen.insert(key.clone()) {
                return Err(CanonError::new(CanonErrorKind::DuplicateKey, key_offset));
            }
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.syntax_error(self.pos));
            }
            self.pos += 1;
            self.skip_whitespace();
            let value = self.parse_value(depth)?;
            members.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(Object::from_unique_members(members)));
                }
                _ => return Err(self.syntax_error(self.pos)),
            }
        }
    }

    /// Parses a string starting at its opening quote and returns it decoded.
    fn parse_string(&mut self) -> Result<String, CanonError> {
        self.pos += 1;
        let mut decoded = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.syntax_error(self.pos));
            };
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(decoded);
                }
                b'\\' => decoded.push(self.parse_escape()?),
                // RFC 8259 requires controls inside strings to be escaped.
                0x00..=0x1f => return Err(self.syntax_error(self.pos)),
                0x20..=0x7f => {
                    decoded.push(char::from(byte));
                    self.pos += 1;
                }
                _ => {
                    let start = self.pos;
                    let Some(len) = utf8_sequence_len(self.input, start) else {
                        return Err(CanonError::new(CanonErrorKind::InvalidUtf8, start));
                    };
                    let end = start + len;
                    let text = self
                        .input
                        .get(start..end)
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                        .ok_or(CanonError::new(CanonErrorKind::InvalidUtf8, start))?;
                    decoded.push_str(text);
                    self.pos = end;
                }
            }
        }
    }

    /// Parses one escape starting at its backslash and returns the character
    /// it stands for. A surrogate escape must be a high surrogate followed
    /// directly by a low one; anything else is a lone surrogate, reported at
    /// the backslash of the escape that cannot be paired.
    fn parse_escape(&mut self) -> Result<char, CanonError> {
        let backslash = self.pos;
        self.pos += 1;
        let Some(byte) = self.peek() else {
            return Err(self.syntax_error(self.pos));
        };
        let simple = match byte {
            b'"' => Some('"'),
            b'\\' => Some('\\'),
            b'/' => Some('/'),
            b'b' => Some('\u{08}'),
            b'f' => Some('\u{0c}'),
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            b'u' => None,
            _ => return Err(self.syntax_error(self.pos)),
        };
        self.pos += 1;
        if let Some(character) = simple {
            return Ok(character);
        }
        let unit = self.parse_hex4()?;
        match unit {
            0xd800..=0xdbff => {
                let low = self.parse_low_surrogate();
                match low {
                    Some(low) => {
                        let code = 0x10000
                            + ((u32::from(unit) - 0xd800) << 10)
                            + (u32::from(low) - 0xdc00);
                        char::from_u32(code)
                            .ok_or(CanonError::new(CanonErrorKind::LoneSurrogate, backslash))
                    }
                    None => Err(CanonError::new(CanonErrorKind::LoneSurrogate, backslash)),
                }
            }
            0xdc00..=0xdfff => Err(CanonError::new(CanonErrorKind::LoneSurrogate, backslash)),
            _ => char::from_u32(u32::from(unit))
                .ok_or(CanonError::new(CanonErrorKind::LoneSurrogate, backslash)),
        }
    }

    /// Reads the four hex digits of a `\u` escape.
    fn parse_hex4(&mut self) -> Result<u16, CanonError> {
        let mut unit: u16 = 0;
        for _ in 0..4 {
            let digit = self
                .peek()
                .and_then(|byte| char::from(byte).to_digit(16))
                .ok_or_else(|| self.syntax_error(self.pos))?;
            // `to_digit(16)` returns at most 15, so this always fits.
            unit = (unit << 4) | u16::try_from(digit).unwrap_or(0);
            self.pos += 1;
        }
        Ok(unit)
    }

    /// Consumes a `\uDC00`–`\uDFFF` escape at the current position and
    /// returns its code unit. When the next bytes are anything else, nothing
    /// is consumed and the preceding high surrogate is unpaired. Any fault in
    /// those bytes lies after that high surrogate's backslash, so the lone
    /// surrogate is the fault at the lowest offset.
    fn parse_low_surrogate(&mut self) -> Option<u16> {
        let rest = self.input.get(self.pos..)?;
        let escape = rest.get(..6)?;
        if escape.first() != Some(&b'\\') || escape.get(1) != Some(&b'u') {
            return None;
        }
        let mut unit: u16 = 0;
        for &byte in escape.get(2..)? {
            let digit = char::from(byte).to_digit(16)?;
            unit = (unit << 4) | u16::try_from(digit).ok()?;
        }
        if (0xdc00..=0xdfff).contains(&unit) {
            self.pos += 6;
            Some(unit)
        } else {
            None
        }
    }

    /// Parses a number token. The token is the longest run of bytes that can
    /// appear in a number, so `-01` is one token. A token that is not a valid
    /// RFC 8259 number is `json_syntax` at the first byte the grammar cannot
    /// accept. A valid token must then be an integer written
    /// `-?(0|[1-9][0-9]*)`, other than `-0`, within ±(2^53 − 1); those faults
    /// are reported at the token's first byte.
    fn parse_number(&mut self) -> Result<Value, CanonError> {
        let start = self.pos;
        let mut end = start;
        while matches!(
            self.input.get(end),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            end += 1;
        }
        let token = self.input.get(start..end).unwrap_or_default();
        let shape = number_shape(token).map_err(|index| self.syntax_error(start + index))?;
        let not_integer = CanonError::new(CanonErrorKind::NumberNotInteger, start);
        if shape.fraction_or_exponent || token == b"-0" {
            return Err(not_integer);
        }
        let (negative, digits) = match token.split_first() {
            Some((b'-', digits)) => (true, digits),
            _ => (false, token),
        };
        let out_of_range = CanonError::new(CanonErrorKind::NumberOutOfRange, start);
        let mut magnitude: i64 = 0;
        for &digit in digits {
            magnitude = magnitude
                .checked_mul(10)
                .and_then(|value| value.checked_add(i64::from(digit - b'0')))
                .ok_or(out_of_range)?;
        }
        let value = if negative { -magnitude } else { magnitude };
        let integer = Integer::new(value).ok_or(out_of_range)?;
        self.pos = end;
        Ok(Value::Integer(integer))
    }
}

/// What a valid RFC 8259 number token contains beyond its integer part.
struct NumberShape {
    fraction_or_exponent: bool,
}

/// Checks `token` against RFC 8259's number grammar
/// `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`. On failure it
/// returns the index of the first byte the grammar cannot accept, which is
/// `token.len()` when the token ends too early.
fn number_shape(token: &[u8]) -> Result<NumberShape, usize> {
    let at = |index: usize| token.get(index).copied();
    let is_digit = |index: usize| matches!(at(index), Some(b'0'..=b'9'));
    let mut index = 0;
    if at(index) == Some(b'-') {
        index += 1;
    }
    match at(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => {
            while is_digit(index) {
                index += 1;
            }
        }
        _ => return Err(index),
    }
    let mut fraction_or_exponent = false;
    if at(index) == Some(b'.') {
        index += 1;
        if !is_digit(index) {
            return Err(index);
        }
        while is_digit(index) {
            index += 1;
        }
        fraction_or_exponent = true;
    }
    if matches!(at(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(at(index), Some(b'+' | b'-')) {
            index += 1;
        }
        if !is_digit(index) {
            return Err(index);
        }
        while is_digit(index) {
            index += 1;
        }
        fraction_or_exponent = true;
    }
    if index == token.len() {
        Ok(NumberShape {
            fraction_or_exponent,
        })
    } else {
        Err(index)
    }
}

/// The length of the well-formed UTF-8 sequence starting at `start`, or
/// `None` when the bytes there are ill-formed or cut off by the end of the
/// input. The ranges are those of Unicode's table of well-formed byte
/// sequences, so overlong forms, encoded surrogates and values past U+10FFFF
/// are refused.
fn utf8_sequence_len(input: &[u8], start: usize) -> Option<usize> {
    let lead = *input.get(start)?;
    let continuation = |offset: usize, range: std::ops::RangeInclusive<u8>| {
        input
            .get(start + offset)
            .is_some_and(|byte| range.contains(byte))
    };
    let (len, second) = match lead {
        0x00..=0x7f => return Some(1),
        0xc2..=0xdf => (2, 0x80..=0xbf),
        0xe0 => (3, 0xa0..=0xbf),
        0xe1..=0xec | 0xee..=0xef => (3, 0x80..=0xbf),
        0xed => (3, 0x80..=0x9f),
        0xf0 => (4, 0x90..=0xbf),
        0xf1..=0xf3 => (4, 0x80..=0xbf),
        0xf4 => (4, 0x80..=0x8f),
        _ => return None,
    };
    if !continuation(1, second) {
        return None;
    }
    if (2..len).all(|offset| continuation(offset, 0x80..=0xbf)) {
        Some(len)
    } else {
        None
    }
}
