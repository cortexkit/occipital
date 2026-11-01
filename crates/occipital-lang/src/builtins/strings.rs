use super::{Context, Failure, Result};
use crate::unicode::{self, Case};
use crate::{Decimal, Value, ValueKind};

/// The string built-ins.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Op {
    Text,
    Len,
    Upper,
    Lower,
    Trim,
    StartsWith,
    EndsWith,
    Replace,
    Split,
    Lines,
    Truncate,
}

pub(super) fn text_len<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    value: &Value,
    cx: &Context<'_, E, F>,
) -> Result<u64, E> {
    match value.kind() {
        ValueKind::String(text) => Ok(text.len()),
        ValueKind::Number(n) => Ok(n.text_len()),
        ValueKind::Boolean(b) => Ok(if *b { 4 } else { 5 }),
        ValueKind::Null | ValueKind::Missing => Ok(0),
        ValueKind::Error(e) => Err(Failure::Value(e.clone())),
        _ => Err(cx.error("E_TYPE")),
    }
}

pub(super) fn append_text(output: &mut String, value: &Value) {
    match value.kind() {
        ValueKind::String(text) => output.push_str(text.as_str()),
        ValueKind::Number(n) => output.push_str(&n.to_string()),
        ValueKind::Boolean(b) => output.push_str(if *b { "true" } else { "false" }),
        // Other kinds were refused during the length preflight (`text_len`)
        // and contribute no text.
        _ => (),
    }
}

pub(super) fn call<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    builtin: Op,
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    use Op::*;
    match builtin {
        Text => {
            let length = text_len(&args[0], cx)?;
            cx.string_limit(length)?;
            cx.pay(length.div_ceil(8))?;
            let mut text = String::new();
            append_text(&mut text, &args[0]);
            cx.build_string(text)
        }
        Len => {
            cx.pay(1)?;
            let len = if args[0].is_absent() {
                0
            } else {
                cx.string(&args[0])?.len()
            };
            Ok(Value::number(Decimal::from_u64(len)))
        }
        StartsWith | EndsWith => {
            let pattern = cx.string(&args[1])?;
            cx.pay(pattern.len().div_ceil(16))?;
            if args[0].is_absent() {
                return Ok(Value::boolean(false));
            }
            let text = cx.string(&args[0])?;
            Ok(Value::boolean(if builtin == StartsWith {
                text.as_str().starts_with(pattern.as_str())
            } else {
                text.as_str().ends_with(pattern.as_str())
            }))
        }
        Upper | Lower => {
            let text = cx.string(&args[0])?;
            let case = if builtin == Upper {
                Case::Upper
            } else {
                Case::Lower
            };
            cx.pay(text.len().div_ceil(8))?;
            let length = unicode::case_mapping(text.as_str(), case).count() as u64;
            cx.string_limit(length)?;
            cx.pay(length.div_ceil(8))?;
            cx.build_string(unicode::case_mapping(text.as_str(), case).collect())
        }
        Trim => {
            let text = cx.string(&args[0])?;
            cx.pay(text.len().div_ceil(8))?;
            let trimmed = unicode::trim(text.as_str());
            let length = trimmed.chars().count() as u64;
            cx.string_limit(length)?;
            cx.pay(length.div_ceil(8))?;
            cx.build_string(trimmed.into())
        }
        Truncate => {
            let text = cx.string(&args[0])?;
            let max = cx
                .number(&args[1])?
                .integer()
                .filter(|n| *n >= 1)
                .ok_or_else(|| cx.error("E_ARG"))?;
            let length = u128::from(text.len()).min(max as u128) as u64;
            cx.string_limit(length)?;
            cx.pay(length.div_ceil(8))?;
            if length == text.len() {
                return Ok(args[0].clone());
            }
            let mut result: String = text.as_str().chars().take(length as usize - 1).collect();
            result.push('…');
            cx.build_string(result)
        }
        Replace => replace(args, cx),
        Split | Lines => split(args, builtin == Lines, cx),
    }
}

/// Knuth–Morris–Pratt search over UTF-8 bytes takes O(source + pattern) even for repetitive
/// adversarial strings. Valid UTF-8 patterns can match only at scalar boundaries.
pub(super) struct Search<'a> {
    pattern: &'a [u8],
    prefix: Vec<usize>,
}

impl<'a> Search<'a> {
    pub(super) fn new(pattern: &'a str) -> Self {
        let pattern = pattern.as_bytes();
        let mut prefix = vec![0; pattern.len()];
        let mut length = 0;
        for i in 1..pattern.len() {
            while length > 0 && pattern[i] != pattern[length] {
                length = prefix[length - 1];
            }
            if pattern[i] == pattern[length] {
                length += 1;
            }
            prefix[i] = length;
        }
        Self { pattern, prefix }
    }

    pub(super) fn matches<'s>(&'s self, source: &'s str) -> Matches<'s, 'a> {
        Matches {
            search: self,
            source: source.as_bytes(),
            offset: 0,
            matched: 0,
            empty: false,
        }
    }
}

pub(super) struct Matches<'s, 'p> {
    search: &'s Search<'p>,
    source: &'s [u8],
    offset: usize,
    matched: usize,
    empty: bool,
}

impl Iterator for Matches<'_, '_> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        if self.search.pattern.is_empty() {
            return if self.empty {
                None
            } else {
                self.empty = true;
                Some(0)
            };
        }
        for i in self.offset..self.source.len() {
            let byte = self.source[i];
            while self.matched > 0 && byte != self.search.pattern[self.matched] {
                self.matched = self.search.prefix[self.matched - 1];
            }
            if byte == self.search.pattern[self.matched] {
                self.matched += 1;
            }
            if self.matched == self.search.pattern.len() {
                self.offset = i + 1;
                self.matched = 0;
                return Some(i + 1 - self.search.pattern.len());
            }
        }
        self.offset = self.source.len();
        None
    }
}

fn replace<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    let text = cx.string(&args[0])?;
    let from = cx.string(&args[1])?;
    let to = cx.string(&args[2])?;
    if from.is_empty() {
        return Err(cx.error("E_ARG"));
    }
    cx.pay(text.len().div_ceil(16))?;
    // A pattern longer than the source cannot match. Avoid allocating a search
    // table for an arbitrarily large host-supplied pattern in that case.
    let search = Search::new(if from.len() > text.len() {
        ""
    } else {
        from.as_str()
    });
    let count = if from.len() > text.len() {
        0
    } else {
        search.matches(text.as_str()).count() as u64
    };
    let length = text
        .len()
        .saturating_sub(count.saturating_mul(from.len()))
        .saturating_add(count.saturating_mul(to.len()));
    cx.string_limit(length)?;
    cx.pay(length.div_ceil(8))?;
    let mut result = String::new();
    let mut previous = 0;
    if count != 0 {
        for index in search.matches(text.as_str()) {
            result.push_str(&text.as_str()[previous..index]);
            result.push_str(to.as_str());
            previous = index + from.as_str().len();
        }
    }
    result.push_str(&text.as_str()[previous..]);
    cx.build_string(result)
}

/// Repeatable piece traversal keeps the preflight free of output values. The
/// second traversal runs only after every piece and the aggregate length fit.
fn pieces(source: &str, search: Option<&Search<'_>>, mut visit: impl FnMut(&str)) {
    if let Some(search) = search {
        let mut previous = 0;
        for index in search.matches(source) {
            visit(&source[previous..index]);
            previous = index + search.pattern.len();
        }
        visit(&source[previous..]);
    } else {
        let bytes = source.as_bytes();
        let mut previous = 0;
        let mut i = 0;
        while i < bytes.len() {
            if matches!(bytes[i], b'\r' | b'\n') {
                visit(&source[previous..i]);
                i += if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                previous = i;
            } else {
                i += 1;
            }
        }
        if previous < source.len() {
            visit(&source[previous..]);
        }
    }
}

fn split<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    lines: bool,
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    let separator = if lines {
        None
    } else {
        let separator = cx.string(&args[1])?;
        if separator.is_empty() {
            return Err(cx.error("E_ARG"));
        }
        Some(separator)
    };
    if args[0].is_absent() {
        return cx.build_list(Vec::new());
    }
    let text = cx.string(&args[0])?;
    cx.pay(text.len().div_ceil(16))?;
    // A non-matching oversized separator needs no prefix table. Using the whole
    // source as one piece avoids scanning that unused separator.
    let oversized = separator.is_some_and(|s| s.len() > text.len());
    let search = separator
        .filter(|_| !oversized)
        .map(|s| Search::new(s.as_str()));
    let traverse = |visit: &mut dyn FnMut(&str)| {
        if oversized {
            visit(text.as_str());
        } else {
            pieces(text.as_str(), search.as_ref(), visit);
        }
    };
    let mut count = 0u64;
    let mut total = 0u64;
    let mut longest = 0;
    traverse(&mut |piece| {
        let length = piece.chars().count() as u64;
        count = count.saturating_add(1);
        total = total.saturating_add(length);
        longest = longest.max(length);
    });
    cx.list_limit(count)?;
    cx.string_limit(longest)?;
    cx.pay(count.saturating_add(total.div_ceil(8)))?;
    let mut output = Vec::with_capacity(count as usize);
    // Each piece passed the string limit above.
    traverse(&mut |piece| output.push(Value::text(piece)));
    cx.build_list(output)
}
