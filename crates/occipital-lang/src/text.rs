use std::fmt;

use serde::Serialize;

use crate::unicode::needs_display_escape;

/// Text safe for display and plain-text export (§5.6).
///
/// Construct this at the output boundary for every tree string (including code
/// and props), failed-tag literal, fallback, title/summary and unavailable row.
/// Keep language strings and raw snapshots unchanged until that boundary: text
/// operations must still see the original scalars. This is not a terminal
/// protocol encoder or an HTML/Markdown parser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DisplayText(String);

impl DisplayText {
    pub(crate) fn retained_allocation(&self) -> Option<crate::RetainedAllocation> {
        (self.0.capacity() != 0).then_some(crate::RetainedAllocation {
            requested: self.0.len() as u64,
            unused_capacity: (self.0.capacity() - self.0.len()) as u64,
        })
    }
    /// Neutralize C0/C1 controls, keeping only tab and LF (§5.6).
    pub fn new(text: &str) -> Self {
        Self(
            text.chars()
                .map(|c| match c {
                    '\t' | '\n' => c,
                    '\u{0}'..='\u{1F}' | '\u{7F}'..='\u{9F}' => '\u{FFFD}',
                    _ => c,
                })
                .collect(),
        )
    }

    /// Reuse an already-safe output buffer, avoiding a second bounded fallback
    /// allocation. Unsafe display controls still go through the sanitizer.
    pub(crate) fn from_string(text: String) -> Self {
        if text
            .chars()
            .any(|ch| matches!(ch, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'..='\u{9f}'))
        {
            Self::new(&text)
        } else {
            Self(text)
        }
    }

    /// Repair UTF-8 and sanitize even source that never parsed.
    ///
    /// Callers enforce their scan/output bounds before passing a prefix here;
    /// this helper neither scans a tail nor applies fallback truncation rules.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::new(&String::from_utf8_lossy(bytes))
    }

    /// A full address/authority-sheet representation, not an opened URL (§3.11).
    ///
    /// Escape controls (including tab/LF), bidi controls and default ignorables
    /// using the pinned Unicode tables, without truncation or normalization.
    /// Validation, canonicalization and the sheet's size bound belong to the
    /// URL/authority layer; this representation must never be opened as a URL.
    pub fn address_sheet(text: &str) -> Self {
        let mut escaped = String::new();
        for c in text.chars() {
            if needs_display_escape(c) {
                escaped.push_str(&format!("\\u{{{:X}}}", c as u32));
            } else {
                escaped.push(c);
            }
        }
        Self::new(&escaped)
    }

    /// The sanitized text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for DisplayText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for DisplayText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "text/tests.rs"]
mod tests;
