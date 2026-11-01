//! Proleptic Gregorian dates and the closed set of date patterns in §6.4.
use super::Sink;

const DAY: i128 = 86_400_000;
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// One field a pattern token renders.
#[derive(Clone, Copy)]
enum Field {
    Year,
    MonthName,
    MonthShort,
    Weekday,
    WeekdayShort,
    Month2,
    Month,
    Day2,
    Day,
    Hour2,
    Hour,
    Minute2,
    Second2,
}

/// The pattern tokens. Prefixes share letters, so a longer token comes
/// before its shorter form and wins.
const TOKENS: [(&str, Field); 13] = [
    ("YYYY", Field::Year),
    ("MMMM", Field::MonthName),
    ("dddd", Field::Weekday),
    ("MMM", Field::MonthShort),
    ("ddd", Field::WeekdayShort),
    ("MM", Field::Month2),
    ("DD", Field::Day2),
    ("HH", Field::Hour2),
    ("mm", Field::Minute2),
    ("ss", Field::Second2),
    ("M", Field::Month),
    ("D", Field::Day),
    ("H", Field::Hour),
];

pub(super) struct Date {
    year: i128,
    month: i128,
    day: i128,
    weekday: usize,
    hour: i128,
    minute: i128,
    second: i128,
}

impl Date {
    pub(super) fn new(ms: i128, offset: i128) -> Option<Self> {
        let local = ms + offset * 60_000;
        let days = local.div_euclid(DAY);
        let time = local.rem_euclid(DAY);
        let (year, month, day) = civil(days);
        (1..=9999).contains(&year).then_some(Self {
            year,
            month,
            day,
            weekday: (days + 3).rem_euclid(7) as usize,
            hour: time / 3_600_000,
            minute: time % 3_600_000 / 60_000,
            second: time % 60_000 / 1000,
        })
    }

    pub(super) fn render(&self, pattern: &str, out: &mut Sink) -> Result<(), &'static str> {
        let pattern = match pattern {
            "date" => "YYYY-MM-DD",
            "time" => "HH:mm",
            "datetime" => "YYYY-MM-DD HH:mm",
            "medium" => "D MMM YYYY",
            "monthYear" => "MMM YYYY",
            _ => pattern,
        };
        let mut rest = pattern;
        let mut quoted = false;
        loop {
            if let Some(suffix) = rest.strip_prefix("''") {
                out.push('\'');
                rest = suffix;
                continue;
            }
            let Some(ch) = rest.chars().next() else {
                break;
            };
            if ch == '\'' {
                quoted = !quoted;
                rest = &rest[1..];
                continue;
            }
            if quoted || !ch.is_ascii_alphabetic() {
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
                continue;
            }
            let (token, field) = TOKENS
                .into_iter()
                .find(|(token, _)| rest.starts_with(token))
                .ok_or("E_ARG")?;
            match field {
                Field::Year => write!(out, "{:04}", self.year),
                Field::MonthName => out.push_str(MONTHS[self.month as usize - 1]),
                Field::MonthShort => out.push_str(&MONTHS[self.month as usize - 1][..3]),
                Field::Weekday => out.push_str(WEEKDAYS[self.weekday]),
                Field::WeekdayShort => out.push_str(&WEEKDAYS[self.weekday][..3]),
                Field::Month2 => write!(out, "{:02}", self.month),
                Field::Month => write!(out, "{}", self.month),
                Field::Day2 => write!(out, "{:02}", self.day),
                Field::Day => write!(out, "{}", self.day),
                Field::Hour2 => write!(out, "{:02}", self.hour),
                Field::Hour => write!(out, "{}", self.hour),
                Field::Minute2 => write!(out, "{:02}", self.minute),
                Field::Second2 => write!(out, "{:02}", self.second),
            }
            rest = &rest[token.len()..];
        }
        if quoted {
            Err("E_ARG")
        } else {
            Ok(())
        }
    }
}

/// Offset in minutes; the sign and width are part of the accepted grammar.
pub(super) fn offset(zone: &str) -> Option<i128> {
    if matches!(zone, "UTC" | "Z") {
        return Some(0);
    }
    let b = zone.as_bytes();
    if b.len() != 6 || !matches!(b[0], b'+' | b'-') || b[3] != b':' {
        return None;
    }
    let hour = digits(b, 1, 2)?;
    let minute = digits(b, 4, 2)?;
    if hour > 14 || minute > 59 || (hour == 14 && minute != 0) {
        return None;
    }
    Some((hour * 60 + minute) * if b[0] == b'-' { -1 } else { 1 })
}

/// Milliseconds since the epoch. Years 1..=9999 keep every result far
/// inside `i64`, so the caller can make a number without a fallible call.
pub(super) fn parse(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    let year = digits(b, 0, 4)?;
    let month = digits(b, 5, 2)?;
    if b.get(4) != Some(&b'-') || year == 0 || !(1..=12).contains(&month) {
        return None;
    }
    let day = if b.len() == 7 {
        1
    } else {
        if b.get(7) != Some(&b'-') {
            return None;
        }
        digits(b, 8, 2)?
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days_in_month).contains(&day) {
        return None;
    }
    let mut ms = days(year, month, day) * DAY;
    if matches!(b.len(), 7 | 10) {
        return i64::try_from(ms).ok();
    }
    if b.get(10) != Some(&b'T') || b.get(13) != Some(&b':') || b.get(16) != Some(&b':') {
        return None;
    }
    let hour = digits(b, 11, 2)?;
    let minute = digits(b, 14, 2)?;
    let second = digits(b, 17, 2)?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    ms += hour * 3_600_000 + minute * 60_000 + second * 1000;
    let mut end = 19;
    if b.get(end) == Some(&b'.') {
        end += 1;
        let start = end;
        let mut place = 100;
        while let Some(digit) = b.get(end).filter(|c| c.is_ascii_digit()) {
            ms += i128::from(*digit - b'0') * place;
            place /= 10;
            end += 1;
        }
        if end == start {
            return None;
        }
    }
    let zone = text.get(end..)?;
    if zone == "-00:00" || zone == "UTC" {
        return None;
    }
    ms -= offset(zone)? * 60_000;
    Date::new(ms, 0)?;
    i64::try_from(ms).ok()
}

fn digits(bytes: &[u8], start: usize, count: usize) -> Option<i128> {
    bytes
        .get(start..start + count)?
        .iter()
        .try_fold(0, |n, ch| {
            ch.is_ascii_digit().then(|| n * 10 + i128::from(*ch - b'0'))
        })
}

// March-based 400-year eras keep leap-day arithmetic uniform, including before the epoch.
fn days(year: i128, month: i128, day: i128) -> i128 {
    let year = year - i128::from(month <= 2);
    let era = year.div_euclid(400);
    let y = year.rem_euclid(400);
    let month = month + if month > 2 { -3 } else { 9 };
    era * 146_097 + y * 365 + y / 4 - y / 100 + (153 * month + 2) / 5 + day - 1 - 719_468
}

fn civil(days: i128) -> (i128, i128, i128) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let d = days.rem_euclid(146_097);
    let y = (d - d / 1460 + d / 36_524 - d / 146_096) / 365;
    let day_of_year = d - (365 * y + y / 4 - y / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month + 2) / 5 + 1;
    let month = month + if month < 10 { 3 } else { -9 };
    (era * 400 + y + i128::from(month <= 2), month, day)
}
