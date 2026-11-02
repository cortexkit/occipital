//! A view's templates are checked against the tool catalog, with `input`
//! typed from the tool's own schema and `view` from the view's record schema.

use cortexkit_role_tool_display::RecordSchema;
use occipital_lang::DiagnosticCode;
use occipital_registry::view::{
    check_view, row_heights, tool_catalog_digest, TemplateReport, SAMPLE_WIDTHS,
};
use serde_json::{json, Value};

/// A tool's input schema as a provider's catalog serves it, with keywords the
/// language's subset doesn't have.
fn bash_input() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "command": {"type": "string", "description": "The command to run."},
            "description": {"type": "string"},
            "timeout": {"type": ["integer", "null"], "minimum": 1, "default": 120},
            "env": {"anyOf": [{"type": "object"}, {"type": "null"}]}
        },
        "required": ["command"]
    })
}

fn record_schema() -> RecordSchema {
    serde_json::from_value(json!({
        "name": "aft.bash", "revision": 1, "fields": {"exitCode": "integer"}
    }))
    .unwrap()
}

fn template(profile: &str, source: &str) -> Value {
    template_naming(profile, source, &tool_catalog_digest())
}

fn template_naming(profile: &str, source: &str, digest: &str) -> Value {
    json!({
        "schema": "occipital.template/1",
        "profile": profile,
        "executionDialect": "1.0",
        "catalog": {"version": "1.1", "digest": digest},
        "reads": [],
        "source": source,
    })
}

/// A template naming the tool catalog's earlier digest still passes, since
/// the catalog only grew from it and views already served name it; one naming
/// any other digest doesn't. A `narrow` Slot checks under the current digest.
/// (`occipital-check`, the authors' gate, still asks new templates for the
/// current digest.)
#[test]
fn a_template_naming_the_earlier_tool_catalog_digest_still_passes() {
    let earlier = "sha256:9a23139f4f1f48d6ed366a4a48b9221b1e5ac0ccd87473c0b241b8a37a876fbf";
    assert_ne!(earlier, tool_catalog_digest());
    let passed = |digest: &str| {
        reports(json!({"row": template_naming("tool_row", ROW, digest)}))[0].passed()
    };
    assert!(passed(earlier));
    assert!(passed(&tool_catalog_digest()));
    assert!(!passed(&format!("sha256:{}", "0".repeat(64))));
    assert!(!passed(earlier.trim_start_matches("sha256:")));
    let narrow = ROW.replace("width=\"short\"", "width=\"narrow\"");
    assert!(reports(json!({"row": template("tool_row", &narrow)}))[0].passed());
}

const ROW: &str = "<Card appearance=\"clear\" direction=\"row\" gap=\"s\">\n<InlineCode code={input.command} />\n<Slot width=\"short\" needs={view.record}>\n<Badge text={view.record.exitCode == 0 ? \"Done\" : \"Failed\"} />\n</Slot>\n</Card>\n";

fn reports(view_template: Value) -> Vec<TemplateReport> {
    check_view(&view_template, &record_schema(), &bash_input()).unwrap()
}

#[test]
fn a_row_that_reads_its_record_inside_a_slot_passes_with_a_digest() {
    let page = "<TextContent text={view.output ?? \"\"} />\n";
    let reports =
        reports(json!({"row": template("tool_row", ROW), "page": template("tool_page", page)}));
    assert_eq!(reports.len(), 2);
    for report in &reports {
        assert!(
            report.passed(),
            "{}: {:?} {:?}",
            report.slot,
            report.problems,
            report.diagnostics
        );
        assert!(report.digest.as_deref().unwrap().starts_with("sha256:"));
    }
    assert_eq!((reports[0].slot, reports[1].slot), ("row", "page"));
}

#[test]
fn a_row_that_reads_its_record_outside_a_slot_fails() {
    let source = "<Badge text={view.record.exitCode == 0 ? \"Done\" : \"Failed\"} />\n";
    let reports = reports(json!({"row": template("tool_row", source)}));
    assert!(!reports[0].passed());
    assert!(reports[0]
        .diagnostics
        .iter()
        .any(|d| d.code() == DiagnosticCode::UnslottedRead));
}

/// Problems with the document itself stop before its source is read.
#[test]
fn a_template_for_the_wrong_slot_catalog_or_with_reads_is_refused() {
    let mut old = template("tool_row", ROW);
    old["catalog"] = json!({"version": "1.0", "digest": format!("sha256:{}", "c".repeat(64))});
    let mut reading = template("tool_page", "<Divider />\n");
    reading["reads"] = json!([{"id": "status", "version": 1}]);
    let mut renamed = template("tool_row", ROW);
    renamed["schema"] = json!("ckios.richui.template/1");
    for (view_template, expected) in [
        (json!({"row": template("tool_page", ROW)}), "profile"),
        (json!({"row": old}), "catalog"),
        (json!({"page": reading}), "reads"),
        (json!({"row": renamed}), "occipital.template/1"),
    ] {
        let report = &reports(view_template)[0];
        assert!(!report.passed());
        assert!(report.diagnostics.is_empty(), "the source isn't checked");
        assert!(
            report.problems.iter().any(|p| p.contains(expected)),
            "{expected}: {:?}",
            report.problems
        );
    }
}

/// A view without templates draws the plain chip; a member other than row and
/// page is an error, since an app would never draw it.
#[test]
fn a_view_holds_only_row_and_page_templates() {
    assert!(reports(json!({})).is_empty());
    assert!(check_view(&json!({"summary": {}}), &record_schema(), &bash_input()).is_err());
    assert!(check_view(&json!(null), &record_schema(), &bash_input()).is_err());
}

/// The input schema's types reach the checker: a string field used as a
/// number is caught, while keywords the subset lacks are dropped, never
/// refused.
#[test]
fn the_input_is_typed_from_the_tool_schema() {
    let source = "<Badge text={input.command} />\n";
    assert!(reports(json!({"row": template("tool_row", source)}))[0].passed());
    let wrong = "<Badge text={input.command * 2} />\n";
    let report = &reports(json!({"row": template("tool_row", wrong)}))[0];
    assert!(!report.passed(), "{:?}", report.diagnostics);
}

/// The guide's examples pass the checker with no diagnostic at all, so the
/// guide can't teach something the checker refuses.
#[test]
fn the_guide_examples_pass_cleanly() {
    let guide = include_str!("../TEMPLATES.md");
    let mut checked = 0;
    for (slot, profile) in [("row", "tool_row"), ("page", "tool_page")] {
        let fence = format!("```occipital-{slot}\n");
        for block in guide.split(&fence).skip(1) {
            let source = block.split("```").next().unwrap();
            let report = &reports(json!({slot: template(profile, source)}))[0];
            assert!(
                report.passed() && report.diagnostics.is_empty(),
                "{slot}: {:?} {:?}",
                report.problems,
                report.diagnostics
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 2);
}

/// A sample row keeps its height when its record arrives, at both widths,
/// even when the record's text would wrap; a record its schema refuses is
/// reported rather than measured.
#[test]
fn sample_rows_keep_their_height_and_bad_records_are_reported() {
    let row = template(
        "tool_row",
        "<Card appearance=\"clear\" direction=\"row\" gap=\"s\">\n<InlineCode code={input.command} />\n<Slot width=\"short\" needs={view.record}>\n<TextContent text={\"exit \" + Text(view.record.exitCode) + \" after a long run with plenty of output\"} />\n</Slot>\n</Card>\n",
    );
    let input = json!({"command": "cargo test --workspace"});
    let record = json!({"durationMs": 1500, "outcome": "success", "exitCode": 0});
    let heights = row_heights(
        &row,
        &record_schema(),
        &bash_input(),
        &input,
        Some(&record),
        None,
    )
    .unwrap();
    assert_eq!(
        heights.iter().map(|h| h.width).collect::<Vec<_>>(),
        SAMPLE_WIDTHS
    );
    for height in &heights {
        assert_eq!(height.waiting, height.filled, "width {}", height.width);
    }
    let refused = json!({"durationMs": 1500, "outcome": "success", "exitCode": "zero"});
    let error = row_heights(
        &row,
        &record_schema(),
        &bash_input(),
        &input,
        Some(&refused),
        None,
    )
    .unwrap_err();
    assert!(error.contains("refuses the sample's record"), "{error}");
}

/// The clock, the agent and the model draw in rows and pages, and the item
/// link in pages only, since tapping a row already opens its page.
#[test]
fn the_app_drawn_parts_check_in_their_profiles() {
    let row = "<Card appearance=\"clear\" direction=\"row\" gap=\"s\">\n<Agent id={input.command} />\n<Model id=\"anthropic/opus-5.5\" />\n<Slot width=\"short\" needs={view.call.startedAtMs}>\n<Elapsed start={view.call.startedAtMs ?? 0} />\n</Slot>\n</Card>\n";
    let page = "Opened <OpenItem id=\"ask_1\" label=\"the question\" />.\n";
    let reports =
        reports(json!({"row": template("tool_row", row), "page": template("tool_page", page)}));
    for report in &reports {
        assert!(
            report.passed(),
            "{}: {:?} {:?}",
            report.slot,
            report.problems,
            report.diagnostics
        );
    }
    let report = &reports_for_row(page)[0];
    assert!(!report.passed(), "OpenItem is not a row part");
}

fn reports_for_row(source: &str) -> Vec<TemplateReport> {
    reports(json!({"row": template("tool_row", source)}))
}
