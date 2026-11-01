use occipital_lang::{
    CheckResult, DataPosition, Diagnostic, DiagnosticCode, DiagnosticPhase, DisplayText,
    SourceSpan, TrialOutcome, TrialResult,
};
use serde_json::json;

#[test]
fn public_diagnostics_and_output_text_are_safe_to_send_to_a_host() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Diagnostic>();
    send_sync::<CheckResult>();
    send_sync::<DisplayText>();

    let diagnostic = Diagnostic::new(DiagnosticCode::UnknownComponent, DiagnosticPhase::Check)
        .with_subject("Gauge\u{1B}")
        .with_span(SourceSpan {
            line: 1,
            column: 2,
            end_line: 1,
            end_column: 7,
        })
        .unwrap();
    let result = CheckResult::new(
        vec![diagnostic],
        TrialResult {
            fuel: 0,
            nodes: 0,
            outcome: TrialOutcome::Ok,
        },
    );
    let encoded = serde_json::to_value(result).unwrap();
    assert_eq!(encoded["ok"], false);
    assert_eq!(encoded["diagnostics"][0]["subject"], "Gauge�");
    assert_eq!(encoded["diagnostics"][0]["code"], "E_UNKNOWN_COMPONENT");

    // Both byte-source fallbacks and evaluated strings enter the same safe type.
    let from_source = DisplayText::from_bytes(b"<Gauge>\x1b</Gauge>");
    let from_value = DisplayText::new("<Gauge>\u{1B}</Gauge>");
    assert_eq!(from_source, from_value);
    assert_eq!(
        serde_json::to_value(from_value).unwrap(),
        json!("<Gauge>�</Gauge>")
    );
}

#[test]
fn data_and_source_diagnostics_share_position_order_and_error_tie_breaking() {
    let data = Diagnostic::new(DiagnosticCode::CsvSyntax, DiagnosticPhase::Data)
        .with_span(DataPosition::new(3, 4).unwrap().with_record(2).unwrap())
        .unwrap();
    let source = Diagnostic::new(DiagnosticCode::Unused, DiagnosticPhase::Check)
        .with_span(SourceSpan {
            line: 3,
            column: 4,
            end_line: 3,
            end_column: 5,
        })
        .unwrap();
    let result = CheckResult::new(
        vec![source, data],
        TrialResult {
            fuel: 1,
            nodes: 0,
            outcome: TrialOutcome::Ok,
        },
    );
    assert_eq!(result.diagnostics()[0].code(), DiagnosticCode::CsvSyntax);
    assert_eq!(result.diagnostics()[1].code(), DiagnosticCode::Unused);
    let encoded = serde_json::to_value(result).unwrap();
    assert_eq!(
        encoded["diagnostics"][0]["span"],
        json!({
            "source": "data", "line": 3, "column": 4, "record": 2
        })
    );
}
