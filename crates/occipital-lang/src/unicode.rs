//! Unicode 16.0 primitives for §4.3 strings and §3.11 address display.
//!
//! Host Unicode APIs are deliberately not used: a toolchain upgrade must not
//! change the language's case mappings, whitespace or invisible-character set.

mod bidi_control;
mod case_ignorable;
mod cased;
mod default_ignorable;
mod lower;
mod upper;
mod white_space;

use std::str::Chars;

#[derive(Clone, Copy)]
pub(crate) enum Case {
    Upper,
    Lower,
}

/// Iterate default full mappings without allocating the result, so the evaluator
/// can count scalars and enforce its output limit before building a string (§6).
pub(crate) fn case_mapping(input: &str, case: Case) -> impl Iterator<Item = char> + '_ {
    CaseMapping {
        input: input.chars(),
        pending: "".chars(),
        preceded_by_cased: false,
        case,
    }
}

struct CaseMapping<'a> {
    input: Chars<'a>,
    pending: Chars<'static>,
    preceded_by_cased: bool,
    case: Case,
}

impl Iterator for CaseMapping<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        if let Some(c) = self.pending.next() {
            return Some(c);
        }
        let c = self.input.next()?;
        let mapping = match self.case {
            Case::Upper => lookup(upper::MAPPINGS, c),
            Case::Lower => {
                // Final_Sigma examines original input, ignoring Case_Ignorable
                // on both sides. Cased characters can themselves be ignorable.
                let final_sigma = c == lower::FINAL_SIGMA.0
                    && self.preceded_by_cased
                    && !self
                        .input
                        .clone()
                        .find(|c| !in_ranges(case_ignorable::RANGES, *c))
                        .is_some_and(|c| in_ranges(cased::RANGES, c));
                if final_sigma {
                    Some(lower::FINAL_SIGMA.1)
                } else {
                    lookup(lower::MAPPINGS, c)
                }
            }
        };
        if !in_ranges(case_ignorable::RANGES, c) {
            self.preceded_by_cased = in_ranges(cased::RANGES, c);
        }
        if let Some(mapped) = mapping {
            self.pending = mapped.chars();
            self.pending.next()
        } else {
            Some(c)
        }
    }
}

fn lookup(table: &[(char, &'static str)], c: char) -> Option<&'static str> {
    table
        .binary_search_by_key(&c, |&(key, _)| key)
        .ok()
        .map(|i| table[i].1)
}

fn in_ranges(table: &[(u32, u32)], c: char) -> bool {
    let cp = u32::from(c);
    let end = table.partition_point(|&(start, _)| start <= cp);
    end > 0 && cp <= table[end - 1].1
}

/// Unicode White_Space, not the host's notion of whitespace (§4.3).
pub(crate) fn is_white_space(c: char) -> bool {
    in_ranges(white_space::RANGES, c)
}

pub(crate) fn trim(input: &str) -> &str {
    input.trim_matches(is_white_space)
}

/// Address/authority display escapes these scalars as visible `\u{HEX}` tokens
/// (§3.11). This does not modify stored values or canonical URLs, and is distinct
/// from §5.6 sanitization, which replaces most controls rather than escaping them.
pub(crate) fn needs_display_escape(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1F}' | '\u{7F}'..='\u{9F}')
        || in_ranges(bidi_control::RANGES, c)
        || in_ranges(default_ignorable::RANGES, c)
}

#[cfg(test)]
mod tests;
