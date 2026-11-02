use std::ptr;

use serde_json::{json, Value as Json};

use super::*;
use crate::status::{OCC_INTERNAL, OCC_INVALID_ARGUMENT, OCC_OK, OCC_REJECTED};
use crate::testing::{c, call, json};

const CALC_PAINT: &str =
    include_str!("../../../occipital-lang/fixtures/section16/sources/16.05-calc-paint.ui");
const CALC_PAINT_TRACE: &str =
    include_str!("../../../occipital-lang/fixtures/section16/golden/16.05-calc-paint.trace");
const DATA_SALES: &str =
    include_str!("../../../occipital-lang/fixtures/section16/sources/16.06-data-sales.ui");
const DATA_SALES_TRACE: &str =
    include_str!("../../../occipital-lang/fixtures/section16/golden/16.06-data-sales.trace");
// A Bash row and page written for Catalog 1.0's site reads, before tool
// templates (`occipital.template/1`) existed; see the language test that checks them.
const BASH_ROW: &str = include_str!("../../../occipital-lang/fixtures/bash-site-reads/row.ui");
const BASH_PAGE: &str = include_str!("../../../occipital-lang/fixtures/bash-site-reads/page.ui");
const BASH_REGISTRY: &str =
    include_str!("../../../occipital-lang/fixtures/bash-site-reads/check/registry.json");
const BASH_HOSTS: &str =
    include_str!("../../../occipital-lang/fixtures/bash-site-reads/check/hosts.json");

/// A `context` member for `profile` with no registry, tool or records.
fn context(profile: &str) -> Json {
    json!({
        "profile": profile,
        "execution_dialect": "1.0",
        "registry": {},
        "tool": null,
        "show_ui": {"title": "", "summary": "", "data_format": null, "data_columns": {}},
        "references": {},
        "handles": [],
    })
}

/// A complete `host` member with the default host values.
fn default_host() -> Json {
    json!({"network": true, "now": "0", "zone": "UTC", "data": null, "snapshots": {}})
}

/// Calls `occ_program_check` on `source` with `ctx` as `ctx_json`.
fn check(source: &[u8], ctx: &str) -> (i32, Option<String>) {
    let ctx = c(ctx);
    // SAFETY: `source` is valid for its length and `ctx` a live C string.
    call(|out| unsafe { occ_program_check(source.as_ptr(), source.len(), ctx.as_ptr(), out) })
}

/// The check result of an `OCC_OK` call.
fn checked(source: &[u8], ctx: &Json) -> Json {
    let (status, out) = check(source, &ctx.to_string());
    assert_eq!(status, OCC_OK, "{out:?}");
    json(&out.unwrap())
}

/// The check result a golden trace recorded for its first case.
fn golden_check(trace: &str) -> &str {
    trace
        .lines()
        .find_map(|line| line.strip_prefix("check="))
        .unwrap()
}

fn codes(result: &Json) -> Vec<String> {
    result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_valid_program_is_ok_with_the_library_check_result() {
    let (status, out) = check(
        CALC_PAINT.as_bytes(),
        &json!({"context": context("show_ui")}).to_string(),
    );
    assert_eq!(status, OCC_OK);
    // The §16.5 golden trace records the library's check result for the
    // same source, context and default host values; the FFI hands back
    // exactly that JSON.
    let golden = golden_check(CALC_PAINT_TRACE);
    assert_eq!(out.as_deref(), Some(golden));
    let result = json(golden);
    assert_eq!(result["ok"], true);
    assert_eq!(result["trial"]["outcome"], "ok");
    // A present host with the default values gives the same result.
    let explicit = checked(
        CALC_PAINT.as_bytes(),
        &json!({"context": context("show_ui"), "host": default_host()}),
    );
    assert_eq!(explicit, result);
}

#[test]
fn error_diagnostics_are_ok_with_the_diagnostics_in_the_result() {
    let source = b"<CardHeader title={missingName} />";
    let result = checked(source, &json!({"context": context("show_ui")}));
    assert_eq!(result["ok"], false, "{result}");
    assert!(
        codes(&result).contains(&"E_UNKNOWN_NAME".to_owned()),
        "{result}"
    );
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["severity"] == "error"));
}

#[test]
fn invalid_utf8_source_is_ok_with_w_encoding() {
    let source = b"<CardHeader title=\"caf\xC3 \xFF\" subtitle=\"x\" />";
    let result = checked(source, &json!({"context": context("show_ui")}));
    assert!(
        codes(&result).contains(&"W_ENCODING".to_owned()),
        "{result}"
    );
    // A split sequence at the very end is repaired the same way.
    let result = checked(
        b"<CardHeader title=\"x\" />\xE2\x98",
        &json!({"context": context("show_ui")}),
    );
    assert!(
        codes(&result).contains(&"W_ENCODING".to_owned()),
        "{result}"
    );
}

#[test]
fn a_registry_and_tool_inputs_reach_the_checker() {
    let hosts: serde_json::Map<String, Json> = serde_json::from_str(BASH_HOSTS).unwrap();
    let registry: Json = serde_json::from_str(BASH_REGISTRY).unwrap();
    for (name, tool) in hosts {
        for (source, profile) in [(BASH_ROW, "tool_row"), (BASH_PAGE, "tool_page")] {
            let mut ctx = context(profile);
            ctx["registry"] = registry.clone();
            ctx["tool"] = tool.clone();
            let result = checked(source.as_bytes(), &json!({ "context": ctx }));
            assert_eq!(result["ok"], true, "{name} {profile}: {result}");
            assert_eq!(
                result["diagnostics"],
                json!([]),
                "{name} {profile}: {result}"
            );
        }
    }
    // The page reads AFT's operations, so without the registry it does not
    // check clean: the registry above was decoded and used.
    let result = checked(
        BASH_PAGE.as_bytes(),
        &json!({"context": context("tool_page")}),
    );
    assert_eq!(result["ok"], false, "{result}");
}

/// Calls `occ_template_evaluate` on `source` with `ctx` as `ctx_json`.
fn evaluate(source: &[u8], ctx: &Json) -> (i32, Option<String>) {
    let ctx = c(&ctx.to_string());
    // SAFETY: `source` is valid for its length and `ctx` a live C string.
    call(|out| unsafe { occ_template_evaluate(source.as_ptr(), source.len(), ctx.as_ptr(), out) })
}

/// A tool row context on the tool catalog with `view` as the call's view.
fn row_context(view: Json) -> Json {
    let mut ctx = context("tool_row");
    ctx["catalog"] = json!("1.1");
    ctx["tool"] = json!({
        "input": {"value": {"command": "ls"}, "schema": {}},
        "view": {"value": view, "schema": {}},
        "truncated": false,
        "truncated_fields": [],
    });
    json!({ "context": ctx })
}

#[test]
fn a_tool_template_evaluates_to_a_tree_that_follows_its_view() {
    let row = "<Slot width=\"short\" needs={view.record}>\n<Badge text={\"exit \" + Text(view.record.exitCode)} />\n</Slot>\n";
    let (status, out) = evaluate(row.as_bytes(), &row_context(json!({"record": null})));
    assert_eq!(status, OCC_OK, "{out:?}");
    let waiting = json(&out.unwrap());
    assert_eq!(waiting["outcome"], "ok");
    assert_eq!(waiting["diagnostics"], json!([]));
    let (status, out) = evaluate(
        row.as_bytes(),
        &row_context(json!({"record": {"exitCode": 3}})),
    );
    assert_eq!(status, OCC_OK, "{out:?}");
    let filled = json(&out.unwrap());
    assert!(
        waiting["tree"].to_string().contains("\"waiting\":true"),
        "{waiting}"
    );
    assert!(filled["tree"].to_string().contains("exit 3"), "{filled}");
    // A template the checker refuses never evaluates.
    let unslotted = "<Badge text={Text(view.record.exitCode)} />\n";
    let (status, out) = evaluate(unslotted.as_bytes(), &row_context(json!({"record": null})));
    assert_eq!(status, OCC_REJECTED, "{out:?}");
    assert!(out.unwrap().contains("E_UNSLOTTED_READ"));
    let (status, _) = evaluate(b"<Divider />\n", &json!({"context": context("show_ui")}));
    assert_eq!(status, OCC_INTERNAL);
}

#[test]
fn the_data_snapshot_reaches_the_trial() {
    let data = include_bytes!("../../../occipital-lang/fixtures/section16/data-sales.json");
    let mut ctx = context("show_ui");
    ctx["show_ui"] = json!({
        "title": "Monthly sales, 2025",
        "summary": "Total, best month, chart and table for one region at a time",
        "data_format": "json",
        "data_columns": {},
    });
    let mut host = default_host();
    host["data"] = json!({
        "record_id": "tc-494",
        "record": {"state": "resolved", "reason": null, "tool": "bash", "bytes": data.len(), "call_key": "call-494"},
        "bytes": crate::base64::encode(data),
    });
    let (status, out) = check(
        DATA_SALES.as_bytes(),
        &json!({"context": ctx, "host": host}).to_string(),
    );
    assert_eq!(status, OCC_OK);
    // The §16.6 golden trace's check ran over the same data snapshot.
    assert_eq!(out.as_deref(), Some(golden_check(DATA_SALES_TRACE)));
    // Without the snapshot the trial does different work.
    let without = checked(DATA_SALES.as_bytes(), &json!({ "context": ctx }));
    assert_ne!(without, json(golden_check(DATA_SALES_TRACE)));
}

#[test]
fn bad_ctx_json_is_an_invalid_argument_with_out_null() {
    let mut cases: Vec<Json> = Vec::new();
    // `host: null`, a host missing a member, and unknown members.
    cases.push(json!({"context": context("show_ui"), "host": null}));
    for member in ["network", "now", "zone", "data", "snapshots"] {
        let mut host = default_host();
        host.as_object_mut().unwrap().remove(member);
        cases.push(json!({"context": context("show_ui"), "host": host}));
    }
    let mut extra = default_host();
    extra["extra"] = json!(1);
    cases.push(json!({"context": context("show_ui"), "host": extra}));
    cases.push(json!({"context": context("show_ui"), "extra": 1}));
    cases.push(json!({}));
    // A context missing a member, or with one it does not define.
    for member in [
        "profile",
        "execution_dialect",
        "registry",
        "tool",
        "show_ui",
        "references",
        "handles",
    ] {
        let mut ctx = context("show_ui");
        ctx.as_object_mut().unwrap().remove(member);
        cases.push(json!({ "context": ctx }));
    }
    let mut ctx = context("show_ui");
    ctx["extra"] = json!(1);
    cases.push(json!({ "context": ctx }));
    let mut ctx = context("show_ui");
    ctx["profile"] = json!("ShowUi");
    cases.push(json!({ "context": ctx }));
    let mut ctx = context("show_ui");
    ctx["references"] = json!({"07": {"state": "held"}});
    cases.push(json!({ "context": ctx }));
    // `now` that is not a canonical decimal string, and bytes that are not
    // canonical base64.
    for now in [json!(0), json!("1.0"), json!("-0")] {
        let mut host = default_host();
        host["now"] = now;
        cases.push(json!({"context": context("show_ui"), "host": host}));
    }
    let mut host = default_host();
    host["snapshots"] = json!({"7": "aGk"});
    cases.push(json!({"context": context("show_ui"), "host": host}));
    let mut host = default_host();
    host["data"] = json!({
        "record_id": "d",
        "record": {"state": "resolved"},
        "bytes": "aGk",
    });
    cases.push(json!({"context": context("show_ui"), "host": host}));
    let mut host = default_host();
    host["data"] = json!({"record_id": "d", "record": {"state": "resolved"}});
    cases.push(json!({"context": context("show_ui"), "host": host}));
    for ctx in cases {
        assert_eq!(
            check(CALC_PAINT.as_bytes(), &ctx.to_string()),
            (OCC_INVALID_ARGUMENT, None),
            "{ctx}"
        );
    }
    // Malformed JSON and invalid UTF-8.
    assert_eq!(check(b"", "{"), (OCC_INVALID_ARGUMENT, None));
    let bad = [b'{', 0xFF, b'}', 0];
    // SAFETY: `bad` is NUL-terminated.
    let status = call(|out| unsafe { occ_program_check(ptr::null(), 0, bad.as_ptr().cast(), out) });
    assert_eq!(status, (OCC_INVALID_ARGUMENT, None));
}

#[test]
fn null_pointers() {
    let ctx = c(&json!({"context": context("show_ui")}).to_string());
    // SAFETY: NULL is allowed for every pointer here.
    let status = call(|out| unsafe { occ_program_check(ptr::null(), 0, ptr::null(), out) });
    assert_eq!(status, (OCC_INVALID_ARGUMENT, None));
    // SAFETY: as above.
    let status = call(|out| unsafe { occ_program_check(ptr::null(), 3, ctx.as_ptr(), out) });
    assert_eq!(status, (OCC_INVALID_ARGUMENT, None));
    // SAFETY: as above.
    let status = unsafe { occ_program_check(ptr::null(), 0, ctx.as_ptr(), ptr::null_mut()) };
    assert_eq!(status, OCC_INVALID_ARGUMENT);
    // An empty source with a NULL pointer is checked.
    // SAFETY: as above.
    let (status, out) = call(|out| unsafe { occ_program_check(ptr::null(), 0, ctx.as_ptr(), out) });
    assert_eq!(status, OCC_OK);
    assert!(json(&out.unwrap())["trial"].is_object());
}
