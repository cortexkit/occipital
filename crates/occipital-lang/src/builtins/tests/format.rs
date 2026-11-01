use super::*;

fn formats(builtin: Builtin, args: &[Value], expected: &str) {
    check(
        builtin,
        args,
        s(expected),
        11 + (expected.chars().count() as u64).div_ceil(8),
    );
}

#[test]
fn number_precision_grouping_sign_and_range_are_text_only() {
    use Builtin::FormatNumber as F;
    for (input, expected) in [
        ("1234.5", "1,234.5"),
        ("-1234567.00001", "-1,234,567.00001"),
        ("0", "0"),
        ("0.000000000000000001", "0.000000000000000001"),
    ] {
        formats(F, &[n(input)], expected);
    }
    for (input, digits, expected) in [
        ("4", "1", "4.0"),
        ("-0.04", "1", "0.0"),
        ("-0.5", "0", "-1"),
        ("2.675", "2", "2.68"),
        ("-2.675", "2", "-2.68"),
        ("999.95", "1", "1,000.0"),
        ("1.25", "18", "1.250000000000000000"),
        (
            "99999999999999999999.999999999999999999",
            "0",
            "100,000,000,000,000,000,000",
        ),
    ] {
        formats(F, &[n(input), n(digits)], expected);
    }
    for absent in [Value::null(), Value::missing()] {
        formats(F, &[n("1234.5"), absent.clone()], "1,234.5");
        formats(F, &[n("1234.5"), absent, Value::boolean(false)], "1234.5");
    }
    formats(F, &[n("-1234"), n("2"), Value::boolean(false)], "-1234.00");
    for d in ["-1", "19", "1.1"] {
        fails(F, &[n("1"), n(d)], "E_ARG");
    }
    fails(F, &[n("1"), s("1")], "E_TYPE");
    fails(F, &[n("1"), Value::null(), Value::null()], "E_TYPE");
}

#[test]
fn bytes_iec_units_rounding_promotion_and_precision() {
    use Builtin::FormatBytes as F;
    for (input, expected) in [
        ("1536", "1.5 KiB"),
        ("2097152", "2 MiB"),
        ("0", "0 B"),
        ("-0.04", "0 B"),
        ("-0.5", "-1 B"),
        ("1023.5", "1024 B"),
        ("1024", "1 KiB"),
        ("-1536", "-1.5 KiB"),
        ("1073741824", "1 GiB"),
        ("1099511627776", "1 TiB"),
        ("1125899906842624", "1 PiB"),
        ("1152921504606846976", "1 EiB"),
        ("1048524.8", "1 MiB"),
        ("1048524.799999999999999999", "1023.9 KiB"),
        ("99999999999999999999.999999999999999999", "86.7 EiB"),
    ] {
        formats(F, &[n(input)], expected);
    }
    for (input, digits, expected) in [
        ("1536", "0", "2 KiB"),
        ("-1536", "0", "-2 KiB"),
        ("1025", "3", "1.001 KiB"),
        ("1280", "3", "1.25 KiB"),
        ("1023.9", "3", "1024 B"),
        ("1048575.488", "3", "1 MiB"),
        ("1048575.487999999999999999", "3", "1023.999 KiB"),
    ] {
        formats(F, &[n(input), n(digits)], expected);
    }
    for d in ["-1", "4", "0.5"] {
        fails(F, &[n("1"), n(d)], "E_ARG");
    }
    fails(F, &[n("1"), Value::null()], "E_TYPE");
}

#[test]
fn duration_compact_thresholds_and_signed_zero() {
    for (input, expected) in [
        ("0", "0 ms"),
        ("-0.04", "-0 ms"),
        ("0.5", "1 ms"),
        ("999.5", "1000 ms"),
        ("1000", "1.0 s"),
        ("4250", "4.3 s"),
        ("59949.999999999999999999", "59.9 s"),
        ("59950", "1m 0s"),
        ("3599499.999999999999999999", "59m 59s"),
        ("3599500", "1h 0m"),
        ("86399499.999999999999999999", "23h 59m"),
        ("86399500", "1d 0h"),
        ("-90061000", "-1d 1h"),
        (
            "99999999999999999999.999999999999999999",
            "1157407407407d 9h",
        ),
    ] {
        formats(Builtin::FormatDuration, &[n(input)], expected);
    }
    formats(Builtin::FormatDuration, &[n("4250"), s("compact")], "4.3 s");
}

#[test]
fn duration_clock_uses_exact_integer_quotients_and_remainders() {
    for (input, expected) in [
        ("499.999999999999999999", "0:00"),
        ("500", "0:01"),
        ("59999.999999999999999999", "1:00"),
        ("3599499.9", "59:59"),
        ("3599500", "1:00:00"),
        ("90061000", "25:01:01"),
        ("-0.01", "-0:00"),
        ("-3661000", "-1:01:01"),
        (
            "99999999999999999999.999999999999999999",
            "27777777777777:46:40",
        ),
    ] {
        formats(Builtin::FormatDuration, &[n(input), s("clock")], expected);
    }
    fails(Builtin::FormatDuration, &[n("0"), s("Clock")], "E_ARG");
    fails(Builtin::FormatDuration, &[n("0"), n("1")], "E_TYPE");
}

#[test]
fn date_presets_tokens_literals_and_explicit_offsets() {
    use Builtin::FormatDate as F;
    for (pattern, expected) in [
        ("date", "2025-03-14"),
        ("time", "09:05"),
        ("datetime", "2025-03-14 09:05"),
        ("medium", "14 Mar 2025"),
        ("monthYear", "Mar 2025"),
        ("MMMMMDDDDHHH", "March31414099"),
        (
            "YYYY MMMM MMM MM M DD D dddd ddd HH H mm ss",
            "2025 March Mar 03 3 14 14 Friday Fri 09 9 05 00",
        ),
        (
            "'Year:' YYYY 'at' HH:mm '' 'it''s' 🦀",
            "Year: 2025 at 09:05 ' it's 🦀",
        ),
        ("", ""),
        ("年月日", "年月日"),
    ] {
        formats(F, &[n("1741943100000"), s(pattern)], expected);
    }
    formats(F, &[n("1741943100000")], "2025-03-14");
    for (zone, expected) in [
        ("UTC", "1970-01-01 00:00:00 Thursday"),
        ("Z", "1970-01-01 00:00:00 Thursday"),
        ("+14:00", "1970-01-01 14:00:00 Thursday"),
        ("-14:00", "1969-12-31 10:00:00 Wednesday"),
        ("+05:45", "1970-01-01 05:45:00 Thursday"),
        ("-00:00", "1970-01-01 00:00:00 Thursday"),
    ] {
        formats(
            F,
            &[n("0"), s("YYYY-MM-DD HH:mm:ss dddd"), s(zone)],
            expected,
        );
    }
    formats(
        F,
        &[n("-0.000000000000000001"), s("YYYY-MM-DD HH:mm:ss")],
        "1969-12-31 23:59:59",
    );
    formats(F, &[n("-62135596800000")], "0001-01-01");
    formats(F, &[n("253402300799999.999999999999999999")], "9999-12-31");
    formats(
        F,
        &[n("-62135596800001"), s("date"), s("+00:01")],
        "0001-01-01",
    );
    for year in ["-62135596800001", "253402300800000", "99999999999999999999"] {
        fails(F, &[n(year)], "E_ARG");
    }
    fails(F, &[n("253402300799999"), s("date"), s("+00:01")], "E_ARG");
    for zone in [
        "utc",
        "+14:01",
        "-14:01",
        "+15:00",
        "+01:60",
        "+1:00",
        "+00:0",
        "Europe/London",
        "",
        "💥",
    ] {
        fails(F, &[n("0"), s("date"), s(zone)], "E_ARG");
    }
    for pattern in ["YY", "hh", "m", "s", "foo", "'unterminated"] {
        fails(F, &[n("0"), s(pattern)], "E_ARG");
    }
}

#[test]
fn date_calendar_leaps_centuries_weekdays_and_names() {
    for (ms, expected) in [
        ("-2203977600000", "1900-02-28 Wednesday"),
        ("-2203891200000", "1900-03-01 Thursday"),
        ("951782400000", "2000-02-29 Tuesday"),
        ("951868800000", "2000-03-01 Wednesday"),
        ("4107542400000", "2100-03-01 Monday"),
    ] {
        formats(
            Builtin::FormatDate,
            &[n(ms), s("YYYY-MM-DD dddd")],
            expected,
        );
    }
    for (i, month) in [
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
    ]
    .iter()
    .enumerate()
    {
        let parsed = run(Builtin::ParseDate, &[s(&format!("2025-{:02}", i + 1))]).0;
        formats(
            Builtin::FormatDate,
            &[parsed, s("MMMM MMM")],
            &format!("{month} {}", &month[..3]),
        );
    }
    for (day, expected) in [
        "Thursday Thu",
        "Friday Fri",
        "Saturday Sat",
        "Sunday Sun",
        "Monday Mon",
        "Tuesday Tue",
        "Wednesday Wed",
    ]
    .iter()
    .enumerate()
    {
        formats(
            Builtin::FormatDate,
            &[n(&(day as u64 * 86400000).to_string()), s("dddd ddd")],
            expected,
        );
    }
}

#[test]
fn relative_thresholds_rounding_and_direction_take_explicit_now() {
    for (delta, past, future) in [
        ("0", "just now", "just now"),
        ("44999.999999999999999999", "just now", "just now"),
        ("45000", "1 minute ago", "in 1 minute"),
        ("89999.999999999999999999", "1 minute ago", "in 1 minute"),
        ("90000", "2 minutes ago", "in 2 minutes"),
        ("2669999", "44 minutes ago", "in 44 minutes"),
        ("2670000", "45 minutes ago", "in 45 minutes"),
        ("2699999.9", "45 minutes ago", "in 45 minutes"),
        ("2700000", "1 hour ago", "in 1 hour"),
        ("5399999.9", "1 hour ago", "in 1 hour"),
        ("5400000", "2 hours ago", "in 2 hours"),
        ("77399999", "21 hours ago", "in 21 hours"),
        ("77400000", "22 hours ago", "in 22 hours"),
        ("79199999.9", "22 hours ago", "in 22 hours"),
        ("79200000", "1 day ago", "in 1 day"),
        ("129599999.9", "1 day ago", "in 1 day"),
        ("129600000", "2 days ago", "in 2 days"),
        ("2203200000", "26 days ago", "in 26 days"),
        ("2246399999.9", "26 days ago", "in 26 days"),
        ("2246400000", "1 Jan 1970", "1 Jan 1970"),
    ] {
        formats(Builtin::FormatRelative, &[n("0"), n(delta)], past);
        formats(
            Builtin::FormatRelative,
            &[n("0"), n(&format!("-{delta}"))],
            future,
        );
    }
    formats(
        Builtin::FormatRelative,
        &[n("1741943100000"), n("0")],
        "14 Mar 2025",
    );
    formats(
        Builtin::FormatRelative,
        &[n("0"), n("99999999999999999999.999999999999999999")],
        "1 Jan 1970",
    );
    formats(
        Builtin::FormatRelative,
        &[n("0"), n("-99999999999999999999.999999999999999999")],
        "1 Jan 1970",
    );
    // An oversized difference must reach the date range check, not Decimal overflow.
    fails(
        Builtin::FormatRelative,
        &[n("-99999999999999999999"), n("99999999999999999999")],
        "E_ARG",
    );
}

#[test]
fn parse_date_exact_epoch_values_offsets_and_fraction_truncation() {
    for (input, ms) in [
        ("1970-01", "0"),
        ("1970-01-01", "0"),
        ("1970-01-01T00:00:00Z", "0"),
        ("2025-03-14T09:05:00Z", "1741943100000"),
        ("2025-03-14T12:05:00+03:00", "1741943100000"),
        ("1970-01-01T14:00:00+14:00", "0"),
        ("1969-12-31T10:00:00-14:00", "0"),
        ("1969-12-31T23:59:59.999999999Z", "-1"),
        ("1970-01-01T00:00:00.1+00:00", "100"),
        ("1970-01-01T00:00:00.12Z", "120"),
        (
            "1970-01-01T00:00:00.123999999999999999999999999999999999999Z",
            "123",
        ),
        ("1970-01-01T00:00:00.0009Z", "0"),
        ("2000-02-29", "951782400000"),
        ("1900-03-01", "-2203891200000"),
        ("0001-01", "-62135596800000"),
        ("9999-12-31T23:59:59.999Z", "253402300799999"),
        ("0001-01-01T14:00:00+14:00", "-62135596800000"),
        ("9999-12-31T09:59:59.999-14:00", "253402300799999"),
    ] {
        check(
            Builtin::ParseDate,
            &[s(input)],
            n(ms),
            1 + (input.len() as u64).div_ceil(8),
        );
    }
}

#[test]
fn parse_date_rejects_every_noncontract_form() {
    for input in [
        "",
        "2025",
        "2025-3",
        "2025-03-1",
        "2025-03-14Z",
        "2025-03+00:00",
        "2025-03-14 ",
        " 2025-03-14",
        "2025-03\n",
        "2025-03-14T09:05Z",
        "0000-01-01",
        "10000-01-01",
        "+2025-03-14",
        "-0001-01-01",
        "２０２５-03-14",
        "2025-00",
        "2025-13",
        "2025-02-29",
        "1900-02-29",
        "2100-02-29",
        "2025-04-31",
        "2025-01-00",
        "2025-01-32",
        "2025-01-011",
        "2025-03-14t09:05:00Z",
        "2025-03-14T09:05:00z",
        "2025-03-14 09:05:00Z",
        "2025-03-14T24:00:00Z",
        "2025-03-14T09:60:00Z",
        "2025-03-14T09:05:60Z",
        "2025-03-14T09:05:00",
        "2025-03-14T09:05:00UTC",
        "2025-03-14T09:05:00-00:00",
        "2025-03-14T09:05:00+14:01",
        "2025-03-14T09:05:00-14:01",
        "2025-03-14T09:05:00+15:00",
        "2025-03-14T09:05:00+01:60",
        "2025-03-14T09:05:00+1:00",
        "2025-03-14T09:05:00+0100",
        "2025-03-14T09:05:00.Z",
        "2025-03-14T09:05:00.1",
        "2025-03-14T09:05:00.١Z",
        "2025-03-14T09:05:00Ztail",
        "2025-03-14T09:05:00Z ",
        "0001-01-01T00:00:00+00:01",
        "9999-12-31T23:59:59.999-00:01",
    ] {
        fails(Builtin::ParseDate, &[s(input)], "E_DATE_PARSE");
    }
}

#[test]
fn formatting_arities_types_and_leftmost_errors_use_shared_dispatch() {
    for (builtin, min, max) in [
        (Builtin::FormatNumber, 1, 3),
        (Builtin::FormatBytes, 1, 2),
        (Builtin::FormatDuration, 1, 2),
        (Builtin::FormatDate, 1, 3),
        (Builtin::FormatRelative, 2, 2),
        (Builtin::ParseDate, 1, 1),
    ] {
        fails(builtin, &vec![n("0"); min - 1], "E_ARITY");
        fails(builtin, &vec![n("0"); max + 1], "E_ARITY");
        let mut args = vec![n("0"); min];
        args[0] = error("E_DOMAIN");
        fails(builtin, &args, "E_DOMAIN");
        for wrong in [
            Value::null(),
            Value::missing(),
            Value::boolean(false),
            list(vec![]),
        ] {
            args[0] = wrong;
            fails(builtin, &args, "E_TYPE");
        }
    }
    fails(Builtin::FormatNumber, &[s("1")], "E_TYPE");
    fails(Builtin::ParseDate, &[n("0")], "E_TYPE");
    fails(Builtin::FormatDate, &[n("0"), Value::null()], "E_TYPE");
    fails(
        Builtin::FormatDate,
        &[n("0"), s("date"), Value::null()],
        "E_TYPE",
    );
    fails(Builtin::FormatRelative, &[n("0"), Value::null()], "E_TYPE");
    fails(
        Builtin::FormatNumber,
        &[error("E_DOMAIN"), error("E_ARG")],
        "E_DOMAIN",
    );
}

#[test]
fn date_output_preflight_checks_expansion_and_scalar_limit() {
    let pattern = s(&"MMMM".repeat(10_000));
    limit(Builtin::FormatDate, &[n("0"), pattern], "E_LIMIT_STRING");
    limit(
        Builtin::FormatDate,
        &[n("0"), s(&"🦀".repeat(65537))],
        "E_LIMIT_STRING",
    );
    let pattern = "🦀".repeat(65536);
    formats(Builtin::FormatDate, &[n("0"), s(&pattern)], &pattern);
}

#[test]
fn formatting_fuel_aborts_before_output_or_parse_scan() {
    let mut fuel = 0;
    let invalid = Builtin::ParseDate.call(
        &[s("🦀🦀🦀🦀🦀🦀🦀🦀🦀")],
        SPAN,
        &mut |cost| {
            fuel += cost;
            Ok::<_, ()>(())
        },
    );
    assert!(matches!(invalid, Err(Failure::Value(ref error)) if &*error.code == "E_DATE_PARSE"));
    assert_eq!(fuel, 3);
    for (builtin, args, allowed) in [
        (Builtin::FormatNumber, vec![n("1234.5")], 11),
        (Builtin::FormatBytes, vec![n("1536")], 11),
        (Builtin::FormatDuration, vec![n("1000")], 11),
        (Builtin::FormatDate, vec![n("0")], 11),
        (Builtin::FormatRelative, vec![n("0"), n("0")], 11),
        (Builtin::ParseDate, vec![s("1970-01-01")], 1),
    ] {
        let mut remaining = allowed;
        let result = builtin.call(&args, SPAN, &mut |cost| {
            if cost > remaining {
                Err("empty")
            } else {
                remaining -= cost;
                Ok(())
            }
        });
        assert!(matches!(result, Err(Failure::Fuel("empty"))), "{builtin:?}");
        assert_eq!(remaining, 0);
    }
}
