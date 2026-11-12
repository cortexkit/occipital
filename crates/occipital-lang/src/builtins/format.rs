//! Fixed English formatting with integer arithmetic only (§6.4).
mod date;

use super::*;
use std::fmt;

const SCALE: u128 = 1_000_000_000_000_000_000;

/// The format built-ins.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Op {
    Number,
    Bytes,
    Duration,
    Date,
    Relative,
}

/// `ParseDate(text)`: milliseconds since the epoch, or `E_DATE_PARSE`.
pub(super) fn parse_date<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    let text = cx.string(&args[0])?;
    cx.pay(text.len().div_ceil(8))?;
    let ms = date::parse(text.as_str()).ok_or_else(|| cx.error("E_DATE_PARSE"))?;
    Ok(Value::number(Decimal::from_i64(ms)))
}

pub(super) fn call<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    builtin: Op,
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    use Op::*;
    cx.pay(10)?;
    let x = cx.number(&args[0])?;
    match builtin {
        Number => {
            let digits = args
                .get(1)
                .filter(|v| !v.is_absent())
                .map(|d| digits_arg(d, 18, cx))
                .transpose()?;
            let grouping = match args.get(2) {
                None => true,
                Some(v) => match v.kind() {
                    ValueKind::Boolean(b) => *b,
                    _ => return Err(cx.error("E_TYPE")),
                },
            };
            formatted(cx, |out| {
                number_into(out, x, digits, grouping);
                Ok(())
            })
        }
        Bytes => {
            let digits = args
                .get(1)
                .map(|v| digits_arg(v, 3, cx))
                .transpose()?
                .unwrap_or(1);
            formatted(cx, |out| {
                bytes_into(out, x, digits);
                Ok(())
            })
        }
        Duration => {
            let style = args
                .get(1)
                .map(|v| cx.string(v))
                .transpose()?
                .map(|s| s.as_str())
                .unwrap_or("compact");
            let clock = match style {
                "compact" => false,
                "clock" => true,
                _ => return Err(cx.error("E_ARG")),
            };
            formatted(cx, |out| {
                duration_into(out, x, clock);
                Ok(())
            })
        }
        Date => {
            let pattern = args
                .get(1)
                .map(|v| cx.string(v))
                .transpose()?
                .map(|s| s.as_str())
                .unwrap_or("date");
            let zone = args
                .get(2)
                .map(|v| cx.string(v))
                .transpose()?
                .map(|s| s.as_str())
                .unwrap_or("UTC");
            let offset = date::offset(zone).ok_or_else(|| cx.error("E_ARG"))?;
            let date = date::Date::new(x.coefficient().div_euclid(SCALE as i128), offset)
                .ok_or_else(|| cx.error("E_ARG"))?;
            formatted(cx, |out| date.render(pattern, out))
        }
        Relative => {
            let now = cx.number(&args[1])?;
            // The difference can exceed the Decimal range, but fits an unsigned coefficient.
            let a = now.coefficient().abs_diff(x.coefficient());
            let second = 1000 * SCALE;
            if a >= 26 * 86400 * second {
                let date = date::Date::new(x.coefficient().div_euclid(SCALE as i128), 0)
                    .ok_or_else(|| cx.error("E_ARG"))?;
                return formatted(cx, |out| date.render("medium", out));
            }
            formatted(cx, |out| {
                if a < 45 * second {
                    out.push_str("just now");
                    return Ok(());
                }
                let (count, unit) = if a < 90 * second {
                    (1, "minute")
                } else if a < 45 * 60 * second {
                    (round_ratio(a, 60 * second), "minute")
                } else if a < 90 * 60 * second {
                    (1, "hour")
                } else if a < 22 * 3600 * second {
                    (round_ratio(a, 3600 * second), "hour")
                } else if a < 36 * 3600 * second {
                    (1, "day")
                } else {
                    (round_ratio(a, 86400 * second), "day")
                };
                if now < x {
                    out.push_str("in ");
                }
                write!(out, "{count} {unit}{}", if count == 1 { "" } else { "s" });
                if now >= x {
                    out.push_str(" ago");
                }
                Ok(())
            })
        }
    }
}

/// `FormatNumber`: `x` in canonical form, or rounded to `digits` places, with
/// thousands grouped by "," when `grouping` is on.
fn number_into(out: &mut Sink, x: Decimal, digits: Option<u32>, grouping: bool) {
    let text = if let Some(d) = digits {
        // Text may round beyond the Decimal range without creating a numeric value.
        let rounded = round_ratio(x.coefficient().unsigned_abs(), SCALE / 10_u128.pow(d));
        let mut text = fixed(rounded, d);
        if x.coefficient() < 0 && rounded != 0 {
            text.insert(0, '-');
        }
        text
    } else {
        x.to_string()
    };
    let end = text.find('.').unwrap_or(text.len());
    for (i, ch) in text.char_indices() {
        if grouping
            && i > usize::from(text.starts_with('-'))
            && i < end
            && (end - i).is_multiple_of(3)
        {
            out.push(',');
        }
        out.push(ch);
    }
}

/// `FormatBytes`: binary units with at most `digits` places, trailing zeros
/// dropped.
fn bytes_into(out: &mut Sink, x: Decimal, digits: u32) {
    let magnitude = x.coefficient().unsigned_abs();
    let mut unit = 0;
    while unit < 6 && magnitude / SCALE >= 1024_u128.pow(unit + 1) {
        unit += 1;
    }
    let places = if unit == 0 { 0 } else { digits };
    let mut rounded = round_ratio(
        magnitude,
        1024_u128.pow(unit) * (SCALE / 10_u128.pow(places)),
    );
    if unit > 0 && unit < 6 && rounded >= 1024 * 10_u128.pow(places) {
        unit += 1;
        rounded = round_ratio(
            magnitude,
            1024_u128.pow(unit) * (SCALE / 10_u128.pow(places)),
        );
    }
    let mut text = fixed(rounded, places);
    if places > 0 {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if x.coefficient() < 0 && rounded != 0 {
        text.insert(0, '-');
    }
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    write!(out, "{text} {}", UNITS[unit as usize]);
}

/// `FormatDuration` of `x` milliseconds: "m:ss" or "h:mm:ss" with `clock`,
/// otherwise the compact form ("920 ms", "2.4 s", "1m 5s", "2h 3m", "1d 2h").
fn duration_into(out: &mut Sink, x: Decimal, clock: bool) {
    let a = x.coefficient().unsigned_abs();
    if x.coefficient() < 0 {
        out.push('-');
    }
    let seconds = round_ratio(a, 1000 * SCALE);
    if clock {
        if seconds >= 3600 {
            write!(
                out,
                "{}:{:02}:{:02}",
                seconds / 3600,
                seconds % 3600 / 60,
                seconds % 60
            );
        } else {
            write!(out, "{}:{:02}", seconds / 60, seconds % 60);
        }
    } else if a < 1000 * SCALE {
        write!(out, "{} ms", round_ratio(a, SCALE));
    } else {
        let tenths = round_ratio(a, 100 * SCALE);
        if tenths < 600 {
            write!(out, "{} s", fixed(tenths, 1));
        } else if seconds < 3600 {
            write!(out, "{}m {}s", seconds / 60, seconds % 60);
        } else if seconds < 86400 {
            write!(out, "{}h {}m", seconds / 3600, seconds % 3600 / 60);
        } else {
            write!(out, "{}d {}h", seconds / 86400, seconds % 86400 / 3600);
        }
    }
}

/// The same three formats as text, for callers outside an evaluation (the
/// fact formatter), which have no fuel or string limit to charge.
pub(crate) fn number_text(x: Decimal, digits: Option<u32>, grouping: bool) -> String {
    let mut out = Sink::new(false);
    number_into(&mut out, x, digits, grouping);
    out.text
}

pub(crate) fn bytes_text(x: Decimal, digits: u32) -> String {
    let mut out = Sink::new(false);
    bytes_into(&mut out, x, digits);
    out.text
}

pub(crate) fn duration_text(x: Decimal, clock: bool) -> String {
    let mut out = Sink::new(false);
    duration_into(&mut out, x, clock);
    out.text
}

fn digits_arg<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    value: &Value,
    max: u32,
    cx: &Context<'_, E, F>,
) -> Result<u32, E> {
    cx.number(value)?
        .integer()
        .filter(|d| (0..=i128::from(max)).contains(d))
        .map(|d| d as u32)
        .ok_or_else(|| cx.error("E_ARG"))
}

// Round the exact nonnegative ratio, without first rounding a decimal quotient.
fn round_ratio(numerator: u128, denominator: u128) -> u128 {
    let (q, r) = (numerator / denominator, numerator % denominator);
    q + u128::from(r >= denominator - r)
}

fn fixed(value: u128, places: u32) -> String {
    if places == 0 {
        return value.to_string();
    }
    let scale = 10_u128.pow(places);
    format!(
        "{}.{:0width$}",
        value / scale,
        value % scale,
        width = places as usize
    )
}

/// Where a formatter writes: first only a scalar count for the limit check,
/// then the text itself. Neither can fail, so rendering has no write errors.
struct Sink {
    count_only: bool,
    length: u64,
    text: String,
}

impl Sink {
    fn new(count_only: bool) -> Self {
        Self {
            count_only,
            length: 0,
            text: String::new(),
        }
    }

    fn push_str(&mut self, s: &str) {
        self.length += s.chars().count() as u64;
        if !self.count_only {
            self.text.push_str(s);
        }
    }

    fn push(&mut self, c: char) {
        self.length += 1;
        if !self.count_only {
            self.text.push(c);
        }
    }

    /// Lets `write!` render into the sink. Formatting into a `String`
    /// cannot fail, so this returns nothing.
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) {
        match args.as_str() {
            Some(text) => self.push_str(text),
            None => self.push_str(&fmt::format(args)),
        }
    }
}

fn formatted<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    cx: &mut Context<'_, E, F>,
    mut render: impl FnMut(&mut Sink) -> std::result::Result<(), &'static str>,
) -> Result<Value, E> {
    // Token expansion may be longer than the pattern. Check and charge before copying it.
    let mut length = Sink::new(true);
    render(&mut length).map_err(|code| cx.error(code))?;
    cx.string_limit(length.length)?;
    cx.pay(length.length.div_ceil(8))?;
    let mut output = Sink::new(false);
    render(&mut output).map_err(|code| cx.error(code))?;
    cx.build_string(output.text)
}
