//! Bounded plain-text exports (§11), independent of checking and evaluation.

mod program;

// The reply exporter replaces only the final source spans it is given;
// failed tags remain literal.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod reply;

use crate::DisplayText;

const PROGRAM_BYTES: usize = 64 * 1024;
const PROGRAM_SCALARS: usize = 4096;
const REPLY_BYTES: usize = 256 * 1024;
const REPLY_SCALARS: usize = 65_536;

/// A sanitized plain-text export and the fuel charged for producing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackText {
    /// The sanitized fallback text.
    pub text: DisplayText,
    /// One per consumed source byte plus one per final output scalar (§9.2).
    pub fuel: u64,
    /// Source bytes the scanner examined.
    pub scanned_bytes: usize,
    /// Whether the text was cut at its 4,096-scalar limit (§11.1).
    pub truncated: bool,
}

/// Produce a bounded plain-text export without parsing (§11.1 of the language
/// specification), even for invalid or oversized source.
///
/// Title and summary each contribute a line of at most 512 scalars. Only the
/// first 64 KiB of source can be inspected, and the final text is at most 4096
/// scalars, including the ellipsis when cut. Memory beyond the output is constant.
pub fn program_fallback(source: &[u8], title: &str, summary: &str) -> FallbackText {
    program::scan(source, title, summary)
}

/// The app, not the language crate, owns the generic tool name/status row (§11.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ToolViewFallback {
    UseGenericRow,
}

#[allow(dead_code)]
pub(crate) fn tool_view_fallback() -> ToolViewFallback {
    ToolViewFallback::UseGenericRow
}

/// A bounded cursor repairs one UTF-8 sequence at a time, never allocating a
/// repaired copy of the input. Byte offsets remain offsets in the supplied bytes.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    tail: bool,
}

impl<'a> Cursor<'a> {
    fn new(source: &'a [u8], cap: usize) -> Self {
        Self {
            bytes: &source[..source.len().min(cap)],
            pos: 0,
            tail: source.len() > cap,
        }
    }

    fn rest(&self) -> &'a [u8] {
        // `pos` only advances over bytes already read, so it never passes the end.
        self.bytes.get(self.pos..).unwrap_or_default()
    }

    fn peek(&self) -> Option<u8> {
        self.rest().first().copied()
    }

    fn starts(&self, token: &[u8]) -> bool {
        self.rest().starts_with(token)
    }

    fn next(&mut self) -> Option<char> {
        let first = self.peek()?;
        let width = match first {
            0..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        let window = &self.rest()[..width.min(self.rest().len())];
        let (ch, length) = match std::str::from_utf8(window) {
            Ok(text) => (text.chars().next()?, width),
            Err(error) => {
                if error.error_len().is_none() && self.tail {
                    // The byte cap bisects a scalar: do not read the tail to
                    // finish it, or export a replacement for that cut scalar.
                    self.pos = self.bytes.len();
                    return None;
                }
                ('\u{fffd}', error.error_len().unwrap_or(window.len()))
            }
        };
        self.pos += length;
        Some(ch)
    }

    fn source_char(&mut self) -> Option<char> {
        let ch = self.next()?;
        if ch == '\r' {
            if self.peek() == Some(b'\n') {
                self.pos += 1;
            }
            Some('\n')
        } else {
            Some(ch)
        }
    }
}

struct Output {
    text: String,
    scalars: usize,
    cap: usize,
    collapse_blank: bool,
    newlines: usize,
    line_byte: usize,
    line_scalars: usize,
    cut: bool,
}

impl Output {
    fn new(cap: usize, collapse_blank: bool) -> Self {
        Self {
            text: String::new(),
            scalars: 0,
            cap,
            collapse_blank,
            newlines: 0,
            line_byte: 0,
            line_scalars: 0,
            cut: false,
        }
    }

    fn full(&self) -> bool {
        self.scalars >= self.cap
    }

    fn push(&mut self, ch: char) {
        if self.full() {
            self.cut = true;
            return;
        }
        let ch = match ch {
            '\t' | '\n' => ch,
            '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' => '\u{fffd}',
            _ => ch,
        };
        if self.collapse_blank && ch == '\n' && self.newlines >= 2 {
            self.text.truncate(self.line_byte);
            self.scalars = self.line_scalars;
            return;
        }
        if ch == '\n' {
            self.newlines += 1;
        } else if !matches!(ch, ' ' | '\t') {
            self.newlines = 0;
        }
        self.text.push(ch);
        self.scalars += 1;
        if ch == '\n' {
            self.line_byte = self.text.len();
            self.line_scalars = self.scalars;
        }
    }

    fn write(&mut self, text: &str) {
        for ch in text.chars() {
            self.push(ch);
            if self.cut {
                break;
            }
        }
    }

    fn line_start(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.push('\n');
        }
    }

    fn finish(mut self, cursor: &Cursor<'_>, append_ellipsis: bool) -> FallbackText {
        let truncated = self.cut || cursor.tail || cursor.pos < cursor.bytes.len();
        if truncated {
            if self.full() && !append_ellipsis {
                self.text.pop();
                self.scalars -= 1;
            }
            self.text.push('…');
            self.scalars += 1;
        }
        FallbackText {
            fuel: (cursor.pos + self.scalars) as u64,
            text: DisplayText::from_string(self.text),
            scanned_bytes: cursor.pos,
            truncated,
        }
    }
}

#[cfg(test)]
mod tests;
