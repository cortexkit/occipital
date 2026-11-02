//! `occipital-check`: check a tool provider's views before it ships them.
//!
//! ```text
//! occipital-check <file>...
//! occipital-check --catalog
//! ```
//!
//! `--catalog` prints the tool catalog's version and digest, the values every
//! template's `catalog` member must hold. Occipital still draws a template
//! that names one of the catalog's earlier digests, which the catalog only
//! grew from, but this check asks a template for the current one, since app
//! builds that know only an earlier digest lack what was added since.
//!
//! Each file holds one view, the input schema of the tool it is for, and
//! optionally sample calls:
//!
//! ```json
//! {"input_schema": {...},
//!  "view": {"record_schema": {...}, "reads": [], "template": {"row": {...}, "page": {...}}},
//!  "samples": [{"input": {...}, "record": {...}, "completion": {...}}]}
//! ```
//!
//! `input_schema` is the tool's `input_schema` from the provider's
//! `tool.catalog`, and `view` is the provider's `tool.views` answer for the
//! tool's pin. Occipital runs the same checks when it fetches the view, and an
//! app draws only templates that pass them.
//!
//! It prints one line per template that passes, with its digest, and one line
//! per problem or diagnostic otherwise, as `<file>: <row|page>: ...`. For each
//! sample, it prints the row's height at a phone's and a desktop's width,
//! before anything is written for the call and with the sample's record and
//! completion; a row whose two heights differ fails. It exits 0 when every
//! template and sample passes, 1 when one doesn't, and 2 when a file can't be
//! read as a view.

use std::process::ExitCode;

use cortexkit_role_tool_display::RecordSchema;
use occipital_lang::{DiagnosticSpan, Height, Severity};
use occipital_registry::view::{check_view, row_heights};
use serde_json::Value as Json;

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files == ["--catalog"] {
        println!(
            "{{\"version\":\"{}\",\"digest\":\"{}\"}}",
            occipital_registry::view::TOOL_CATALOG_VERSION,
            occipital_registry::view::tool_catalog_digest()
        );
        return ExitCode::SUCCESS;
    }
    if files.is_empty() || files.iter().any(|file| file.starts_with('-')) {
        eprintln!("usage: occipital-check <file>... | occipital-check --catalog");
        return ExitCode::from(2);
    }
    let mut status = ExitCode::SUCCESS;
    for file in &files {
        match check(file) {
            Ok(true) => {}
            Ok(false) => status = ExitCode::from(1),
            Err(error) => {
                eprintln!("{file}: {error}");
                return ExitCode::from(2);
            }
        }
    }
    status
}

/// Check one file, printing what it finds; `Ok(true)` when every template
/// passes.
fn check(file: &str) -> Result<bool, String> {
    let text = std::fs::read_to_string(file).map_err(|error| error.to_string())?;
    let document: Json = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let view = document.get("view").ok_or("the file has no view member")?;
    let input_schema = document
        .get("input_schema")
        .ok_or("the file has no input_schema member")?;
    let record_schema: RecordSchema = serde_json::from_value(
        view.get("record_schema")
            .cloned()
            .ok_or("the view has no record_schema")?,
    )
    .map_err(|error| format!("the view's record_schema: {error}"))?;
    record_schema
        .validate()
        .map_err(|error| format!("the view's record_schema: {error}"))?;
    let mut passed = true;
    if view
        .get("reads")
        .and_then(Json::as_array)
        .is_some_and(|reads| !reads.is_empty())
    {
        println!("{file}: the view declares reads; views make no reads in this version");
        passed = false;
    }
    let template = view.get("template").ok_or("the view has no template")?;
    let reports = check_view(template, &record_schema, input_schema)?;
    let current = occipital_registry::view::tool_catalog_digest();
    for report in &reports {
        let named = template[report.slot]["catalog"]["digest"].as_str();
        if report.passed() && named.is_some_and(|named| named != current) {
            println!(
                "{file}: {}: it names an earlier tool catalog digest; name {current}",
                report.slot
            );
            passed = false;
        } else if report.passed() {
            println!(
                "{file}: {}: passed ({})",
                report.slot,
                report.digest.as_deref().unwrap_or("no digest")
            );
        } else {
            passed = false;
        }
        for problem in &report.problems {
            println!("{file}: {}: {problem}", report.slot);
        }
        for diagnostic in &report.diagnostics {
            let position = match diagnostic.span() {
                Some(DiagnosticSpan::Source(span)) => format!("{}:{}: ", span.line, span.column),
                _ => String::new(),
            };
            let severity = match diagnostic.severity() {
                Severity::Error => "error",
                _ => "warning",
            };
            println!(
                "{file}: {}: {position}{severity} {}: {}",
                report.slot,
                diagnostic.code(),
                diagnostic.message()
            );
        }
    }
    let samples = match document.get("samples") {
        None => &Vec::new(),
        Some(Json::Array(samples)) => samples,
        Some(_) => return Err("samples is not a list".into()),
    };
    let row = template.get("row");
    let row_passed = reports
        .iter()
        .any(|report| report.slot == "row" && report.passed());
    for (index, sample) in samples.iter().enumerate() {
        let n = index + 1;
        let (Some(row), true) = (row, row_passed) else {
            println!("{file}: sample {n}: not measured: the view has no row that passes");
            continue;
        };
        let input = sample
            .get("input")
            .ok_or(format!("sample {n} has no input"))?;
        let heights = row_heights(
            row,
            &record_schema,
            input_schema,
            input,
            sample.get("record"),
            sample.get("completion"),
        );
        match heights {
            Err(error) => {
                println!("{file}: sample {n}: {error}");
                passed = false;
            }
            Ok(heights) => {
                for height in heights {
                    let same = height.waiting == height.filled;
                    println!(
                        "{file}: sample {n}: width {}: {} before its record, {} with it{}",
                        height.width,
                        units(height.waiting),
                        units(height.filled),
                        if same {
                            ""
                        } else {
                            ": error: the row changes height"
                        }
                    );
                    passed &= same;
                }
            }
        }
    }
    Ok(passed)
}

fn units(height: Height) -> String {
    match height {
        Height::Known(units) => units.to_string(),
        Height::ProseDeferred => "measured by the app".into(),
    }
}
