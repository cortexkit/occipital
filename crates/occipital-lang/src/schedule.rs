//! Schedules: when a module's recurring task runs, worded the same way by
//! every app.
//!
//! A module reports a schedule in one of three closed forms, each with
//! `time_zone`, the IANA name of the zone it runs in (the module's machine):
//! `{"every": "hour", "minute": 15}`, `{"every": "day", "at": "03:00"}` or
//! `{"every": "week", "days": ["mon", "thu"], "at": "09:30"}`. A schedule
//! none of those can express arrives as `{"cron": "...", "time_zone"}`,
//! which this module refuses so each app draws its own fallback.
//!
//! The wording is in the person's own clock. Converting between zones needs
//! the zone rules, which this dependency-light library doesn't carry and the
//! apps do, so the app passes the shift: its device's UTC offset minus the
//! schedule zone's UTC offset, both now, in minutes. The library moves the
//! time by it, and the days with it when the time crosses midnight.

use serde::Deserialize;
use serde_json::Value as Json;

/// The widest real shift: UTC+14 against UTC-12.
const MAX_SHIFT_MINUTES: i32 = 26 * 60;

const MINUTES_PER_DAY: i32 = 24 * 60;

/// Days in week order, Monday first, with their names as a schedule spells
/// them, their plural for one or two days, and their short form for more.
const DAYS: [(&str, &str, &str); 7] = [
    ("mon", "Mondays", "Mon"),
    ("tue", "Tuesdays", "Tue"),
    ("wed", "Wednesdays", "Wed"),
    ("thu", "Thursdays", "Thu"),
    ("fri", "Fridays", "Fri"),
    ("sat", "Saturdays", "Sat"),
    ("sun", "Sundays", "Sun"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    every: Option<String>,
    minute: Option<i64>,
    at: Option<String>,
    days: Option<Vec<String>>,
    cron: Option<String>,
    time_zone: String,
}

/// Why a schedule has no wording. An app shows its own fallback, such as the
/// cron text, rather than guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    /// Not one of the three forms: a missing or unknown member, a member of
    /// the wrong type, an `at` that isn't `HH:MM`, a minute outside 0 to 59,
    /// days empty, repeated, out of week order or unknown, or a shift wider
    /// than any two zones differ.
    Malformed(String),
    /// A cron-only schedule: the module couldn't express it in a closed
    /// form, so there is no wording for it.
    Unworded,
}

impl ScheduleError {
    /// The stable code apps and the C binding report.
    pub fn code(&self) -> &'static str {
        match self {
            ScheduleError::Malformed(_) => "schedule_malformed",
            ScheduleError::Unworded => "schedule_unworded",
        }
    }
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::Malformed(detail) => write!(f, "{}: {detail}", self.code()),
            ScheduleError::Unworded => f.write_str(self.code()),
        }
    }
}

fn malformed(detail: impl Into<String>) -> ScheduleError {
    ScheduleError::Malformed(detail.into())
}

/// Words one schedule in the person's clock, `shift_minutes` from the
/// schedule's zone (the device's UTC offset minus the zone's, both now).
///
/// - `hour`: "Every hour" on the hour, else "Every hour at :15". A whole-hour
///   shift leaves it as it is; a half-hour one moves the minute.
/// - `day`: "Every day at 03:00", on a 24-hour clock.
/// - `week`: one or two days by name ("Mondays at 09:30", "Mondays and
///   Thursdays at 09:30"), Monday to Friday as "Weekdays", Saturday and
///   Sunday as "Weekends", all seven as "Every day", and any other three to
///   six by short name ("Mon, Wed and Fri at 09:30"). When the shift moves
///   the time past midnight, every day moves with it, so 23:30 on Monday in
///   Istanbul is "Tuesdays at 00:30" one hour east.
///
/// A cron-only schedule is [`ScheduleError::Unworded`]; any other shape is
/// [`ScheduleError::Malformed`].
pub fn format_schedule(schedule: &Json, shift_minutes: i32) -> Result<String, ScheduleError> {
    if shift_minutes.abs() > MAX_SHIFT_MINUTES {
        return Err(malformed("shift must be within 26 hours"));
    }
    let wire: Wire =
        serde_json::from_value(schedule.clone()).map_err(|e| malformed(e.to_string()))?;
    if wire.time_zone.is_empty() {
        return Err(malformed("time_zone must name a zone"));
    }
    if wire.cron.is_some() {
        return match (&wire.every, &wire.minute, &wire.at, &wire.days) {
            (None, None, None, None) => Err(ScheduleError::Unworded),
            _ => Err(malformed("cron stands alone")),
        };
    }
    let every = wire.every.as_deref().ok_or_else(|| malformed("every"))?;
    match every {
        "hour" => {
            if wire.at.is_some() || wire.days.is_some() {
                return Err(malformed("an hourly schedule has only minute"));
            }
            let minute = wire.minute.ok_or_else(|| malformed("minute"))?;
            if !(0..60).contains(&minute) {
                return Err(malformed("minute must be 0 to 59"));
            }
            // `minute` is 0..60 and the shift within 26 hours, so the sum
            // fits an i32.
            let shifted = (minute as i32 + shift_minutes).rem_euclid(60);
            Ok(if shifted == 0 {
                "Every hour".to_owned()
            } else {
                format!("Every hour at :{shifted:02}")
            })
        }
        "day" => {
            if wire.minute.is_some() || wire.days.is_some() {
                return Err(malformed("a daily schedule has only at"));
            }
            let at = clock(wire.at.as_deref())?;
            let (_, time) = shift(at, shift_minutes);
            Ok(format!("Every day at {}", hh_mm(time)))
        }
        "week" => {
            if wire.minute.is_some() {
                return Err(malformed("a weekly schedule has days and at"));
            }
            let at = clock(wire.at.as_deref())?;
            let days = week_days(wire.days.as_deref())?;
            let (moved, time) = shift(at, shift_minutes);
            let mut shifted: Vec<usize> = days
                .iter()
                // `moved` is -2..=2, so the sum is never below -2.
                .map(|day| (*day as i32 + moved).rem_euclid(7) as usize)
                .collect();
            shifted.sort_unstable();
            Ok(format!("{} at {}", days_words(&shifted), hh_mm(time)))
        }
        other => Err(malformed(format!("unknown every {other:?}"))),
    }
}

/// `HH:MM` as minutes after midnight.
fn clock(at: Option<&str>) -> Result<i32, ScheduleError> {
    let at = at.ok_or_else(|| malformed("at"))?;
    let bytes = at.as_bytes();
    let digit = |i: usize| -> Result<i32, ScheduleError> {
        bytes
            .get(i)
            .filter(|b| b.is_ascii_digit())
            .map(|b| i32::from(b - b'0'))
            .ok_or_else(|| malformed("at must be HH:MM"))
    };
    if bytes.len() != 5 || bytes[2] != b':' {
        return Err(malformed("at must be HH:MM"));
    }
    let (hours, minutes) = (digit(0)? * 10 + digit(1)?, digit(3)? * 10 + digit(4)?);
    if hours > 23 || minutes > 59 {
        return Err(malformed("at must be 00:00 to 23:59"));
    }
    Ok(hours * 60 + minutes)
}

/// The days a weekly schedule names, as indexes into [`DAYS`]: non-empty,
/// known, and in week order with none repeated.
fn week_days(days: Option<&[String]>) -> Result<Vec<usize>, ScheduleError> {
    let days = days.ok_or_else(|| malformed("days"))?;
    if days.is_empty() {
        return Err(malformed("days must name at least one day"));
    }
    let mut indexes = Vec::with_capacity(days.len());
    for day in days {
        let index = DAYS
            .iter()
            .position(|(name, _, _)| name == day)
            .ok_or_else(|| malformed(format!("unknown day {day:?}")))?;
        if indexes.last().is_some_and(|last| *last >= index) {
            return Err(malformed("days must be in week order, Monday first"));
        }
        indexes.push(index);
    }
    Ok(indexes)
}

/// The time moved by `shift`: how many days it crossed (-2 to 2) and the
/// time on the day it lands.
fn shift(at: i32, shift: i32) -> (i32, i32) {
    let total = at + shift;
    (
        total.div_euclid(MINUTES_PER_DAY),
        total.rem_euclid(MINUTES_PER_DAY),
    )
}

fn hh_mm(minutes: i32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Sorted day indexes as words.
fn days_words(days: &[usize]) -> String {
    match days {
        [0, 1, 2, 3, 4, 5, 6] => "Every day".to_owned(),
        [0, 1, 2, 3, 4] => "Weekdays".to_owned(),
        [5, 6] => "Weekends".to_owned(),
        [one] => DAYS[*one].1.to_owned(),
        [first, second] => format!("{} and {}", DAYS[*first].1, DAYS[*second].1),
        [rest @ .., last] => {
            let rest: Vec<&str> = rest.iter().map(|day| DAYS[*day].2).collect();
            format!("{} and {}", rest.join(", "), DAYS[*last].2)
        }
        [] => String::new(),
    }
}

#[cfg(test)]
mod tests;
