use serde_json::json;

use super::*;
use crate::{CodecMode, ScalarString};

mod caps;
mod csv;
mod json;

const SPAN: SourceSpan = SourceSpan {
    line: 3,
    column: 5,
    end_line: 3,
    end_column: 21,
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

fn encoded(value: &Value) -> String {
    value.encode_ordered(CodecMode::Event).unwrap()
}

fn same(actual: &Value, expected: &Value) {
    assert_eq!(encoded(actual), encoded(expected));
}

/// Parse with an unlimited account, returning the result and the fuel used.
fn run(bytes: &[u8], format: Format, options: &Options) -> (Prepared, u64) {
    let mut fuel = 0;
    let prepared = prepare(bytes, format, options, SPAN, &mut |cost| {
        fuel += cost;
        Ok::<_, ()>(())
    })
    .unwrap();
    (prepared, fuel)
}

fn json_with(text: &str, options: &Options) -> Prepared {
    run(text.as_bytes(), Format::Json, options).0
}
fn json(text: &str) -> Prepared {
    json_with(text, &Options::default())
}
fn csv_with(text: &str, options: &Options) -> Prepared {
    run(text.as_bytes(), Format::Csv, options).0
}
fn csv(text: &str) -> Prepared {
    csv_with(text, &Options::default())
}

fn columns(declared: &[(&str, ColumnType)]) -> Options {
    Options {
        columns: declared
            .iter()
            .map(|(name, column)| (name.to_string(), *column))
            .collect(),
        ..Options::default()
    }
}

fn diagnostics(prepared: &Prepared) -> Vec<serde_json::Value> {
    prepared
        .diagnostics
        .iter()
        .map(|d| serde_json::to_value(d).unwrap())
        .collect()
}

fn codes(prepared: &Prepared) -> Vec<&'static str> {
    prepared
        .diagnostics
        .iter()
        .map(|d| d.code().as_str())
        .collect()
}

/// A failed parse: one error diagnostic with this code at this data position,
/// `value` and `rows` both the error value, and `text` kept (§10.2).
fn fails_at(prepared: &Prepared, code: &str, line: u32, column: u32, record: Option<u64>) {
    assert_eq!(codes(prepared), [code], "{:?}", prepared.diagnostics);
    let mut span = json!({"source": "data", "line": line, "column": column});
    if let Some(record) = record {
        span["record"] = json!(record);
    }
    assert_eq!(diagnostics(prepared)[0]["span"], span);
    assert_eq!(diagnostics(prepared)[0]["severity"], "error");
    assert_eq!(diagnostics(prepared)[0]["phase"], "data");
    same(&prepared.value, &error(code));
    same(&prepared.rows, &error(code));
}

fn text_of(prepared: &Prepared) -> String {
    match prepared.text.kind() {
        crate::ValueKind::String(text) => text.as_str().to_owned(),
        _ => panic!("text is always a string"),
    }
}

#[test]
fn every_section_10_3_and_10_4_code_is_produced_without_quoting_data() {
    // Every string, key, header name and cell holds the marker, so a
    // diagnostic that quoted any of them would contain it (§10.6).
    let secret = "SECRET";
    let typed = columns(&[("n", ColumnType::Number), ("missing", ColumnType::Boolean)]);
    let cases: Vec<(Prepared, &str)> = vec![
        (json("[\"SECRET\" \"SECRET\"]"), "E_JSON_SYNTAX"),
        (json("{\"SECRET\":1,\"SECRET\":2}"), "E_JSON_DUPLICATE_KEY"),
        (
            run(b"[\"SECRET\",\xFF]", Format::Json, &Options::default()).0,
            "E_DATA_ENCODING",
        ),
        (csv("SECRET\n\"SECRET\"SECRET\n"), "E_CSV_SYNTAX"),
        (
            json(&format!("[\"{}\"]", "SECRET".repeat(11_000))),
            "E_DATA_LIMIT",
        ),
        (
            json("[\"SECRET\",1.0000000000000000001]"),
            "W_NUMBER_ROUNDED",
        ),
        (json("[\"SECRET\",1e20]"), "W_NUMBER_RANGE"),
        (json("[\"SECRET\\ud800\"]"), "W_JSON_SURROGATE"),
        (json_with("[{\"n\":\"SECRET\"}]", &typed), "W_CELL_TYPE"),
        (
            csv("SECRET,SECRET\nSECRET,SECRET\n"),
            "W_CSV_HEADER_RENAMED",
        ),
        (csv("SECRET,b\nSECRET\n"), "W_CSV_SHORT_ROW"),
        (csv("SECRET\nSECRET,SECRET\n"), "W_CSV_LONG_ROW"),
        (csv_with("n\nSECRET\n", &typed), "W_CSV_UNKNOWN_COLUMN"),
    ];
    let mut produced = Vec::new();
    for (prepared, code) in &cases {
        assert!(
            codes(prepared).contains(code),
            "{code}: {:?}",
            prepared.diagnostics
        );
        produced.push(*code);
        for diagnostic in diagnostics(prepared) {
            let rendered = diagnostic.to_string();
            assert!(!rendered.contains(secret), "{rendered}");
            assert_eq!(diagnostic["phase"], "data");
        }
    }
    produced.sort_unstable();
    let mut expected = vec![
        "E_DATA_ENCODING",
        "E_JSON_SYNTAX",
        "E_JSON_DUPLICATE_KEY",
        "E_CSV_SYNTAX",
        "E_DATA_LIMIT",
        "W_NUMBER_ROUNDED",
        "W_NUMBER_RANGE",
        "W_JSON_SURROGATE",
        "W_CELL_TYPE",
        "W_CSV_HEADER_RENAMED",
        "W_CSV_SHORT_ROW",
        "W_CSV_LONG_ROW",
        "W_CSV_UNKNOWN_COLUMN",
    ];
    expected.sort_unstable();
    assert_eq!(produced, expected);
}

#[test]
fn preparation_charges_one_per_byte_and_two_per_value_or_cell() {
    // 18 bytes; values: the array, 1, "a", the object and null.
    assert_eq!(
        run(b"[1,\"a\",{\"b\":null}]", Format::Json, &Options::default()).1,
        18 + 2 * 5
    );
    // The byte-order mark is input: 3 + 2 bytes, one value.
    assert_eq!(
        run(b"\xEF\xBB\xBF[]", Format::Json, &Options::default()).1,
        5 + 2
    );
    // 8 bytes, four cells including the header's.
    assert_eq!(
        run(b"a,b\n1,2\n", Format::Csv, &Options::default()).1,
        8 + 2 * 4
    );
    // A missing cell is not in the input, so it is not charged.
    assert_eq!(
        run(b"a,b\n1", Format::Csv, &Options::default()).1,
        5 + 2 * 3
    );
}

#[test]
fn a_refused_charge_stops_the_parse_and_is_returned() {
    // JSON: 7 bytes, the array (9), then `1` would reach 11. CSV: 6 bytes,
    // cells `a` (8) and `1` (10), then `2` would reach 12.
    for (bytes, format, charged) in [
        (&b"[1,2,3]"[..], Format::Json, 9),
        (&b"a\n1\n2\n"[..], Format::Csv, 10),
    ] {
        let mut spent = 0;
        let outcome = prepare(bytes, format, &Options::default(), SPAN, &mut |cost| {
            if spent + cost > 10 {
                return Err("exhausted");
            }
            spent += cost;
            Ok(())
        });
        assert_eq!(outcome.err(), Some("exhausted"));
        assert_eq!(spent, charged);
    }
}

#[test]
fn text_drops_only_a_utf8_byte_order_mark_and_repairs_invalid_sequences() {
    let prepared = run(
        b"\xEF\xBB\xBF{\"a\":\r\n1}",
        Format::Json,
        &Options::default(),
    )
    .0;
    assert_eq!(text_of(&prepared), "{\"a\":\r\n1}");
    let prepared = run(b"a\xFFb", Format::Csv, &Options::default()).0;
    assert_eq!(text_of(&prepared), "a\u{FFFD}b");
}

#[test]
fn positions_count_scalars_and_lines_ending_at_lf_crlf_or_cr() {
    let text = "ab\ncé\r\nd\re";
    let resolved = positions(text, vec![0, 3, 4, 8, 10, 11, 99]);
    assert_eq!(resolved[&0], (1, 1));
    assert_eq!(resolved[&3], (2, 1));
    // `é` is two bytes but one scalar.
    assert_eq!(resolved[&4], (2, 2));
    assert_eq!(resolved[&8], (3, 1));
    assert_eq!(resolved[&10], (4, 1));
    assert_eq!(resolved[&11], (4, 2));
    assert_eq!(resolved[&99], (4, 2));
}
