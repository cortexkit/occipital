//! Strict RFC 8259 JSON (§10.3) building language values directly, so every
//! §10.5 cap is checked before the value it guards is built.

use std::collections::HashSet;

use super::{
    cell_type_findings, convert_string, Cap, ColumnType, Finding, Format, Meter, Scalar, Step,
    Stop, Tally, COLUMNS_CAP, DEPTH_CAP, ROWS_CAP, STRING_CAP,
};
use crate::number::Decimal;
use crate::{DiagnosticCode, ScalarString, SourceSpan, Value};

/// Where a value sits, which decides the caps and conversions that apply.
#[derive(Clone, Copy)]
enum Place {
    /// The root: as an array its elements are rows (row cap).
    Root,
    /// An element of the root array, numbered from 1. As an object it is a
    /// record whose fields are columns (column cap, `columns` conversions).
    Row(u64),
    /// A field of a row record that `columns` declares, by declaration index.
    Cell(usize, u64),
    Other,
}

struct Parser<'t, 'm, 'f, E, F: super::Charge<E>> {
    text: &'t str,
    bytes: &'t [u8],
    at: usize,
    meter: &'m mut Meter<'f, E, F>,
    columns: &'t [(String, ColumnType)],
    span: SourceSpan,
    rounded: Tally,
    range: Tally,
    surrogate: Tally,
    cells: Vec<Tally>,
}

pub(super) fn parse<E, F: super::Charge<E>>(
    text: &str,
    columns: &[(String, ColumnType)],
    span: SourceSpan,
    meter: &mut Meter<'_, E, F>,
) -> Step<(Value, Vec<Finding>), E> {
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        at: 0,
        meter,
        columns,
        span,
        rounded: Tally::default(),
        range: Tally::default(),
        surrogate: Tally::default(),
        cells: vec![Tally::default(); columns.len()],
    };
    parser.whitespace();
    let value = parser.value(0, Place::Root)?;
    parser.whitespace();
    if parser.at != parser.bytes.len() {
        return Err(Stop::syntax(DiagnosticCode::JsonSyntax, parser.at));
    }
    let mut findings = Vec::new();
    findings.extend(
        parser
            .rounded
            .finding(DiagnosticCode::NumberRounded)
            .map(|f| {
                f.counts(
                    "{} data numbers had more than 18 fraction digits and were rounded.",
                    vec![parser.rounded.count],
                )
            }),
    );
    findings.extend(parser.range.finding(DiagnosticCode::NumberRange).map(|f| {
        f.counts(
            "{} data numbers were out of range and became E_OVERFLOW errors.",
            vec![parser.range.count],
        )
    }));
    findings.extend(
        parser
            .surrogate
            .finding(DiagnosticCode::JsonSurrogate)
            .map(|f| {
                f.counts(
                    "{} unpaired surrogate escapes were replaced with U+FFFD.",
                    vec![parser.surrogate.count],
                )
            }),
    );
    findings.extend(cell_type_findings(columns, &parser.cells));
    Ok((value, findings))
}

impl<'t, E, F: super::Charge<E>> Parser<'t, '_, '_, E, F> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn syntax<T>(&self, offset: usize) -> Step<T, E> {
        Err(Stop::syntax(DiagnosticCode::JsonSyntax, offset))
    }

    /// RFC 8259 white space: space, tab, LF and CR only.
    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self, depth: u32, place: Place) -> Step<Value, E> {
        let start = self.at;
        let first = match self.peek() {
            Some(byte @ (b'{' | b'[' | b'"' | b't' | b'f' | b'n' | b'-' | b'0'..=b'9')) => byte,
            _ => return self.syntax(start),
        };
        self.meter.value(start)?;
        match first {
            b'{' => self.object(depth + 1, place),
            b'[' => self.array(depth + 1, place),
            b'"' => {
                let (text, _) = self.string()?;
                let scalar = match place {
                    Place::Cell(column, row) => {
                        let (scalar, failed) =
                            convert_string(text, self.columns[column].1, Format::Json);
                        if failed {
                            self.cells[column].note(start, row, None);
                        }
                        scalar
                    }
                    _ => Scalar::Text(text),
                };
                self.build(scalar, start)
            }
            b't' | b'f' | b'n' => {
                let (word, scalar) = match first {
                    b't' => ("true", Scalar::Boolean(true)),
                    b'f' => ("false", Scalar::Boolean(false)),
                    _ => ("null", Scalar::Null),
                };
                if !self.bytes[start..].starts_with(word.as_bytes()) {
                    return self.syntax(start);
                }
                self.at += word.len();
                let scalar = match (place, scalar) {
                    (_, Scalar::Null) => Scalar::Null,
                    (Place::Cell(column, row), Scalar::Boolean(value)) => {
                        match self.columns[column].1 {
                            ColumnType::Text => Scalar::Text(word.into()),
                            ColumnType::Boolean => Scalar::Boolean(value),
                            ColumnType::Number => {
                                self.cells[column].note(start, row, None);
                                Scalar::Error("E_TYPE")
                            }
                        }
                    }
                    (_, scalar) => scalar,
                };
                self.build(scalar, start)
            }
            _ => {
                let token = self.number()?;
                let scalar = match place {
                    Place::Cell(column, row) => match self.columns[column].1 {
                        // The exact source token, so ids beyond the number range
                        // survive intact; nothing was rounded, so no warning.
                        ColumnType::Text => Scalar::Text(token.into()),
                        ColumnType::Number => self.decimal(token, start),
                        ColumnType::Boolean => {
                            self.cells[column].note(start, row, None);
                            Scalar::Error("E_TYPE")
                        }
                    },
                    _ => self.decimal(token, start),
                };
                self.build(scalar, start)
            }
        }
    }

    fn build(&mut self, scalar: Scalar, offset: usize) -> Step<Value, E> {
        self.meter.grow(scalar.size(), offset)?;
        self.meter.scalar(&scalar)?;
        Ok(scalar.build(self.span))
    }

    fn array(&mut self, depth: u32, place: Place) -> Step<Value, E> {
        let start = self.at;
        if depth > DEPTH_CAP {
            return Err(Stop::limit(Cap::Depth, start));
        }
        self.meter.grow(16, start)?;
        self.at += 1;
        self.whitespace();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.at += 1;
        } else {
            loop {
                let element = match place {
                    Place::Root => {
                        if items.len() == ROWS_CAP {
                            return Err(Stop::limit(Cap::Rows, self.at));
                        }
                        Place::Row(items.len() as u64 + 1)
                    }
                    _ => Place::Other,
                };
                items.push(self.value(depth, element)?);
                self.whitespace();
                match self.peek() {
                    Some(b',') => {
                        self.at += 1;
                        self.whitespace();
                    }
                    Some(b']') => {
                        self.at += 1;
                        break;
                    }
                    _ => return self.syntax(self.at),
                }
            }
        }
        self.meter.slice::<Value>(items.len())?;
        Value::list(items).map_err(|_| Stop::limit(Cap::Size, start))
    }

    fn object(&mut self, depth: u32, place: Place) -> Step<Value, E> {
        let start = self.at;
        if depth > DEPTH_CAP {
            return Err(Stop::limit(Cap::Depth, start));
        }
        self.meter.grow(16, start)?;
        self.at += 1;
        self.whitespace();
        let mut fields = Vec::new();
        let mut keys = HashSet::new();
        if self.peek() == Some(b'}') {
            self.at += 1;
        } else {
            loop {
                let key_at = self.at;
                if self.peek() != Some(b'"') {
                    return self.syntax(key_at);
                }
                // The fields of a row record are its columns.
                if matches!(place, Place::Row(_)) && fields.len() == COLUMNS_CAP {
                    return Err(Stop::limit(Cap::Columns, key_at));
                }
                let (key, scalars) = self.string()?;
                if !keys.insert(key.clone()) {
                    return Err(Stop::syntax(DiagnosticCode::JsonDuplicateKey, key_at));
                }
                self.meter.grow(16 + 4 * scalars, key_at)?;
                self.whitespace();
                if self.peek() != Some(b':') {
                    return self.syntax(self.at);
                }
                self.at += 1;
                self.whitespace();
                let field = match place {
                    Place::Row(row) => self
                        .columns
                        .iter()
                        .position(|(name, _)| *name == key)
                        .map_or(Place::Other, |column| Place::Cell(column, row)),
                    _ => Place::Other,
                };
                let value = self.value(depth, field)?;
                self.meter.slice::<u8>(key.len())?;
                fields.push((ScalarString::new(key), value));
                self.whitespace();
                match self.peek() {
                    Some(b',') => {
                        self.at += 1;
                        self.whitespace();
                    }
                    Some(b'}') => {
                        self.at += 1;
                        break;
                    }
                    _ => return self.syntax(self.at),
                }
            }
        }
        self.meter.slice::<(ScalarString, Value)>(fields.len())?;
        Value::record(fields).map_err(|_| Stop::limit(Cap::Size, start))
    }

    /// A string starting at its opening quote: its decoded text and length in
    /// scalars. The scalar cap stops it as soon as it is exceeded.
    fn string(&mut self) -> Step<(String, u64), E> {
        let start = self.at;
        self.at += 1;
        let mut text = String::new();
        let mut scalars: u64 = 0;
        loop {
            let at = self.at;
            match self.peek() {
                None => return self.syntax(at),
                Some(b'"') => {
                    self.at += 1;
                    return Ok((text, scalars));
                }
                Some(b'\\') => {
                    let c = self.escape()?;
                    text.push(c);
                }
                // RFC 8259 requires control characters to be escaped.
                Some(0x00..=0x1F) => return self.syntax(at),
                Some(_) => {
                    // `peek` found a byte here, and `at` is always on a
                    // scalar boundary, so a scalar starts here.
                    let Some(c) = self.text.get(at..).and_then(|rest| rest.chars().next()) else {
                        return self.syntax(at);
                    };
                    self.at += c.len_utf8();
                    text.push(c);
                }
            }
            scalars += 1;
            if scalars > STRING_CAP {
                return Err(Stop::limit(Cap::String, start));
            }
        }
    }

    /// One escape sequence starting at its backslash. A surrogate pair becomes
    /// one scalar; a lone surrogate becomes U+FFFD and is tallied (§10.3).
    fn escape(&mut self) -> Step<char, E> {
        let start = self.at;
        let c = match self.bytes.get(start + 1) {
            Some(b'"') => '"',
            Some(b'\\') => '\\',
            Some(b'/') => '/',
            Some(b'b') => '\u{8}',
            Some(b'f') => '\u{C}',
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'u') => {
                let unit = self
                    .hex(start + 2)
                    .ok_or(Stop::syntax(DiagnosticCode::JsonSyntax, start))?;
                self.at = start + 6;
                let high = (0xD800..0xDC00).contains(&unit);
                let low = self.bytes[self.at..]
                    .starts_with(b"\\u")
                    .then(|| self.hex(self.at + 2))
                    .flatten()
                    .filter(|unit| (0xDC00..0xE000).contains(unit));
                // A high surrogate followed by a low one names a scalar in
                // U+10000..=U+10FFFF, so `from_u32` always succeeds.
                let pair = low.filter(|_| high).and_then(|low| {
                    char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))
                });
                return Ok(match pair {
                    Some(scalar) => {
                        self.at += 6;
                        scalar
                    }
                    _ => match char::from_u32(unit) {
                        Some(c) => c,
                        None => {
                            self.surrogate.note(start, 0, None);
                            '\u{FFFD}'
                        }
                    },
                });
            }
            _ => return self.syntax(start),
        };
        self.at = start + 2;
        Ok(c)
    }

    /// Four hexadecimal digits at `at`, as a UTF-16 code unit.
    fn hex(&self, at: usize) -> Option<u32> {
        let digits = self.bytes.get(at..at + 4)?;
        digits.iter().try_fold(0, |unit, &digit| {
            Some(unit * 16 + char::from(digit).to_digit(16)?)
        })
    }

    /// An RFC 8259 number token: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
    /// No leading `+`, no leading zeros, no bare `.` (§10.3).
    fn number(&mut self) -> Step<&'t str, E> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return self.syntax(self.at);
                }
            }
            Some(b'1'..=b'9') => self.digits(),
            _ => return self.syntax(self.at),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return self.syntax(self.at);
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return self.syntax(self.at);
            }
            self.digits();
        }
        let text = self.text;
        Ok(&text[start..self.at])
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
    }

    /// The exact number of a token, rounded half to even past 18 places, or the
    /// error value `E_OVERFLOW` out of range. Each case is tallied once per
    /// parse (§10.3). `-0` is 0, because there is no negative zero.
    fn decimal(&mut self, token: &str, offset: usize) -> Scalar {
        match Decimal::parse_text(token) {
            Ok(number) => {
                if rounds(token) {
                    self.rounded.note(offset, 0, None);
                }
                Scalar::Number(number)
            }
            Err(_) => {
                self.range.note(offset, 0, None);
                Scalar::Error("E_OVERFLOW")
            }
        }
    }
}

/// Whether a valid JSON number token has a nonzero digit beyond the 18th
/// fraction place once its exponent is applied, so converting it rounds.
/// Trailing zeros do not count: `0.1000000000000000000` is exactly 0.1.
fn rounds(token: &str) -> bool {
    let unsigned = token.strip_prefix('-').unwrap_or(token);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], &unsigned[at + 1..]),
        None => (unsigned, "0"),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = || integer.bytes().chain(fraction.bytes());
    if digits().all(|digit| digit == b'0') {
        return false;
    }
    let trailing_zeros = integer
        .bytes()
        .chain(fraction.bytes())
        .rev()
        .take_while(|&digit| digit == b'0')
        .count() as i128;
    // Saturating keeps an absurd exponent far beyond any digit count.
    let negative = exponent.starts_with('-');
    let magnitude = exponent
        .trim_start_matches(['+', '-'])
        .bytes()
        .fold(0_i128, |value, digit| {
            (value * 10 + i128::from(digit - b'0')).min(1 << 64)
        });
    let exponent = if negative { -magnitude } else { magnitude };
    fraction.len() as i128 - exponent - trailing_zeros > 18
}
