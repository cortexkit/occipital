//! Facts: one labelled value a module reports, worded the same way by every
//! app.
//!
//! A module reports facts in a command's preview or result, such as
//! `{"label": "Removes", "value": 18200, "unit": "tokens", "approx": true}`
//! or `{"label": "Code map", "state": "building", "progress": 0.4}`. The
//! label is the module's own words. The value is never text: it is a number
//! with a unit from a closed list, or a state from a closed list, and this
//! module words it ("about 18k tokens", "Building, 40%") and gives it a
//! tone. Both apps draw facts with [`format_fact`], so the same fact reads and
//! colors the same everywhere, and no module invents its own "ready".

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::builtins::format::{bytes_text, duration_text, number_text};
use crate::Decimal;

/// The longest label, in Unicode scalar values.
const MAX_LABEL: usize = 64;
/// The longest noun after a count, in Unicode scalar values.
const MAX_NOUN: usize = 24;

/// What a number fact counts. It decides how the number is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Unit {
    /// A plain count, grouped by thousands, followed by the fact's `noun`.
    Count,
    /// A token count, rounded to a whole number.
    Tokens,
    /// A size in bytes, in binary units.
    Bytes,
    /// A duration in milliseconds.
    Duration,
    /// A share, where 1 is the whole, written as a whole percent.
    Ratio,
}

/// A fixed condition a fact can report instead of a number. Each has one
/// wording and one tone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Ready,
    Building,
    Paused,
    Stale,
    Unavailable,
    Off,
}

impl State {
    fn words(self) -> &'static str {
        match self {
            State::Ready => "Ready",
            State::Building => "Building",
            State::Paused => "Paused",
            State::Stale => "Out of date",
            State::Unavailable => "Unavailable",
            State::Off => "Off",
        }
    }

    fn tone(self) -> Tone {
        match self {
            State::Ready => Tone::Success,
            State::Building => Tone::Info,
            State::Paused | State::Stale => Tone::Warning,
            State::Unavailable => Tone::Danger,
            State::Off => Tone::Neutral,
        }
    }
}

/// The catalog's tones, each with one meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// No particular meaning.
    Neutral,
    /// Something in progress or worth noting.
    Info,
    /// Working as it should.
    Success,
    /// Needs attention soon.
    Warning,
    /// Broken or failed.
    Danger,
}

/// A fact as it arrives. Exactly one of `value` and `state` is present; the
/// members that only make sense for one of them are refused with the other.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    label: String,
    value: Option<serde_json::Number>,
    unit: Option<Unit>,
    noun: Option<String>,
    #[serde(default)]
    approx: bool,
    state: Option<State>,
    progress: Option<serde_json::Number>,
    tone: Option<Tone>,
}

/// A fact as an app draws it: the module's label, the value's wording, and
/// its tone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FormattedFact {
    /// The module's label, as given.
    pub label: String,
    /// The value's wording, such as "about 18k tokens" or "Ready".
    pub text: String,
    /// The tone to draw the value in.
    pub tone: Tone,
}

/// Why a fact can't be drawn. An app shows such a fact as missing rather
/// than guessing at it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactError {
    /// Not a fact object: a missing label, a member of the wrong type, an
    /// unknown member, unit or state.
    Malformed(String),
    /// Members that don't go together: both or neither of `value` and
    /// `state`; `unit`, `noun`, `approx` or `tone` on a state; `progress`
    /// on anything but `building`; `noun` on a unit other than `count`.
    Conflict(&'static str),
    /// A value out of range: an empty or overlong label or noun, a number
    /// the language can't hold, or `progress` outside 0 to 1.
    Range(&'static str),
}

impl FactError {
    /// The stable code apps and the C binding report.
    pub fn code(&self) -> &'static str {
        match self {
            FactError::Malformed(_) => "fact_malformed",
            FactError::Conflict(_) => "fact_conflict",
            FactError::Range(_) => "fact_range",
        }
    }
}

impl std::fmt::Display for FactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FactError::Malformed(detail) => write!(f, "{}: {detail}", self.code()),
            FactError::Conflict(detail) | FactError::Range(detail) => {
                write!(f, "{}: {detail}", self.code())
            }
        }
    }
}

/// Words one fact: its label as given, its value in the fixed English
/// wording, and its tone.
///
/// - A number is written by its unit: `count` grouped by thousands with its
///   `noun` ("41 parts"), `tokens` rounded ("18,200 tokens", "1 token"),
///   `bytes` in binary units ("7.6 MiB"), `duration` from milliseconds
///   ("2m 5s"), `ratio` as a whole percent ("61%"). Without a unit it is a
///   `count`. `approx` puts "about " in front, and an estimate shows no more
///   digits than it means: a count or tokens from 1,000 up is shortened to
///   thousands or millions, one decimal under ten ("about 6.7k tokens",
///   "about 115k tokens", "about 1.2M tokens", "about 12k parts"), rounded
///   half away from zero. An estimate of zero, in any unit, is "none". Its
///   tone is `neutral` unless the fact names one, such as `danger` for an
///   error count.
/// - A state is written by its fixed wording ("Ready", "Out of date"), and
///   `building` with `progress` adds the percent ("Building, 40%"). Its tone
///   is fixed: `ready` success, `building` info, `paused` and `stale`
///   warning, `unavailable` danger, `off` neutral.
///
/// JSON numbers arrive through `serde_json` without `arbitrary_precision`,
/// so a fraction is the shortest text that round-trips its `f64`; that is
/// exact for the few significant digits a fact carries.
pub fn format_fact(fact: &Json) -> Result<FormattedFact, FactError> {
    let wire: Wire =
        serde_json::from_value(fact.clone()).map_err(|e| FactError::Malformed(e.to_string()))?;
    let label_len = wire.label.chars().count();
    if label_len == 0 || label_len > MAX_LABEL {
        return Err(FactError::Range("label must be 1 to 64 characters"));
    }
    let (text, tone) = match (&wire.value, wire.state) {
        (Some(value), None) => {
            if wire.progress.is_some() {
                return Err(FactError::Conflict(
                    "progress is only for the building state",
                ));
            }
            let unit = wire.unit.unwrap_or(Unit::Count);
            if wire.noun.is_some() && unit != Unit::Count {
                return Err(FactError::Conflict("noun is only for a count"));
            }
            let x = decimal(value)?;
            let mut text = String::new();
            if wire.approx && x == Decimal::ZERO {
                text.push_str("none");
            } else {
                let words = number_words(x, unit, wire.noun.as_deref())?;
                if wire.approx {
                    text.push_str("about ");
                    let short = matches!(unit, Unit::Count | Unit::Tokens)
                        .then(|| shortened(x))
                        .transpose()?
                        .flatten();
                    match short {
                        // The number's words with its digits replaced: the
                        // noun or "tokens" after it stays as it was.
                        Some(short) => {
                            let rest = words.split_once(' ').map_or("", |(_, rest)| rest);
                            text.push_str(&short);
                            if unit == Unit::Tokens {
                                text.push_str(" tokens");
                            } else if !rest.is_empty() {
                                text.push(' ');
                                text.push_str(rest);
                            }
                        }
                        None => text.push_str(&words),
                    }
                } else {
                    text.push_str(&words);
                }
            }
            (text, wire.tone.unwrap_or(Tone::Neutral))
        }
        (None, Some(state)) => {
            if wire.unit.is_some() || wire.noun.is_some() || wire.approx || wire.tone.is_some() {
                return Err(FactError::Conflict(
                    "a state takes no unit, noun, approx or tone",
                ));
            }
            let mut text = state.words().to_owned();
            if let Some(progress) = &wire.progress {
                if state != State::Building {
                    return Err(FactError::Conflict(
                        "progress is only for the building state",
                    ));
                }
                let share = decimal(progress)?;
                if share < Decimal::ZERO || share > Decimal::ONE {
                    return Err(FactError::Range("progress must be 0 to 1"));
                }
                text.push_str(", ");
                text.push_str(&percent(share)?);
            }
            (text, state.tone())
        }
        _ => {
            return Err(FactError::Conflict(
                "a fact has exactly one of value and state",
            ))
        }
    };
    Ok(FormattedFact {
        label: wire.label,
        text,
        tone,
    })
}

fn decimal(number: &serde_json::Number) -> Result<Decimal, FactError> {
    Decimal::parse_text(&number.to_string())
        .map_err(|_| FactError::Range("a number the language can't hold"))
}

fn number_words(x: Decimal, unit: Unit, noun: Option<&str>) -> Result<String, FactError> {
    Ok(match unit {
        Unit::Count => {
            let mut text = number_text(x, None, true);
            if let Some(noun) = noun {
                let noun_len = noun.chars().count();
                if noun_len == 0 || noun_len > MAX_NOUN {
                    return Err(FactError::Range("noun must be 1 to 24 characters"));
                }
                text.push(' ');
                text.push_str(noun);
            }
            text
        }
        Unit::Tokens => {
            let text = number_text(x, Some(0), true);
            let word = if text == "1" { "token" } else { "tokens" };
            format!("{text} {word}")
        }
        Unit::Bytes => bytes_text(x, 1),
        Unit::Duration => duration_text(x, false),
        Unit::Ratio => percent(x)?,
    })
}

/// An estimate of 1,000 or more in thousands ("6.7k", "115k") or millions
/// ("1.2M", "15M"), one decimal while under ten; `None` below 1,000, which
/// keeps its digits. Each step is chosen after rounding, so 9,960 is "10k",
/// not "10.0k", and 999,600 is "1M".
fn shortened(x: Decimal) -> Result<Option<String>, FactError> {
    let int = |n: i128| {
        Decimal::from_integer(n).map_err(|_| FactError::Range("a number the language can't hold"))
    };
    let held =
        |r: Result<Decimal, _>| r.map_err(|_| FactError::Range("a number the language can't hold"));
    let (thousand, ten) = (int(1000)?, int(10)?);
    if x.abs() < thousand {
        return Ok(None);
    }
    for (scale, suffix) in [(thousand, "k"), (int(1_000_000)?, "M")] {
        let scaled = held(x.checked_div(scale))?;
        let tenths = held(scaled.round(int(1)?))?;
        if tenths.abs() < ten {
            return Ok(Some(format!(
                "{}{suffix}",
                number_text(tenths, None, false)
            )));
        }
        let whole = held(scaled.round(Decimal::ZERO))?;
        if whole.abs() < thousand || suffix == "M" {
            return Ok(Some(format!("{}{suffix}", number_text(whole, None, true))));
        }
    }
    Ok(None)
}

/// A share as a whole percent, rounded half away from zero: 0.615 is "62%".
fn percent(share: Decimal) -> Result<String, FactError> {
    let scaled = share
        .checked_mul(Decimal::from_scaled(100, 0))
        .map_err(|_| FactError::Range("a number the language can't hold"))?;
    Ok(format!("{}%", number_text(scaled, Some(0), true)))
}

#[cfg(test)]
mod tests;
