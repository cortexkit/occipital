//! Each §10.5 cap passes at its value and fails one unit beyond it with
//! `E_DATA_LIMIT` naming the cap, while `text` stays available.

use super::*;

/// A failed parse naming `cap`, with `text` kept.
fn breaks(prepared: &Prepared, cap: &str, input: &[u8]) {
    assert_eq!(
        codes(prepared),
        ["E_DATA_LIMIT"],
        "{:?}",
        prepared.diagnostics
    );
    let diagnostic = &diagnostics(prepared)[0];
    assert_eq!(diagnostic["subject"], cap);
    assert!(
        diagnostic["message"].as_str().unwrap().contains("cap"),
        "{diagnostic}"
    );
    same(&prepared.value, &error("E_DATA_LIMIT"));
    same(&prepared.rows, &error("E_DATA_LIMIT"));
    assert_eq!(text_of(prepared).as_bytes(), input);
}

fn passes(prepared: &Prepared) {
    assert!(
        !codes(prepared).contains(&"E_DATA_LIMIT"),
        "{:?}",
        prepared.diagnostics
    );
}

fn both(format: Format, at_cap: String, over: String, cap: &str) -> (Prepared, Prepared) {
    let options = Options::default();
    let fine = run(at_cap.as_bytes(), format, &options).0;
    passes(&fine);
    let broken = run(over.as_bytes(), format, &options).0;
    breaks(&broken, cap, over.as_bytes());
    (fine, broken)
}

#[test]
fn bytes_parsed_cap_is_two_mebibytes() {
    // JSON white space pads a one-byte document to exactly 2 MiB.
    let at_cap = format!("0{}", " ".repeat(2 * 1024 * 1024 - 1));
    let over = format!("{at_cap} ");
    let (_, broken) = both(Format::Json, at_cap, over, "bytes");
    // Nothing is parsed or charged past the cap, so there is no position.
    assert_eq!(diagnostics(&broken)[0].get("span"), None);
    assert_eq!(
        diagnostics(&broken)[0]["message"],
        "Data exceeds the cap of 2 MiB parsed bytes."
    );

    // A header line, 9,999 rows of 209 bytes and a last row filling the rest.
    let mut at_cap = String::from("a\n");
    for _ in 0..9_999 {
        at_cap.push_str(&"x".repeat(208));
        at_cap.push('\n');
    }
    at_cap.push_str(&"x".repeat(2 * 1024 * 1024 - at_cap.len()));
    assert_eq!(at_cap.len(), 2_097_152);
    let over = format!("{at_cap}x");
    both(Format::Csv, at_cap, over, "bytes");
}

#[test]
fn nesting_depth_cap_is_32_arrays_and_objects() {
    let nest = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let (_, broken) = both(Format::Json, nest(32), nest(33), "depth");
    // The 33rd opening bracket.
    assert_eq!(
        diagnostics(&broken)[0]["span"],
        json!({"source": "data", "line": 1, "column": 33})
    );
    // Objects count too.
    let objects = format!(
        "{}0{}",
        "{\"k\":".repeat(16) + &"[".repeat(17),
        "]".repeat(17) + &"}".repeat(16)
    );
    breaks(&json(&objects), "depth", objects.as_bytes());
}

#[test]
fn rows_cap_is_10000_root_elements_or_records_after_the_header() {
    let elements = |count: usize| format!("[{}]", vec!["0"; count].join(","));
    let (fine, broken) = both(Format::Json, elements(10_000), elements(10_001), "rows");
    assert_eq!(
        fine.value.list_element(9_999).value_type(),
        crate::ValueType::Number
    );
    // The 10,001st element starts at byte 1 + 2 × 10,000.
    assert_eq!(
        diagnostics(&broken)[0]["span"],
        json!({"source": "data", "line": 1, "column": 20_002})
    );
    // Nested arrays are not rows.
    passes(&json(&format!("[{}]", elements(10_001))));

    let records = |count: usize| format!("h\n{}", "1\n".repeat(count));
    let (_, broken) = both(Format::Csv, records(10_000), records(10_001), "rows");
    assert_eq!(
        diagnostics(&broken)[0]["span"],
        json!({"source": "data", "line": 10_002, "column": 1, "record": 10_002})
    );
}

#[test]
fn columns_cap_is_256() {
    let record = |count: usize| {
        let fields: Vec<String> = (1..=count).map(|i| format!("\"k{i}\":0")).collect();
        format!("[{{{}}}]", fields.join(","))
    };
    both(Format::Json, record(256), record(257), "columns");
    // Only a row record's fields are columns.
    let root = record(257);
    passes(&json(&root[1..root.len() - 1]));

    let header = |count: usize| {
        let names: Vec<String> = (1..=count).map(|i| format!("c{i}")).collect();
        format!("{}\n", names.join(","))
    };
    let (_, broken) = both(Format::Csv, header(256), header(257), "columns");
    // The 257th name starts right after the 256th name and its delimiter.
    let column = header(256).len() as u64 + 1;
    assert_eq!(
        diagnostics(&broken)[0]["span"],
        json!({"source": "data", "line": 1, "column": column, "record": 1})
    );
}

#[test]
fn cells_cap_is_200000_values_or_cells() {
    // The root array, the inner array and its elements are all values.
    let values = |count: usize| format!("[[{}]]", vec!["0"; count - 2].join(","));
    both(Format::Json, values(200_000), values(200_001), "cells");

    // 20 cells in each of 10,000 records, the header's included; then one
    // extra cell in the last record.
    let line = vec!["1"; 20].join(",");
    let at_cap = format!("{}\n", vec![line.as_str(); 10_000].join("\n"));
    let over = format!("{}\n{line},1\n", vec![line.as_str(); 9_999].join("\n"));
    let (_, broken) = both(Format::Csv, at_cap, over, "cells");
    assert_eq!(
        diagnostics(&broken)[0]["span"],
        json!({"source": "data", "line": 10_000, "column": 41, "record": 10_000})
    );
}

#[test]
fn one_string_or_cell_cap_is_65536_scalars() {
    // Two-byte scalars: the cap counts scalars, not bytes.
    let string = |count: usize| format!("[\"{}\"]", "é".repeat(count));
    both(Format::Json, string(65_536), string(65_537), "string");
    let key = |count: usize| format!("{{\"{}\":0}}", "k".repeat(count));
    both(Format::Json, key(65_536), key(65_537), "string");
    // An escape decodes to one scalar.
    let escaped = |count: usize| format!("\"{}\"", "\\n".repeat(count));
    both(Format::Json, escaped(65_536), escaped(65_537), "string");

    let cell = |count: usize| format!("h\n{}\n", "é".repeat(count));
    both(Format::Csv, cell(65_536), cell(65_537), "string");
    let quoted = |count: usize| format!("h\n\"{}\"\n", "\"\"".repeat(count));
    both(Format::Csv, quoted(65_536), quoted(65_537), "string");
    let header = |count: usize| format!("{}\n", "h".repeat(count));
    both(Format::Csv, header(65_536), header(65_537), "string");
}

#[test]
fn expanded_logical_size_cap_is_16_mebibytes() {
    // One column named by 65,000 scalars, 64 rows. §9.3 sizes: the list 16;
    // each row record 16 + (16 + 4 × 65,000) for its field plus the cell's
    // 16 + 4L. With L = 33,532 in the first row and 0 elsewhere:
    // 16 + 64 × 260,048 + 4 × 33,532 = 16,777,216 exactly.
    let table = |first: usize| {
        let mut text = format!("{}\n{}\n", "k".repeat(65_000), "x".repeat(first));
        text.push_str(&"\"\"\n".repeat(63));
        text
    };
    let (fine, broken) = both(Format::Csv, table(33_532), table(33_533), "size");
    assert_eq!(fine.value.logical_size(), 16 * 1024 * 1024);
    // Sizes grow in steps of 4 bytes (one scalar), so one more scalar is the
    // smallest possible breach. The total first passes the cap at the last
    // row's cell: line 65, record 65.
    assert_eq!(
        diagnostics(&broken)[0]["span"],
        json!({"source": "data", "line": 65, "column": 1, "record": 65})
    );
}
