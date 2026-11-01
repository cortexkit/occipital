use super::*;
use crate::{DiagnosticCode as Code, Operation, Schema, Snapshot, Value, ValueKind};
use serde_json::json;

fn run(source: &str) -> CheckResult {
    check_program(
        source.as_bytes(),
        &CheckContext::new(Profile::ShowUi),
        &Inputs::new(Profile::ShowUi),
    )
    .unwrap()
}

fn codes(result: &CheckResult) -> Vec<Code> {
    result.diagnostics().iter().map(Diagnostic::code).collect()
}

fn snapshot(value: Value) -> Value {
    let ValueKind::Record(members) =
        Value::record(vec![(crate::ScalarString::new("value"), value)])
            .unwrap()
            .kind()
            .clone()
    else {
        unreachable!()
    };
    Value::snapshot(Snapshot {
        record_id: "snapshot".into(),
        members,
    })
}

fn tables(count: usize) -> String {
    let columns = (0..8)
        .map(|i| format!("\"c{i}\""))
        .collect::<Vec<_>>()
        .join(",");
    let record = format!(
        "{{{}}}",
        (0..8)
            .map(|i| format!("c{i}: {i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let records = std::iter::repeat_n(record, 50)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "export const rows = [{records}]\n\n{}",
        std::iter::repeat_n(
            format!("<Table columns={{[{columns}]}} data={{rows}} displayRows={{50}} />"),
            count
        )
        .collect::<Vec<_>>()
        .join("\n\n")
    )
}

fn doubling(name: &str, initial: &str, expression: &str, n: usize) -> String {
    let mut source = format!("export const {name}0 = {initial}\n");
    for i in 1..=n {
        source.push_str(&format!(
            "export const {name}{i} = {}\n",
            expression.replace('P', &format!("{name}{}", i - 1))
        ));
    }
    source.push_str(&format!("\n{{{name}{n}}}"));
    source
}

#[test]
fn load_and_five_second_timeout_sum_fuel_but_not_output_nodes() {
    let result = serde_json::to_value(run("{1}")).unwrap();
    // Each pass visits the literal (1), inserts its text (1 + ceil(1/8)),
    // and adds the paragraph (1): 4 fuel, two nodes. The trial runs twice.
    assert_eq!(
        result,
        json!({
            "ok": true, "language": "1.0", "executionDialect": "1.0",
            "approvalChecked": false, "requires": "1.0", "diagnostics": [], "omitted": 0,
            "trial": {"fuel": 8, "nodes": 2, "outcome": "ok"}
        })
    );
}

#[test]
fn static_errors_and_non_show_ui_profiles_report_not_run() {
    let result = run("{unknown}");
    assert!(!result.ok());
    assert_eq!(codes(&result), [Code::UnknownName]);
    assert_eq!(
        serde_json::to_value(result.trial()).unwrap(),
        json!({"fuel": 0, "nodes": 0, "outcome": "not_run"})
    );
    let result = check_program(
        b"{1 / 0}",
        &CheckContext::new(Profile::ToolPage),
        &Inputs::new(Profile::ToolPage),
    )
    .unwrap();
    assert_eq!(result.trial().outcome, TrialOutcome::NotRun);
    assert_eq!(result.trial().fuel, 0);
    assert!(result.diagnostics().is_empty());
    // §12.2: no error means ok, and a tool profile has no trial to pass.
    assert!(result.ok());
}

#[test]
fn trial_reads_snapshot_initial_state_and_loading_site_defaults_without_answers() {
    let mut context = CheckContext::new(Profile::ShowUi);
    let schema = |v| serde_json::from_value::<Schema>(v).unwrap();
    context.registry.0.insert(
        "read".into(),
        Operation::Query {
            arguments: schema(json!({"type":"object"})),
            result: schema(
                json!({"type":"object", "properties":{"n":{"type":"number"}}, "required":["n"]}),
            ),
        },
    );
    let mut inputs = Inputs::new(Profile::ShowUi);
    inputs.data = snapshot(Value::number(crate::Decimal::from_integer(9).unwrap()));
    inputs.state.insert(
        "n".into(),
        Value::number(crate::Decimal::from_integer(1).unwrap()),
    );
    inputs.sites.insert("q".into(), Value::null());
    let source = br#"export let $n = 0
export const q = Query("read", {}, {n: 7})

{q.status == "loading" && q.value.n == 7 && $n == 0 && data.value == 9 ? 1 / 0 : 1}"#;
    let result = check_program(source, &context, &inputs).unwrap();
    assert_eq!(codes(&result), [Code::DivZero]);
    assert_eq!(result.diagnostics()[0].phase(), DiagnosticPhase::Evaluate);
    assert!(!result.ok());
    assert_eq!(result.trial().outcome, TrialOutcome::Ok);
    assert!(result.trial().fuel > 0);
}

#[test]
fn two_hundred_fifty_tables_fail_the_page_reservation_not_the_node_limit() {
    let result = run(&tables(250));
    // 2 + 8 * (50 + 1) = 410 per Table, hence 102,500 visits. Row
    // placement would cost only 22,500, so accepting here uses the wrong bound.
    assert_eq!(result.trial().outcome, TrialOutcome::LimitLayout);
    assert_eq!(result.trial().nodes, 250);
    assert!(result.trial().fuel > 0);
    assert!(!result.ok());
    assert_eq!(codes(&result), [Code::LimitLayout]);
    assert_eq!(
        serde_json::to_value(&result.diagnostics()[0]).unwrap(),
        json!({
            "code": "E_LIMIT_LAYOUT", "severity": "error", "phase": "evaluate",
            "message": "Layout work exceeds its page-visit reservation."
        })
    );
}

#[test]
fn page_reservation_admits_just_under_and_at_the_limit_but_refuses_one_more_visit() {
    let mut source = tables(243);
    // 243 * 410 = 99,630, and one 45-row Table adds 2 + 8 * 46 = 370.
    assert!(run(&source).ok());
    source.push_str("\n\n<Table columns={[\"c0\",\"c1\",\"c2\",\"c3\",\"c4\",\"c5\",\"c6\",\"c7\"]} data={rows} displayRows={45} />");
    assert!(run(&source).ok());
    source.push_str("\n\n<Divider />");
    let result = run(&source);
    assert_eq!(codes(&result), [Code::LimitLayout]);
    assert_eq!(result.trial().outcome, TrialOutcome::LimitLayout);
}

#[test]
fn trial_does_not_reserve_decoded_bytes_or_require_layout_metrics() {
    // Image frames have a decoded-byte tariff, but no width or scale exists
    // in the checker. Only their 200 defined visits are reserved here.
    let source = std::iter::repeat_n("<Image image={data} />", 100)
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut inputs = Inputs::new(Profile::ShowUi);
    inputs.data = snapshot(Value::null());
    let result = check_program(
        source.as_bytes(),
        &CheckContext::new(Profile::ShowUi),
        &inputs,
    )
    .unwrap();
    assert!(result.ok(), "{:?}", result.diagnostics());
    assert_eq!(result.trial().outcome, TrialOutcome::Ok);
    assert_eq!(result.trial().nodes, 100);
}

#[test]
fn trial_abort_returns_its_diagnostic_fuel_and_partial_nodes() {
    let result = run(&doubling("s", r#""abcdefgh""#, "P + P", 14));
    // F(0)=2; F(k)=F(k-1)+3+2^k. The aborting s14 costs
    // F(13)+3=16,426, before creating or charging its oversized string.
    assert_eq!(
        result.trial(),
        TrialResult {
            fuel: 16_426,
            nodes: 0,
            outcome: TrialOutcome::LimitString
        }
    );
    assert_eq!(codes(&result), [Code::LimitString]);
    assert!(!result.ok());
    let result = run(&doubling(
        "t",
        r#"<Badge tone="info">x</Badge>"#,
        "[P, P]",
        10,
    ));
    assert_eq!(result.trial().outcome, TrialOutcome::LimitNodes);
    assert_eq!(result.trial().nodes, 2_000);
    assert!(result.trial().fuel > 0);
    assert_eq!(codes(&result), [Code::LimitNodes]);
}

#[test]
fn repeated_trial_errors_are_reported_once_and_omissions_are_kept() {
    let result = run(&std::iter::repeat_n("{1 / 0}", 52)
        .collect::<Vec<_>>()
        .join("\n\n"));
    assert!(!result.ok());
    assert_eq!(result.trial().outcome, TrialOutcome::Ok);
    assert_eq!(result.diagnostics().len(), 50);
    assert_eq!(result.omitted(), 2);
    assert!(result
        .diagnostics()
        .iter()
        .all(|d| d.code() == Code::DivZero));
}

#[test]
fn fractional_display_rows_is_a_prop_error_before_work_is_counted() {
    let context = CheckContext::new(Profile::ShowUi);
    let source = b"<Table data={[]} displayRows={data.value} />";
    let program = crate::check::check(source, &context).unwrap();
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    let mut inputs = Inputs::new(Profile::ShowUi);
    inputs.data = snapshot(Value::number(crate::Decimal::from_literal("1.5").unwrap()));
    let result = check_program(source, &context, &inputs).unwrap();
    // Fractional values are rejected as prop errors and normalized to an error
    // cell (§5.6), not handed to geometry as a malformed component.
    assert_eq!(codes(&result), [Code::PropType]);
    assert_eq!(result.diagnostics()[0].phase(), DiagnosticPhase::Evaluate);
    assert_eq!(result.trial().outcome, TrialOutcome::Ok);
    assert!(!result.ok());
    let evaluation = evaluate_trial(&program, &inputs).unwrap();
    assert!(matches!(
        evaluation.tree.unwrap().blocks.as_slice(),
        [crate::eval::tree::TreeNode::ErrorCell {
            code: Code::PropType,
            ..
        }]
    ));

    let literal = run("<Table data={[]} displayRows={1.5} />");
    assert_eq!(codes(&literal), [Code::PropType]);
    assert_eq!(literal.diagnostics()[0].phase(), DiagnosticPhase::Check);
    assert_eq!(literal.trial().outcome, TrialOutcome::NotRun);
}

#[test]
fn generic_tool_row_is_rejected_before_a_show_ui_trial() {
    let result = run("<GenericToolRow />");
    assert_eq!(codes(&result), [Code::UnknownComponent]);
    assert_eq!(result.diagnostics()[0].phase(), DiagnosticPhase::Check);
    assert_eq!(
        result.trial(),
        TrialResult {
            fuel: 0,
            nodes: 0,
            outcome: TrialOutcome::NotRun
        }
    );
}

#[test]
fn reservation_over_limit_and_numeric_overflow_are_layout_aborts() {
    assert_eq!(reserve_visits(Ok(100_000)), Ok(None));
    // Overflow is injected at the counter boundary: checked trees have only
    // 2,000 nodes and bounded per-component work, so they cannot sum to u64::MAX.
    for work in [
        Ok(100_001),
        Err(MeasureError::Number(NumberError::Overflow)),
    ] {
        let abort = reserve_visits(work).unwrap().unwrap();
        assert_eq!(abort.code(), Code::LimitLayout);
        assert_eq!(abort.outcome(), TrialOutcome::LimitLayout);
    }
}

#[test]
fn reservation_non_limit_errors_return_typed_internal_faults() {
    for error in [
        MeasureError::InvalidTree("displayRows".into()),
        MeasureError::GenericToolRow,
        MeasureError::Number(NumberError::DivZero),
        MeasureError::Number(NumberError::Domain),
        MeasureError::Number(NumberError::Arg),
        MeasureError::Number(NumberError::NumberParse),
        MeasureError::Number(NumberError::NumberLiteral),
    ] {
        assert_eq!(
            reserve_visits(Err(error.clone())),
            Err(TrialFault::Reservation(error))
        );
    }
}
