use serde_json::json;

use super::*;

fn word(schedule: Json, shift: i32) -> String {
    format_schedule(&schedule, shift).unwrap()
}

fn code(schedule: Json, shift: i32) -> &'static str {
    format_schedule(&schedule, shift).unwrap_err().code()
}

const ZONE: &str = "Europe/Istanbul";

/// The four schedules in Magic Context's `mc-app-read-samples-v13`
/// (`dreamer-status.json` and `dreamer-status-forms.json`), read in their own
/// zone.
#[test]
fn magic_contexts_samples_word_as_the_apps_draw_them() {
    let cases = [
        (
            json!({"at": "06:00", "every": "day", "time_zone": ZONE}),
            Ok("Every day at 06:00"),
        ),
        (
            json!({"every": "hour", "minute": 0, "time_zone": ZONE}),
            Ok("Every hour"),
        ),
        (
            json!({"at": "04:30", "days": ["mon", "tue", "wed", "thu", "fri"], "every": "week", "time_zone": ZONE}),
            Ok("Weekdays at 04:30"),
        ),
        (
            json!({"cron": "15 */2 * * *", "time_zone": ZONE}),
            Err("schedule_unworded"),
        ),
    ];
    for (schedule, expected) in cases {
        let got = format_schedule(&schedule, 0).map_err(|e| e.code());
        assert_eq!(got.as_deref(), expected.as_deref(), "{schedule}");
    }
}

#[test]
fn days_are_worded_by_how_many_and_which() {
    let week =
        |days: &[&str]| json!({"every": "week", "days": days, "at": "09:30", "time_zone": ZONE});
    assert_eq!(word(week(&["mon"]), 0), "Mondays at 09:30");
    assert_eq!(
        word(week(&["mon", "thu"]), 0),
        "Mondays and Thursdays at 09:30"
    );
    assert_eq!(
        word(week(&["mon", "wed", "fri"]), 0),
        "Mon, Wed and Fri at 09:30"
    );
    assert_eq!(word(week(&["sat", "sun"]), 0), "Weekends at 09:30");
    assert_eq!(
        word(week(&["mon", "tue", "wed", "thu", "fri", "sat", "sun"]), 0),
        "Every day at 09:30"
    );
    assert_eq!(
        word(json!({"every": "hour", "minute": 15, "time_zone": ZONE}), 0),
        "Every hour at :15"
    );
}

/// The shift moves the time into the person's clock, and the days with it
/// when the time crosses midnight either way.
#[test]
fn a_shift_moves_the_time_and_carries_the_days_across_midnight() {
    let week = |days: &[&str], at: &str| json!({"every": "week", "days": days, "at": at, "time_zone": ZONE});
    assert_eq!(word(week(&["mon"], "23:30"), 60), "Tuesdays at 00:30");
    assert_eq!(word(week(&["mon"], "00:30"), -60), "Sundays at 23:30");
    assert_eq!(word(week(&["sun"], "23:30"), 60), "Mondays at 00:30");
    // Friday and Saturday a day later are the weekend.
    assert_eq!(
        word(week(&["fri", "sat"], "22:00"), 180),
        "Weekends at 01:00"
    );
    assert_eq!(
        word(
            json!({"every": "day", "at": "03:00", "time_zone": ZONE}),
            -180
        ),
        "Every day at 00:00"
    );
    // A whole-hour shift leaves an hourly minute alone; a half hour moves it.
    let hourly = json!({"every": "hour", "minute": 15, "time_zone": ZONE});
    assert_eq!(word(hourly.clone(), -180), "Every hour at :15");
    assert_eq!(word(hourly, 330), "Every hour at :45");
    assert_eq!(
        word(
            json!({"every": "hour", "minute": 30, "time_zone": ZONE}),
            30
        ),
        "Every hour"
    );
}

#[test]
fn shapes_outside_the_three_forms_are_refused() {
    let refused = [
        json!({"every": "day", "at": "3:00", "time_zone": ZONE}),
        json!({"every": "day", "at": "24:00", "time_zone": ZONE}),
        json!({"every": "day", "time_zone": ZONE}),
        json!({"every": "hour", "minute": 60, "time_zone": ZONE}),
        json!({"every": "hour", "minute": 5, "at": "01:00", "time_zone": ZONE}),
        json!({"every": "week", "days": [], "at": "09:00", "time_zone": ZONE}),
        json!({"every": "week", "days": ["thu", "mon"], "at": "09:00", "time_zone": ZONE}),
        json!({"every": "week", "days": ["mon", "mon"], "at": "09:00", "time_zone": ZONE}),
        json!({"every": "week", "days": ["monday"], "at": "09:00", "time_zone": ZONE}),
        json!({"every": "month", "at": "09:00", "time_zone": ZONE}),
        json!({"every": "day", "at": "09:00"}),
        json!({"every": "day", "at": "09:00", "time_zone": ""}),
        json!({"every": "day", "at": "09:00", "time_zone": ZONE, "extra": 1}),
        json!({"cron": "0 3 * * *", "every": "day", "at": "03:00", "time_zone": ZONE}),
    ];
    for schedule in refused {
        assert_eq!(
            code(schedule.clone(), 0),
            "schedule_malformed",
            "{schedule}"
        );
    }
    let daily = json!({"every": "day", "at": "09:00", "time_zone": ZONE});
    assert_eq!(code(daily.clone(), 26 * 60 + 1), "schedule_malformed");
    assert_eq!(word(daily, -26 * 60), "Every day at 07:00");
}
