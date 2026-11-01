use super::*;

#[test]
fn any_root_is_the_value_and_only_an_array_root_is_rows() {
    let prepared = json(" {\"b\": 2, \"a\": [true, null, \"x\"]} ");
    let expected = record(vec![
        ("b", n("2")),
        ("a", list(vec![Value::boolean(true), Value::null(), s("x")])),
    ]);
    same(&prepared.value, &expected);
    // Keys keep their source order.
    same(&prepared.rows, &Value::missing());
    assert!(prepared.diagnostics.is_empty());

    for (text, root) in [
        ("\"x\"", s("x")),
        ("-12.5", n("-12.5")),
        ("false", Value::boolean(false)),
        ("null", Value::null()),
    ] {
        let prepared = json(text);
        same(&prepared.value, &root);
        same(&prepared.rows, &Value::missing());
    }

    let prepared = json("[{\"a\": 1}, []]");
    let expected = list(vec![record(vec![("a", n("1"))]), list(vec![])]);
    same(&prepared.value, &expected);
    same(&prepared.rows, &expected);
}

#[test]
fn grammar_violations_are_json_syntax_at_their_line_and_column() {
    for (text, line, column) in [
        // Trailing commas.
        ("[1,]", 1, 4),
        ("{\"a\":1,}", 1, 8),
        // Comments.
        ("// c\n1", 1, 1),
        ("[1 /*x*/]", 1, 4),
        // Single quotes, NaN, Infinity and a leading `+`.
        ("['a']", 1, 2),
        ("NaN", 1, 1),
        ("Infinity", 1, 1),
        ("[-Infinity]", 1, 3),
        ("+1", 1, 1),
        // Leading zeros.
        ("01", 1, 2),
        ("[-01]", 1, 4),
        // Incomplete numbers.
        ("1.", 1, 3),
        (".5", 1, 1),
        ("1e", 1, 3),
        ("1e+", 1, 4),
        // Unescaped control characters, including a raw tab.
        ("\"a\u{1}b\"", 1, 3),
        ("\"a\tb\"", 1, 3),
        // An unterminated string, unknown escapes, bad hex.
        ("\"abc", 1, 5),
        ("\"\\x\"", 1, 2),
        ("\"\\u12G4\"", 1, 2),
        // Misspelled literals, extra values, nothing at all.
        ("tru", 1, 1),
        ("nul", 1, 1),
        ("1 2", 1, 3),
        ("", 1, 1),
        ("   ", 1, 4),
        ("{\"a\" 1}", 1, 6),
        ("{1:2}", 1, 2),
        // Lines end at LF or CRLF; columns count scalars, not bytes.
        ("{\n  \"a\": tru\n}", 2, 8),
        ("[\r\n1,\r\n]", 3, 1),
        ("[\"é\", x]", 1, 7),
    ] {
        let prepared = json(text);
        fails_at(&prepared, "E_JSON_SYNTAX", line, column, None);
        assert_eq!(text_of(&prepared), text);
    }
}

#[test]
fn a_duplicate_key_fails_the_parse_at_the_repeated_key() {
    let prepared = json("{\"a\":1,\"b\":2,\"a\":3}");
    fails_at(&prepared, "E_JSON_DUPLICATE_KEY", 1, 14, None);
    let prepared = json("[{\"x\": {\"k\": 1,\n \"k\": 1}}]");
    fails_at(&prepared, "E_JSON_DUPLICATE_KEY", 2, 2, None);
    // Keys that differ only in normalization are different keys (§4.3).
    let prepared = json("{\"é\":1,\"e\u{301}\":2}");
    assert!(prepared.diagnostics.is_empty());
}

#[test]
fn only_utf8_is_accepted_and_its_byte_order_mark_is_removed() {
    // The BOM is not part of the snapshot text, so positions start after it.
    let prepared = run(
        b"\xEF\xBB\xBF{\"a\":1,\"a\":2}",
        Format::Json,
        &Options::default(),
    )
    .0;
    fails_at(&prepared, "E_JSON_DUPLICATE_KEY", 1, 8, None);
    let prepared = run(b"\xEF\xBB\xBF[1]", Format::Json, &Options::default()).0;
    same(&prepared.value, &list(vec![n("1")]));

    for bytes in [
        &b"\xFF\xFE[\x00]\x00"[..],
        b"\xFE\xFF\x00[\x00]",
        b"\xFF\xFE\x00\x00[\x00\x00\x00",
        b"\x00\x00\xFE\xFF\x00\x00\x00[",
    ] {
        let prepared = run(bytes, Format::Json, &Options::default()).0;
        fails_at(&prepared, "E_DATA_ENCODING", 1, 1, None);
        assert_eq!(text_of(&prepared), String::from_utf8_lossy(bytes));
    }
    let prepared = run(b"[1,\n\"\xC3\"]", Format::Json, &Options::default()).0;
    fails_at(&prepared, "E_DATA_ENCODING", 2, 2, None);
    assert_eq!(text_of(&prepared), "[1,\n\"\u{FFFD}\"]");
}

#[test]
fn numbers_convert_exactly_round_past_18_places_and_overflow_locally() {
    let text = "[0.1, -0, 99999999999999999999, 1.0000000000000000005, \
                1.0000000000000000015, 1e-19, 0.1000000000000000000, \
                100000000000000000000, -1e20, 12.5e-1, 1E2]";
    let prepared = json(text);
    let column = |token: &str| text.find(token).unwrap() as u64 + 1;
    let expected = list(vec![
        n("0.1"),
        n("0"),
        n("99999999999999999999"),
        // Half to even: ...0|5 stays, ...1|5 goes up.
        n("1"),
        n("1.000000000000000002"),
        n("0"),
        n("0.1"),
        error("E_OVERFLOW"),
        error("E_OVERFLOW"),
        n("1.25"),
        n("100"),
    ]);
    same(&prepared.value, &expected);
    match prepared.value.list_element(1).kind() {
        crate::ValueKind::Number(zero) => assert_eq!(zero.coefficient(), 0),
        _ => panic!("-0 is a number"),
    }
    assert_eq!(
        diagnostics(&prepared),
        [
            json!({"code": "W_NUMBER_ROUNDED", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 1, "column": column("1.0000000000000000005")},
                "message": "3 data numbers had more than 18 fraction digits and were rounded."}),
            json!({"code": "W_NUMBER_RANGE", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 1, "column": column("100000000000000000000")},
                "message": "2 data numbers were out of range and became E_OVERFLOW errors."}),
        ]
    );
}

#[test]
fn surrogate_pairs_join_and_lone_surrogates_become_replacement_characters() {
    let text = r#"["\ud83d\ude00", "\ud83d", "\uDE00", "\ud83dx", "\ud83d\ud83d\ude00", "\u00e9\/\b\f\n\r\t\"\\"]"#;
    let prepared = json(text);
    let expected = list(vec![
        s("😀"),
        s("\u{FFFD}"),
        s("\u{FFFD}"),
        s("\u{FFFD}x"),
        s("\u{FFFD}😀"),
        s("é/\u{8}\u{C}\n\r\t\"\\"),
    ]);
    same(&prepared.value, &expected);
    let first = text.find(r#""\ud83d""#).unwrap() as u64 + 2;
    assert_eq!(
        diagnostics(&prepared),
        [
            json!({"code": "W_JSON_SURROGATE", "severity": "warning", "phase": "data",
            "span": {"source": "data", "line": 1, "column": first},
            "message": "4 unpaired surrogate escapes were replaced with U+FFFD."})
        ]
    );
}

#[test]
fn declared_columns_convert_row_fields_by_the_section_10_3_table() {
    let options = columns(&[
        ("t", ColumnType::Text),
        ("n", ColumnType::Number),
        ("b", ColumnType::Boolean),
    ]);
    let lines = [
        "[",
        "{\"t\": 1e5, \"n\": 2.50, \"b\": true, \"other\": \"7\"},",
        "{\"t\": \"x\", \"n\": \"12.5\", \"b\": \"true\"},",
        "{\"t\": true, \"n\": \" 7 \", \"b\": \"false\"},",
        "{\"t\": null, \"n\": \"\", \"b\": null},",
        "{\"t\": -0, \"n\": \"abc\", \"b\": 1},",
        "{\"t\": 123456789012345678901234567890, \"n\": true, \"b\": \"yes\"},",
        "{\"t\": false, \"n\": null, \"b\": \"\"},",
        "{\"t\": [1], \"n\": \"1e30\"},",
        "{\"n\": \"  \"},",
        "\"not a record\"",
        "]",
    ];
    let prepared = json_with(&lines.join("\n"), &options);
    let expected = list(vec![
        record(vec![
            ("t", s("1e5")),
            ("n", n("2.5")),
            ("b", Value::boolean(true)),
            ("other", s("7")),
        ]),
        record(vec![
            ("t", s("x")),
            ("n", n("12.5")),
            ("b", Value::boolean(true)),
        ]),
        record(vec![
            ("t", s("true")),
            ("n", n("7")),
            ("b", Value::boolean(false)),
        ]),
        record(vec![
            ("t", Value::null()),
            ("n", Value::null()),
            ("b", Value::null()),
        ]),
        record(vec![
            ("t", s("-0")),
            ("n", error("E_NUMBER_PARSE")),
            ("b", error("E_TYPE")),
        ]),
        record(vec![
            ("t", s("123456789012345678901234567890")),
            ("n", error("E_TYPE")),
            ("b", error("E_TYPE")),
        ]),
        record(vec![
            ("t", s("false")),
            ("n", Value::null()),
            ("b", error("E_TYPE")),
        ]),
        record(vec![("t", list(vec![n("1")])), ("n", error("E_OVERFLOW"))]),
        // Only the empty string is null; white space goes through Number(s).
        record(vec![("n", error("E_NUMBER_PARSE"))]),
        s("not a record"),
    ]);
    same(&prepared.value, &expected);
    same(&prepared.rows, &expected);
    // Row 5 is on line 6. The beyond-range id in a text column is kept as
    // its token, so no W_NUMBER_RANGE.
    let column = |token: &str| lines[5].find(token).unwrap() as u64 + 1;
    assert_eq!(
        diagnostics(&prepared),
        [
            json!({"code": "W_CELL_TYPE", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 6, "column": column("\"abc\"")},
                "subject": "n",
                "message": "4 values in this column could not be converted; the first is in row 5."}),
            json!({"code": "W_CELL_TYPE", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 6, "column": column("1}")},
                "subject": "b",
                "message": "3 values in this column could not be converted; the first is in row 5."}),
        ]
    );
}

#[test]
fn columns_convert_only_records_directly_in_a_root_array() {
    let options = columns(&[("n", ColumnType::Number)]);
    for text in [
        "{\"n\": \"x\"}",
        "[[{\"n\": \"x\"}]]",
        "[{\"o\": {\"n\": \"x\"}}]",
    ] {
        let prepared = json_with(text, &options);
        same(&prepared.value, &json(text).value);
        assert!(prepared.diagnostics.is_empty(), "{text}");
    }
}
