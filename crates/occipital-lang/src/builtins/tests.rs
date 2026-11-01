use super::*;
use crate::{CodecMode, ScalarString};

mod format;

const SPAN: SourceSpan = SourceSpan {
    line: 1,
    column: 7,
    end_line: 1,
    end_column: 19,
};

fn n(text: &str) -> Value {
    Value::number(Decimal::parse_text(text).unwrap())
}
fn s(text: &str) -> Value {
    Value::string(text).unwrap()
}
fn list(values: Vec<Value>) -> Value {
    Value::list(values).unwrap()
}
fn error(code: &str) -> Value {
    Value::error(ValueError::new(code, SPAN))
}
fn record(fields: Vec<(&str, Value)>) -> Value {
    Value::record(
        fields
            .into_iter()
            .map(|(key, value)| (ScalarString::new(key), value))
            .collect(),
    )
    .unwrap()
}

fn run(builtin: Builtin, args: &[Value]) -> (Value, u64) {
    let mut fuel = 0;
    let value = builtin
        .call(args, SPAN, &mut |cost| {
            fuel += cost;
            Ok::<_, ()>(())
        })
        .unwrap();
    (value, fuel)
}

fn same(actual: &Value, expected: &Value) {
    assert_eq!(
        actual.encode_ordered(CodecMode::Event).unwrap(),
        expected.encode_ordered(CodecMode::Event).unwrap()
    );
}

fn check(builtin: Builtin, args: &[Value], expected: Value, fuel: u64) {
    let (actual, used) = run(builtin, args);
    same(&actual, &expected);
    assert_eq!(used, fuel, "{builtin:?}");
}

fn fails(builtin: Builtin, args: &[Value], code: &str) {
    match builtin.call(args, SPAN, &mut |_| Ok::<_, ()>(())) {
        Err(Failure::Value(error)) => {
            assert_eq!(&*error.code, code);
            assert_eq!(error.span, SPAN);
        }
        other => panic!("expected {code}, got {other:?}"),
    }
}

fn limit(builtin: Builtin, args: &[Value], code: &'static str) {
    assert!(
        matches!(builtin.call(args, SPAN, &mut |_| Ok::<_, ()>(())), Err(Failure::Limit(actual)) if actual == code)
    );
}

#[test]
fn round_worked_values_are_exact() {
    for (args, expected) in [
        (vec![n("2.5")], "3"),
        (vec![n("-2.5")], "-3"),
        (vec![n("2.675"), n("2")], "2.68"),
        (vec![n("1250"), n("-2")], "1300"),
    ] {
        check(Builtin::Round, &args, n(expected), 2);
    }
}

#[test]
fn all_numeric_rows_charge_once_and_report_their_errors() {
    use Builtin::*;
    check(Floor, &[n("-2.5")], n("-3"), 2);
    check(Ceil, &[n("-2.5")], n("-2"), 2);
    check(Abs, &[n("-2.5")], n("2.5"), 2);
    check(Sqrt, &[n("2")], n("1.414213562373095049"), 21);
    for (x, expected) in [("-1", "0"), ("1", "1"), ("3", "2")] {
        check(Clamp, &[n(x), n("0"), n("2")], n(expected), 2);
    }
    fails(Clamp, &[n("1"), n("2"), n("0")], "E_ARG");
    fails(Sqrt, &[n("-1")], "E_DOMAIN");
    for digits in ["1.1", "-21", "19"] {
        fails(Round, &[n("1"), n(digits)], "E_ARG");
    }
    let max = n("99999999999999999999.999999999999999999");
    let min = n("-99999999999999999999.999999999999999999");
    fails(Ceil, std::slice::from_ref(&max), "E_OVERFLOW");
    fails(Floor, &[min], "E_OVERFLOW");
    fails(Round, &[max], "E_OVERFLOW");
    for builtin in [Round, Floor, Ceil, Abs, Sqrt] {
        fails(builtin, &[Value::null()], "E_TYPE");
        fails(builtin, &[s("1")], "E_TYPE");
    }
}

#[test]
fn div_is_exact_coefficient_quotient_not_rounded_decimal_division() {
    for (a, b, answer) in [
        ("7", "3", "2"),
        ("-7", "3", "-2"),
        ("7", "-3", "-2"),
        ("0.999999999999999999", "1", "0"),
        ("1.2", "0.5", "2"),
    ] {
        check(Builtin::Div, &[n(a), n(b)], n(answer), 2);
    }
    fails(Builtin::Div, &[n("1"), n("0")], "E_DIV_ZERO");
    fails(
        Builtin::Div,
        &[n("100"), n("0.000000000000000001")],
        "E_OVERFLOW",
    );
    fails(Builtin::Div, &[Value::null(), n("1")], "E_TYPE");
}

#[test]
fn number_trims_pinned_whitespace_parses_and_rounds_once() {
    check(
        Builtin::Number,
        &[s("\u{2000}+001.25e1\u{3000}")],
        n("12.5"),
        3,
    );
    check(
        Builtin::Number,
        &[s("0.0000000000000000025")],
        n("0.000000000000000002"),
        4,
    );
    check(
        Builtin::Number,
        &[s("-0.0000000000000000015")],
        n("-0.000000000000000002"),
        4,
    );
    check(Builtin::Number, &[n("2")], n("2"), 1);
    for value in [Value::null(), Value::missing()] {
        check(Builtin::Number, &[value], Value::null(), 1);
    }
    for text in ["", " ", ".1", "1.", "1x", "1_000", "\u{180e}1", "\u{200b}1"] {
        fails(Builtin::Number, &[s(text)], "E_NUMBER_PARSE");
    }
    fails(Builtin::Number, &[s("1e20")], "E_OVERFLOW");
    fails(Builtin::Number, &[Value::boolean(true)], "E_TYPE");
}

#[test]
fn sum_and_average_use_wide_coefficients_before_final_range_check() {
    use Builtin::*;
    let max = n("99999999999999999999.999999999999999999");
    let negative = n("-99999999999999999999.999999999999999999");
    check(
        Sum,
        &[list(vec![
            max.clone(),
            max.clone(),
            max.clone(),
            negative.clone(),
            negative,
        ])],
        max.clone(),
        6,
    );
    check(Avg, &[list(vec![max.clone(); 10_000])], max.clone(), 10_001);
    fails(Sum, &[list(vec![max.clone(), max])], "E_OVERFLOW");
    for (values, expected) in [
        (vec![], Value::null()),
        (vec![Value::null(), Value::missing()], Value::null()),
        (vec![n("0"), n("0.000000000000000001")], n("0")),
        (
            vec![n("0.000000000000000001"), n("0.000000000000000002")],
            n("0.000000000000000002"),
        ),
        (
            vec![n("-0.000000000000000001"), n("-0.000000000000000002")],
            n("-0.000000000000000002"),
        ),
        (vec![n("2"), Value::null(), n("4")], n("3")),
    ] {
        let fuel = values.len() as u64 + 1;
        check(Avg, &[list(values)], expected, fuel);
    }
    check(Sum, &[Value::null()], n("0"), 1);
    check(
        Sum,
        &[list(vec![n("1"), Value::missing(), n("2")])],
        n("3"),
        4,
    );
    for builtin in [Sum, Avg] {
        fails(builtin, &[list(vec![s("1")])], "E_TYPE");
        fails(builtin, &[list(vec![error("E_DOMAIN")])], "E_DOMAIN");
    }
}

#[test]
fn min_max_skip_absent_validate_homogeneity_and_charge_comparisons() {
    for builtin in [Builtin::Min, Builtin::Max] {
        let expected = if builtin == Builtin::Min {
            n("1")
        } else {
            n("3")
        };
        check(
            builtin,
            &[list(vec![n("3"), Value::missing(), n("1")])],
            expected.clone(),
            4,
        );
        check(builtin, &[n("3"), Value::null(), n("1")], expected, 4);
        check(builtin, &[Value::null()], Value::null(), 1);
        check(builtin, &[list(vec![])], Value::null(), 1);
        fails(builtin, &[n("1")], "E_TYPE");
        fails(builtin, &[n("1"), s("1")], "E_TYPE");
        fails(builtin, &[list(vec![Value::boolean(false)])], "E_TYPE");
        fails(builtin, &[list(vec![error("E_ARG")])], "E_ARG");
    }
    check(Builtin::Min, &[s("β"), s("a")], s("a"), 4);
    check(Builtin::Max, &[s("a"), s("β")], s("β"), 4);
    check(
        Builtin::Min,
        &[s("abcdefghijklmnopA"), s("abcdefghijklmnopB")],
        s("abcdefghijklmnopA"),
        5,
    );
}

#[test]
fn count_first_last_and_absent_lists_have_exact_results() {
    let values = list(vec![n("1"), Value::null(), n("3")]);
    check(Builtin::Count, std::slice::from_ref(&values), n("3"), 2);
    check(Builtin::First, std::slice::from_ref(&values), n("1"), 2);
    check(Builtin::Last, &[values], n("3"), 2);
    for input in [list(vec![]), Value::null(), Value::missing()] {
        check(Builtin::Count, std::slice::from_ref(&input), n("0"), 2);
        check(
            Builtin::First,
            std::slice::from_ref(&input),
            Value::missing(),
            2,
        );
        check(Builtin::Last, &[input], Value::missing(), 2);
    }
    for builtin in [Builtin::Count, Builtin::First, Builtin::Last] {
        fails(builtin, &[n("0")], "E_TYPE");
    }
}

#[test]
fn range_preflights_exact_count_direction_fraction_and_limit() {
    check(
        Builtin::Range,
        &[n("1"), n("4")],
        list(vec![n("1"), n("2"), n("3")]),
        4,
    );
    check(
        Builtin::Range,
        &[n("3"), n("-1"), n("-2")],
        list(vec![n("3"), n("1")]),
        3,
    );
    check(
        Builtin::Range,
        &[n("0"), n("0.31"), n("0.1")],
        list(vec![n("0"), n("0.1"), n("0.2"), n("0.3")]),
        5,
    );
    check(Builtin::Range, &[n("3"), n("1")], list(vec![]), 1);
    check(Builtin::Range, &[n("1"), n("3"), n("-1")], list(vec![]), 1);
    check(
        Builtin::Range,
        &[
            n("99999999999999999999"),
            n("99999999999999999999.1"),
            n("9"),
        ],
        list(vec![n("99999999999999999999")]),
        2,
    );
    let (value, fuel) = run(Builtin::Range, &[n("0"), n("10000")]);
    assert_eq!(value.list_element(9999).to_text(SPAN).unwrap(), "9999");
    assert_eq!(fuel, 10_001);
    limit(Builtin::Range, &[n("0"), n("10001")], "E_LIMIT_LIST");
    limit(
        Builtin::Range,
        &[
            n("-99999999999999999999"),
            n("99999999999999999999"),
            n("0.000000000000000001"),
        ],
        "E_LIMIT_LIST",
    );
    fails(Builtin::Range, &[n("0"), n("1"), n("0")], "E_ARG");
}

#[test]
fn slice_uses_scalar_positions_negative_clamping_and_empty_results() {
    check(Builtin::Slice, &[s("aé🦀z"), n("1"), n("-1")], s("é🦀"), 3);
    check(Builtin::Slice, &[s("aé🦀z"), n("-2")], s("🦀z"), 3);
    check(
        Builtin::Slice,
        &[s("abc"), n("-100"), n("100")],
        s("abc"),
        2,
    );
    check(Builtin::Slice, &[s("abc"), n("2"), n("1")], s(""), 2);
    check(
        Builtin::Slice,
        &[list(vec![n("1"), n("2"), n("3")]), n("-2")],
        list(vec![n("2"), n("3")]),
        3,
    );
    check(
        Builtin::Slice,
        &[Value::null(), n("1")],
        Value::missing(),
        1,
    );
    fails(Builtin::Slice, &[s("abc"), n("0.1")], "E_ARG");
    fails(Builtin::Slice, &[s("abc"), s("1")], "E_TYPE");
}

#[test]
fn contains_charges_search_or_deep_equality_and_propagates_errors() {
    check(
        Builtin::Contains,
        &[s("aé🦀"), s("é🦀")],
        Value::boolean(true),
        2,
    );
    check(
        Builtin::Contains,
        &[s("aaaaab"), s("aaac")],
        Value::boolean(false),
        2,
    );
    check(Builtin::Contains, &[s(""), s("")], Value::boolean(true), 1);
    check(
        Builtin::Contains,
        &[Value::missing(), n("1")],
        Value::boolean(false),
        1,
    );
    check(
        Builtin::Contains,
        &[
            list(vec![list(vec![n("1")]), list(vec![n("2")])]),
            list(vec![n("2")]),
        ],
        Value::boolean(true),
        7,
    );
    fails(Builtin::Contains, &[s("1"), n("1")], "E_TYPE");
    fails(
        Builtin::Contains,
        &[list(vec![error("E_DOMAIN"), n("1")]), n("1")],
        "E_DOMAIN",
    );
}

#[test]
fn keys_and_values_preserve_insertion_order() {
    let record = record(vec![("b", n("2")), ("a", n("1"))]);
    check(
        Builtin::Keys,
        std::slice::from_ref(&record),
        list(vec![s("b"), s("a")]),
        3,
    );
    check(Builtin::Values, &[record], list(vec![n("2"), n("1")]), 3);
    for builtin in [Builtin::Keys, Builtin::Values] {
        check(builtin, &[Value::missing()], list(vec![]), 1);
        fails(builtin, &[list(vec![])], "E_TYPE");
    }
}

#[test]
fn join_preflights_text_conversions_and_total_scalar_length() {
    check(
        Builtin::Join,
        &[
            list(vec![
                n("-2.5"),
                Value::null(),
                Value::boolean(false),
                s("é"),
            ]),
            s("|"),
        ],
        s("-2.5||false|é"),
        7,
    );
    check(Builtin::Join, &[Value::missing(), s(",")], s(""), 1);
    for element in [list(vec![]), record(vec![])] {
        fails(Builtin::Join, &[list(vec![element]), s(",")], "E_TYPE");
    }
    fails(
        Builtin::Join,
        &[list(vec![error("E_DOMAIN")]), s(",")],
        "E_DOMAIN",
    );
    limit(
        Builtin::Join,
        &[list(vec![s(&"a".repeat(40_000)); 2]), s("")],
        "E_LIMIT_STRING",
    );
}

#[test]
fn filter_supports_every_operator_fields_and_nonmatching_comparison_errors() {
    for (op, expected) in [
        ("==", vec![n("2")]),
        ("!=", vec![n("1"), n("3")]),
        ("<", vec![n("1")]),
        ("<=", vec![n("1"), n("2")]),
        (">", vec![n("3")]),
        (">=", vec![n("2"), n("3")]),
    ] {
        check(
            Builtin::Filter,
            &[
                list(vec![n("1"), n("2"), n("3"), error("E_DOMAIN")]),
                Value::null(),
                s(op),
                n("2"),
            ],
            list(expected),
            5,
        );
    }
    let a = record(vec![("k", n("2")), ("id", s("a"))]);
    let b = record(vec![("k", n("1")), ("id", s("b"))]);
    check(
        Builtin::Filter,
        &[list(vec![a.clone(), b]), s("k"), s(">"), n("1")],
        list(vec![a]),
        3,
    );
    check(
        Builtin::Filter,
        &[
            list(vec![s("abc"), n("1"), s("b")]),
            Value::null(),
            s("contains"),
            s("b"),
        ],
        list(vec![s("abc"), s("b")]),
        6,
    );
    check(
        Builtin::Filter,
        &[
            list(vec![list(vec![n("1"), n("2")]), list(vec![n("3")])]),
            Value::null(),
            s("contains"),
            n("2"),
        ],
        list(vec![list(vec![n("1"), n("2")])]),
        6,
    );
    check(
        Builtin::Filter,
        &[Value::missing(), Value::null(), s("=="), n("1")],
        list(vec![]),
        1,
    );
    fails(
        Builtin::Filter,
        &[list(vec![]), n("0"), s("=="), n("1")],
        "E_ARG",
    );
    fails(
        Builtin::Filter,
        &[list(vec![]), Value::missing(), s("=="), n("1")],
        "E_ARG",
    );
    fails(
        Builtin::Filter,
        &[list(vec![]), Value::null(), s("wat"), n("1")],
        "E_ARG",
    );
}

#[test]
fn sort_is_stable_for_equal_keys_in_both_directions_and_keeps_absent_then_errors_last() {
    let input = list(vec![
        record(vec![("id", s("error")), ("k", error("E_ARG"))]),
        record(vec![("id", s("two-a")), ("k", n("2"))]),
        record(vec![("id", s("missing"))]),
        record(vec![("id", s("opaque-list")), ("k", list(vec![n("9")]))]),
        record(vec![("id", s("one")), ("k", n("1"))]),
        record(vec![("id", s("true")), ("k", Value::boolean(true))]),
        record(vec![("id", s("two-b")), ("k", n("2"))]),
        record(vec![("id", s("text")), ("k", s("z"))]),
        record(vec![("id", s("false")), ("k", Value::boolean(false))]),
        record(vec![("id", s("null")), ("k", Value::null())]),
        record(vec![("id", s("opaque-record")), ("k", record(vec![]))]),
    ]);
    for (direction, expected) in [
        (
            "asc",
            vec![
                "one",
                "two-a",
                "two-b",
                "text",
                "false",
                "true",
                "opaque-list",
                "opaque-record",
                "missing",
                "null",
                "error",
            ],
        ),
        (
            "desc",
            vec![
                "opaque-list",
                "opaque-record",
                "true",
                "false",
                "text",
                "two-a",
                "two-b",
                "one",
                "missing",
                "null",
                "error",
            ],
        ),
    ] {
        let (result, fuel) = run(Builtin::Sort, &[input.clone(), s("k"), s(direction)]);
        let ids = match result.kind() {
            ValueKind::List(items) => items.iter().map(|v| v.record_field("id")).collect(),
            _ => unreachable!(),
        };
        same(&list(ids), &list(expected.into_iter().map(s).collect()));
        // Call base 1, key discovery n=11, and 2*n*ceil(log2(n))*(1+ceil(K/64))
        // comparison/build fuel with a longest string key of K=1 scalar.
        assert_eq!(fuel, 188);
    }
    check(
        Builtin::Sort,
        &[list(vec![n("2"), n("1")])],
        list(vec![n("1"), n("2")]),
        7,
    );
    check(Builtin::Sort, &[Value::null()], list(vec![]), 1);
    fails(
        Builtin::Sort,
        &[list(vec![]), Value::null(), s("down")],
        "E_ARG",
    );
    fails(Builtin::Sort, &[list(vec![]), n("1")], "E_TYPE");
}

#[test]
fn sort_preserves_large_equal_key_runs_and_uses_longest_scalar_key_tariff() {
    let rows = list(
        (0..128)
            .map(|i| {
                record(vec![
                    ("k", n(&(i % 4).to_string())),
                    ("id", n(&i.to_string())),
                ])
            })
            .collect(),
    );
    for (direction, groups) in [("asc", vec![0, 1, 2, 3]), ("desc", vec![3, 2, 1, 0])] {
        let (result, fuel) = run(Builtin::Sort, &[rows.clone(), s("k"), s(direction)]);
        let ids = match result.kind() {
            ValueKind::List(items) => items.iter().map(|v| v.record_field("id")).collect(),
            _ => unreachable!(),
        };
        let expected = groups
            .into_iter()
            .flat_map(|g| (g..128).step_by(4).map(|i| n(&i.to_string())))
            .collect();
        same(&list(ids), &list(expected));
        assert_eq!(fuel, 1921);
    }
    for (length, fuel) in [(64, 28), (65, 40)] {
        let long = s(&"é".repeat(length));
        check(
            Builtin::Sort,
            &[list(vec![long.clone(), s("z"), s("a")])],
            list(vec![s("a"), s("z"), long]),
            fuel,
        );
    }
}

#[test]
fn each_is_lazy_ordered_metered_and_stops_on_abort() {
    let input = list(vec![n("1"), n("2"), n("3")]);
    let mut seen = Vec::new();
    let mut fuel = 0;
    let result = each(
        &input,
        SPAN,
        &mut |cost| {
            fuel += cost;
            Ok::<_, ()>(())
        },
        |value, charge| {
            charge(2).map_err(Failure::Fuel)?;
            seen.push(value.to_text(SPAN).unwrap());
            Ok(value.clone())
        },
    )
    .unwrap();
    same(&result, &input);
    assert_eq!(seen, ["1", "2", "3"]);
    assert_eq!(fuel, 10);
    let result = each(&Value::missing(), SPAN, &mut |_| Ok::<_, ()>(()), |_, _| {
        panic!("absent list evaluates no template")
    })
    .unwrap();
    same(&result, &list(vec![]));
    let mut calls = 0;
    let result = each(&input, SPAN, &mut |_| Ok::<_, ()>(()), |_, _| {
        calls += 1;
        Err(Failure::Limit("E_LIMIT_STRING"))
    });
    assert!(matches!(result, Err(Failure::Limit("E_LIMIT_STRING"))));
    assert_eq!(calls, 1);
}

#[test]
fn text_len_and_pinned_unicode_string_operations_have_exact_fuel() {
    check(Builtin::Text, &[n("-2.5")], s("-2.5"), 2);
    check(Builtin::Text, &[n("0")], s("0"), 2);
    check(Builtin::Text, &[Value::boolean(true)], s("true"), 2);
    check(Builtin::Text, &[Value::missing()], s(""), 1);
    check(Builtin::Text, &[s("e\u{301}")], s("e\u{301}"), 2);
    fails(Builtin::Text, &[record(vec![])], "E_TYPE");
    check(Builtin::Len, &[s("a🦀e\u{301}")], n("4"), 2);
    check(Builtin::Len, &[Value::null()], n("0"), 2);
    check(Builtin::Upper, &[s("ß")], s("SS"), 3);
    check(Builtin::Lower, &[s("İ")], s("i\u{307}"), 3);
    check(Builtin::Lower, &[s("ΟΣ ΟΣΑ")], s("ος οσα"), 3);
    check(Builtin::Upper, &[s("\u{1c8a}")], s("\u{1c89}"), 3);
    check(Builtin::Lower, &[s("\u{1c89}")], s("\u{1c8a}"), 3);
    check(Builtin::Trim, &[s("\u{2000}\t a \u{3000}")], s("a"), 3);
    check(
        Builtin::Trim,
        &[s("\u{180e}a\u{200b}")],
        s("\u{180e}a\u{200b}"),
        3,
    );
    for builtin in [Builtin::Upper, Builtin::Lower, Builtin::Trim] {
        fails(builtin, &[Value::null()], "E_TYPE");
    }
}

#[test]
fn prefix_suffix_truncate_and_replace_obey_scalar_boundaries() {
    check(
        Builtin::StartsWith,
        &[s("é🦀z"), s("é")],
        Value::boolean(true),
        2,
    );
    check(
        Builtin::EndsWith,
        &[s("é🦀z"), s("🦀z")],
        Value::boolean(true),
        2,
    );
    check(
        Builtin::StartsWith,
        &[s("abc"), s("")],
        Value::boolean(true),
        1,
    );
    check(
        Builtin::EndsWith,
        &[Value::null(), s("z")],
        Value::boolean(false),
        2,
    );
    check(Builtin::Truncate, &[s("a🦀éz"), n("3")], s("a🦀…"), 2);
    check(Builtin::Truncate, &[s("abc"), n("1")], s("…"), 2);
    check(Builtin::Truncate, &[s("abc"), n("3")], s("abc"), 2);
    check(Builtin::Truncate, &[s(""), n("1")], s(""), 1);
    for max in ["0", "-1", "1.1"] {
        fails(Builtin::Truncate, &[s("x"), n(max)], "E_ARG");
    }
    check(
        Builtin::Replace,
        &[s("aaaaa"), s("aa"), s("b")],
        s("bba"),
        3,
    );
    check(
        Builtin::Replace,
        &[s("é🦀é"), s("é"), s("SS")],
        s("SS🦀SS"),
        3,
    );
    check(Builtin::Replace, &[s("abc"), s("x"), s("z")], s("abc"), 3);
    check(Builtin::Replace, &[s(""), s("a"), s("z")], s(""), 1);
    fails(Builtin::Replace, &[s("x"), s(""), s("z")], "E_ARG");
}

#[test]
fn split_worked_value_and_lines_all_break_forms_are_exact() {
    check(
        Builtin::Split,
        &[s("a,,b"), s(",")],
        list(vec![s("a"), s(""), s("b")]),
        6,
    );
    check(Builtin::Split, &[s(""), s(",")], list(vec![s("")]), 2);
    check(
        Builtin::Split,
        &[s("a,"), s(",")],
        list(vec![s("a"), s("")]),
        5,
    );
    check(
        Builtin::Split,
        &[s("é🦀é"), s("🦀")],
        list(vec![s("é"), s("é")]),
        5,
    );
    check(
        Builtin::Split,
        &[s("a\nb"), s("longer")],
        list(vec![s("a\nb")]),
        4,
    );
    check(Builtin::Split, &[Value::null(), s(",")], list(vec![]), 1);
    fails(Builtin::Split, &[s("a"), s("")], "E_ARG");
    check(
        Builtin::Lines,
        &[s("a\r\nb\rc\n\n")],
        list(vec![s("a"), s("b"), s("c"), s("")]),
        7,
    );
    check(Builtin::Lines, &[s("\r\n")], list(vec![s("")]), 3);
    check(Builtin::Lines, &[s("")], list(vec![]), 1);
    check(Builtin::Lines, &[Value::missing()], list(vec![]), 1);
    let (lines, fuel) = run(Builtin::Lines, &[s(&"\n".repeat(10_000))]);
    assert!(matches!(lines.kind(), ValueKind::List(items) if items.len() == 10_000));
    assert_eq!(fuel, 10_626);
    limit(Builtin::Lines, &[s(&"\n".repeat(10_001))], "E_LIMIT_LIST");
    limit(
        Builtin::Split,
        &[s(&",".repeat(10_000)), s(",")],
        "E_LIMIT_LIST",
    );
}

#[test]
fn string_lengths_are_limited_before_any_output_is_built() {
    limit(Builtin::Upper, &[s(&"ß".repeat(32_769))], "E_LIMIT_STRING");
    limit(
        Builtin::Replace,
        &[s(&"x".repeat(32_769)), s("x"), s("yy")],
        "E_LIMIT_STRING",
    );
    limit(Builtin::Text, &[s(&"x".repeat(65_537))], "E_LIMIT_STRING");
    limit(Builtin::Trim, &[s(&"x".repeat(65_537))], "E_LIMIT_STRING");
    limit(
        Builtin::Split,
        &[s(&"x".repeat(65_537)), s(",")],
        "E_LIMIT_STRING",
    );
    let (result, _) = run(Builtin::Upper, &[s(&"ß".repeat(32_768))]);
    assert!(matches!(result.kind(), ValueKind::String(text) if text.len() == 65_536));
}

#[test]
fn repeated_trim_of_two_mib_whitespace_exhausts_evaluation_fuel() {
    let source = s(&" ".repeat(2 * 1024 * 1024));
    let mut used = 0u64;
    let mut charge = |cost| {
        if used + cost > 1_000_000 {
            return Err("E_FUEL");
        }
        used += cost;
        Ok(())
    };
    for _ in 0..3 {
        let result = Builtin::Trim
            .call(std::slice::from_ref(&source), SPAN, &mut charge)
            .unwrap();
        same(&result, &s(""));
    }
    assert!(matches!(
        Builtin::Trim.call(&[source], SPAN, &mut charge),
        Err(Failure::Fuel("E_FUEL"))
    ));
    assert_eq!(used, 786_436);
}

#[test]
fn fuel_is_checked_before_scans_sorts_outputs_and_each_templates() {
    for (builtin, args, charges) in [
        (Builtin::Trim, vec![s(" abc ")], vec![1, 1, 1]),
        (Builtin::Upper, vec![s("ß")], vec![1, 1, 1]),
        (
            Builtin::Replace,
            vec![s("xx"), s("x"), s("zz")],
            vec![1, 1, 1],
        ),
        (Builtin::Split, vec![s("a,,b"), s(",")], vec![1, 1, 4]),
        (
            Builtin::Join,
            vec![list(vec![s("a"), s("b")]), s(",")],
            vec![1, 2, 1],
        ),
        (
            Builtin::Sort,
            vec![list(vec![s("b"), s("a")])],
            vec![1, 2, 8],
        ),
        (Builtin::Range, vec![n("0"), n("3")], vec![1, 3]),
        (Builtin::Number, vec![s(" 1 ")], vec![1, 1]),
        (Builtin::Sqrt, vec![n("4")], vec![1, 20]),
    ] {
        for stop in 0..charges.len() {
            let mut seen = Vec::new();
            let result = builtin.call(&args, SPAN, &mut |cost| {
                seen.push(cost);
                if seen.len() == stop + 1 {
                    Err("E_FUEL")
                } else {
                    Ok(())
                }
            });
            assert!(
                matches!(result, Err(Failure::Fuel("E_FUEL"))),
                "{builtin:?}"
            );
            assert_eq!(seen, charges[..=stop], "{builtin:?}");
        }
    }
    let result = each(
        &list(vec![n("1")]),
        SPAN,
        &mut |cost| {
            if cost == 1 {
                Err("E_FUEL")
            } else {
                Ok(())
            }
        },
        |_, _| panic!("a refused precharge must not run a template"),
    );
    assert!(matches!(result, Err(Failure::Fuel("E_FUEL"))));
}

#[test]
fn errors_propagate_left_to_right_except_iserror_and_bad_arity_is_reported() {
    check(
        Builtin::IsError,
        &[error("E_DOMAIN")],
        Value::boolean(true),
        2,
    );
    check(
        Builtin::IsError,
        &[Value::missing()],
        Value::boolean(false),
        2,
    );
    fails(
        Builtin::Replace,
        &[error("E_DOMAIN"), error("E_ARG"), s("")],
        "E_DOMAIN",
    );
    fails(Builtin::Count, &[], "E_ARITY");
    fails(Builtin::Round, &[n("1"), n("2"), n("3")], "E_ARITY");
}

#[test]
fn linear_search_handles_adversarial_prefixes_and_nonoverlapping_matches() {
    let source = format!("{}b", "a".repeat(100_000));
    let pattern = format!("{}b", "a".repeat(20_000));
    let search = strings::Search::new(&pattern);
    assert_eq!(search.matches(&source).collect::<Vec<_>>(), [80_000]);
    assert_eq!(
        strings::Search::new("aba")
            .matches("abababa")
            .collect::<Vec<_>>(),
        [0, 4]
    );
    assert_eq!(
        strings::Search::new("éé")
            .matches("ééééé")
            .collect::<Vec<_>>(),
        [0, 4]
    );
}

#[test]
fn tiny_slice_at_a_large_scalar_index_pays_for_the_prefix_before_scanning() {
    let source = s(&"é".repeat(100_001));
    check(
        Builtin::Slice,
        &[source.clone(), n("100000"), n("100001")],
        s("é"),
        12_502,
    );
    let mut attempted = Vec::new();
    let result = Builtin::Slice.call(&[source, n("100000"), n("100001")], SPAN, &mut |fuel| {
        attempted.push(fuel);
        if fuel > 100 {
            Err("E_FUEL")
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(Failure::Fuel("E_FUEL"))));
    assert_eq!(attempted, [1, 12_500]);
}

#[test]
fn each_templates_can_dispatch_builtins_and_stop_before_oversized_list_creation() {
    let mut fuel = 0;
    let result = each(
        &list(vec![n("-1"), n("-2")]),
        SPAN,
        &mut |cost| {
            fuel += cost;
            Ok::<_, ()>(())
        },
        |value, charge| Builtin::Abs.call(std::slice::from_ref(value), SPAN, charge),
    )
    .unwrap();
    same(&result, &list(vec![n("1"), n("2")]));
    assert_eq!(fuel, 7);
    let large = s(&"x".repeat(65_536));
    let mut calls = 0;
    let result = each(
        &list(vec![n("1"); 20]),
        SPAN,
        &mut |_| Ok::<_, ()>(()),
        |_, _| {
            calls += 1;
            Ok(large.clone())
        },
    );
    assert!(matches!(result, Err(Failure::Limit("E_LIMIT_VALUE"))));
    assert_eq!(calls, 16);
}
