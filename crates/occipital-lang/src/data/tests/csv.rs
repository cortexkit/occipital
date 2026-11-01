use super::*;

fn row(fields: &[(&str, &str)]) -> Value {
    record(fields.iter().map(|(key, value)| (*key, s(value))).collect())
}

#[test]
fn records_become_rows_keyed_by_the_header_in_order() {
    let prepared = csv("b,a\n1,2\n3,4");
    let expected = list(vec![
        row(&[("b", "1"), ("a", "2")]),
        row(&[("b", "3"), ("a", "4")]),
    ]);
    same(&prepared.value, &expected);
    same(&prepared.rows, &expected);
    assert!(prepared.diagnostics.is_empty());
    // Every cell is text exactly as written, spaces included.
    let prepared = csv("id,zip\r\n007, 00501 \r\n1234567890123456789012345,x\r\n");
    let expected = list(vec![
        row(&[("id", "007"), ("zip", " 00501 ")]),
        row(&[("id", "1234567890123456789012345"), ("zip", "x")]),
    ]);
    same(&prepared.value, &expected);
}

#[test]
fn quoted_fields_may_span_lines_and_keep_their_breaks_exactly() {
    let prepared = csv("a,b\n\"x\r\ny\nz\",\"q\"\"z,\"\n\"\",3\n");
    let expected = list(vec![
        row(&[("a", "x\r\ny\nz"), ("b", "q\"z,")]),
        row(&[("a", ""), ("b", "3")]),
    ]);
    same(&prepared.value, &expected);
    assert!(prepared.diagnostics.is_empty());
}

#[test]
fn empty_lines_are_skipped_but_a_line_of_delimiters_is_a_record() {
    let prepared = csv("\na,b\n\n1,2\r\n\r\n,\n\n");
    let expected = list(vec![
        row(&[("a", "1"), ("b", "2")]),
        row(&[("a", ""), ("b", "")]),
    ]);
    same(&prepared.value, &expected);
    assert!(prepared.diagnostics.is_empty());
    // A header with no records after it gives no rows; so does no input.
    same(&csv("a,b\n").value, &list(vec![]));
    same(&csv("").value, &list(vec![]));
    same(&csv("").rows, &list(vec![]));
}

#[test]
fn malformed_records_are_csv_syntax_with_line_column_and_record() {
    for (text, line, column, record) in [
        // A lone CR outside quotes, inside a record and between records.
        ("a,b\n1,2\r3,4\n", 2, 4, 2),
        ("a\n\rb", 2, 1, 2),
        // A quote inside an unquoted field.
        ("a,b\nx,y\"z\n", 2, 4, 2),
        // Something other than a delimiter, break or end after a closing quote.
        ("a,b\n\"x\"y,z\n", 2, 4, 2),
        ("h\n\"1\n2\"x\n", 3, 3, 2),
        ("h\n\"1\"\r2\n", 2, 4, 2),
        // A quote still open at the end, reported at its opening quote.
        ("a,b\n1,2\n\"x,y\n", 3, 1, 3),
    ] {
        let prepared = csv(text);
        fails_at(&prepared, "E_CSV_SYNTAX", line, column, Some(record));
        assert_eq!(text_of(&prepared), text);
    }
}

#[test]
fn only_utf8_is_accepted_and_its_byte_order_mark_is_removed() {
    let prepared = run(b"\xEF\xBB\xBFa\n1\n", Format::Csv, &Options::default()).0;
    same(&prepared.value, &list(vec![row(&[("a", "1")])]));
    let prepared = run(b"\xFF\xFEa\x00", Format::Csv, &Options::default()).0;
    fails_at(&prepared, "E_DATA_ENCODING", 1, 1, None);
    let prepared = run(b"a\nb\xE2\x82\n", Format::Csv, &Options::default()).0;
    fails_at(&prepared, "E_DATA_ENCODING", 2, 2, None);
    assert_eq!(text_of(&prepared), "a\nb\u{FFFD}\n");
}

#[test]
fn empty_and_repeated_header_names_are_renamed_once_with_a_count() {
    let prepared = csv("name,name,,name\n1,2,3,4\n");
    let expected = list(vec![row(&[
        ("name", "1"),
        ("name_2", "2"),
        ("column3", "3"),
        ("name_3", "4"),
    ])]);
    same(&prepared.value, &expected);
    assert_eq!(
        diagnostics(&prepared),
        [
            json!({"code": "W_CSV_HEADER_RENAMED", "severity": "warning", "phase": "data",
            "span": {"source": "data", "line": 1, "column": 6, "record": 1},
            "message": "3 CSV header names were empty or repeated and were renamed; the first is column 2."})
        ]
    );
    // Generated names skip names already taken.
    let prepared = csv("a,a_2,a\n1,2,3\n");
    same(
        &prepared.value,
        &list(vec![row(&[("a", "1"), ("a_2", "2"), ("a_3", "3")])]),
    );
    let prepared = csv("column2,\n1,2\n");
    same(
        &prepared.value,
        &list(vec![row(&[("column2", "1"), ("column2_2", "2")])]),
    );
    assert_eq!(codes(&prepared), ["W_CSV_HEADER_RENAMED"]);
}

#[test]
fn without_a_header_columns_are_numbered_from_the_first_record() {
    let options = Options {
        header: false,
        ..Options::default()
    };
    let prepared = csv_with("1,2\n3,4,5\n6\n", &options);
    let expected = list(vec![
        row(&[("column1", "1"), ("column2", "2")]),
        row(&[("column1", "3"), ("column2", "4")]),
        record(vec![("column1", s("6")), ("column2", Value::null())]),
    ]);
    same(&prepared.value, &expected);
    assert_eq!(codes(&prepared), ["W_CSV_LONG_ROW", "W_CSV_SHORT_ROW"]);
}

#[test]
fn short_rows_get_nulls_and_long_rows_lose_their_extra_cells() {
    let prepared = csv("a,b,c\n1,2\n1,2,3\n1\n1,2,3,4,5\n1,2,3,4\n");
    let full = row(&[("a", "1"), ("b", "2"), ("c", "3")]);
    let expected = list(vec![
        record(vec![("a", s("1")), ("b", s("2")), ("c", Value::null())]),
        full.clone(),
        record(vec![
            ("a", s("1")),
            ("b", Value::null()),
            ("c", Value::null()),
        ]),
        full.clone(),
        full,
    ]);
    same(&prepared.value, &expected);
    assert_eq!(
        diagnostics(&prepared),
        [
            json!({"code": "W_CSV_SHORT_ROW", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 2, "column": 1, "record": 2},
                "message": "2 CSV records have missing cells; the first is record 2."}),
            json!({"code": "W_CSV_LONG_ROW", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 5, "column": 1, "record": 5},
                "message": "2 CSV records have extra cells, which were dropped; the first is record 5."}),
        ]
    );
}

#[test]
fn declared_columns_convert_cells_and_warn_once_per_column() {
    let options = columns(&[
        ("n", ColumnType::Number),
        ("b", ColumnType::Boolean),
        ("t", ColumnType::Text),
        ("z", ColumnType::Number),
    ]);
    let text = "id,n,b,t\n007, 12.5 ,true,00501\n2,,,x\n3,1 000,True,y\n4,abc,false,\n5,1e30,yes,z\n6,\u{3000}-1e2\u{3000},false\n";
    let prepared = csv_with(text, &options);
    let typed = |id: &str, n: Value, b: Value, t: Value| {
        record(vec![("id", s(id)), ("n", n), ("b", b), ("t", t)])
    };
    let expected = list(vec![
        typed("007", super::n("12.5"), Value::boolean(true), s("00501")),
        typed("2", Value::null(), Value::null(), s("x")),
        typed("3", error("E_NUMBER_PARSE"), error("E_TYPE"), s("y")),
        typed("4", error("E_NUMBER_PARSE"), Value::boolean(false), s("")),
        typed("5", error("E_OVERFLOW"), error("E_TYPE"), s("z")),
        // §4.3 White_Space is trimmed, including U+3000; the short row's
        // missing `t` is null, not a failed conversion.
        typed("6", super::n("-100"), Value::boolean(false), Value::null()),
    ]);
    same(&prepared.value, &expected);
    assert_eq!(
        diagnostics(&prepared),
        [
            json!({"code": "W_CSV_UNKNOWN_COLUMN", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 1, "column": 1, "record": 1},
                "subject": "z",
                "message": "A requested column is not in the CSV header."}),
            json!({"code": "W_CELL_TYPE", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 4, "column": 3, "record": 4},
                "subject": "n",
                "message": "3 values in this column could not be converted; the first is in row 3."}),
            json!({"code": "W_CELL_TYPE", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 4, "column": 9, "record": 4},
                "subject": "b",
                "message": "2 values in this column could not be converted; the first is in row 3."}),
            json!({"code": "W_CSV_SHORT_ROW", "severity": "warning", "phase": "data",
                "span": {"source": "data", "line": 7, "column": 1, "record": 7},
                "message": "1 CSV records have missing cells; the first is record 7."}),
        ]
    );
}

#[test]
fn the_other_delimiters_split_fields_and_commas_become_ordinary() {
    for (delimiter, text) in [
        (Delimiter::Semicolon, "a;b\n1,5;\"x;y\"\n"),
        (Delimiter::Tab, "a\tb\n1,5\t\"x\ty\"\n"),
        (Delimiter::Pipe, "a|b\n1,5|\"x|y\"\n"),
    ] {
        let options = Options {
            delimiter,
            ..Options::default()
        };
        let prepared = csv_with(text, &options);
        let separator = char::from(delimiter.byte());
        let expected = list(vec![row(&[("a", "1,5"), ("b", &format!("x{separator}y"))])]);
        same(&prepared.value, &expected);
        assert!(prepared.diagnostics.is_empty());
    }
}
