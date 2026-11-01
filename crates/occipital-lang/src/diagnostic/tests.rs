use serde_json::{json, Value as Json};

use super::*;

fn span(line: u32, column: u32) -> SourceSpan {
    SourceSpan {
        line,
        column,
        end_line: line,
        end_column: column + 1,
    }
}

fn trial() -> TrialResult {
    TrialResult {
        fuel: 18342,
        nodes: 41,
        outcome: TrialOutcome::Ok,
    }
}

fn diagnostic(code: DiagnosticCode, line: u32, column: u32, name: &str) -> Diagnostic {
    Diagnostic::new(code, DiagnosticPhase::Check)
        .with_span(span(line, column))
        .unwrap()
        .with_subject(name)
}

#[test]
fn diagnostic_matches_the_exact_section_12_1_example() {
    let diagnostic = Diagnostic::new(DiagnosticCode::Arity, DiagnosticPhase::Check)
        .with_span(SourceSpan {
            line: 14,
            column: 31,
            end_line: 14,
            end_column: 52,
        })
        .unwrap()
        .with_subject("Round")
        .with_message("Round takes 1 or 2 arguments; this call has 3.")
        .unwrap()
        .with_signature("Round(x: number, digits?: number) -> number")
        .with_hint("Remove the third argument.");
    assert_eq!(
        serde_json::to_value(diagnostic).unwrap(),
        json!({
            "code": "E_ARITY", "severity": "error", "phase": "check",
            "span": {"line": 14, "column": 31, "endLine": 14, "endColumn": 52},
            "subject": "Round", "message": "Round takes 1 or 2 arguments; this call has 3.",
            "signature": "Round(x: number, digits?: number) -> number",
            "hint": "Remove the third argument."
        })
    );
}

#[test]
fn optional_fields_are_absent_not_null_and_all_phases_have_stable_spellings() {
    for (phase, spelling) in [
        (DiagnosticPhase::Parse, "parse"),
        (DiagnosticPhase::Check, "check"),
        (DiagnosticPhase::Data, "data"),
        (DiagnosticPhase::Evaluate, "evaluate"),
    ] {
        let diagnostic = Diagnostic::new(DiagnosticCode::Encoding, phase);
        assert_eq!(diagnostic.phase(), phase);
        assert_eq!(diagnostic.span(), None);
        assert_eq!(
            serde_json::to_value(diagnostic).unwrap(),
            json!({"code": "W_ENCODING", "severity": "warning", "phase": spelling,
                "message": "Invalid UTF-8 in the source was replaced."})
        );
    }
}

#[test]
fn diagnostic_strings_use_the_shared_control_sanitizer() {
    let diagnostic = Diagnostic::new(DiagnosticCode::Cycle, DiagnosticPhase::Evaluate)
        .with_subject("value\u{1B}\u{85}")
        .with_message("A cycle\rcontains a control\u{7F}, a tab\tand LF\n.")
        .unwrap()
        .with_signature("x\0: string")
        .with_hint("Remove\u{9F}the cycle.")
        .with_path(vec![3, 0])
        .unwrap()
        .with_cycle(&["a\u{1B}", "b\r", "a\u{1B}"])
        .unwrap();
    let encoded = serde_json::to_value(diagnostic).unwrap();
    assert_eq!(encoded["subject"], "value��");
    assert_eq!(
        encoded["message"],
        "A cycle�contains a control�, a tab\tand LF\n."
    );
    assert_eq!(encoded["signature"], "x�: string");
    assert_eq!(encoded["hint"], "Remove�the cycle.");
    assert_eq!(encoded["path"], json!([3, 0]));
    assert_eq!(encoded["cycle"], json!(["a�", "b�", "a�"]));
}

#[test]
fn messages_are_bounded_in_scalars_not_bytes_and_reject_301() {
    const HUNDRED: &str = "éééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééé";
    // Static test wording exercises the API without admitting runtime data.
    const THREE_HUNDRED: &str = concat!(
        "éééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééé",
        "éééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééé",
        "éééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééééé"
    );
    assert_eq!(HUNDRED.chars().count(), 100);
    assert_eq!(THREE_HUNDRED.chars().count(), 300);
    let diagnostic = Diagnostic::new(DiagnosticCode::Type, DiagnosticPhase::Evaluate)
        .with_message(THREE_HUNDRED)
        .unwrap();
    assert_eq!(diagnostic.message().chars().count(), 300);
    let too_long = concat!(
        "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789",
        "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789",
        "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789."
    );
    assert_eq!(
        Diagnostic::new(DiagnosticCode::Type, DiagnosticPhase::Evaluate).with_message(too_long),
        Err(DiagnosticBuildError::MessageTooLong)
    );
}

#[test]
fn data_positions_are_tagged_and_do_not_expose_snapshot_contents() {
    let diagnostic = Diagnostic::new(DiagnosticCode::CsvSyntax, DiagnosticPhase::Data)
        .with_span(DataPosition::new(4, 8).unwrap().with_record(3).unwrap())
        .unwrap();
    assert_eq!(
        serde_json::to_value(diagnostic).unwrap(),
        json!({"code": "E_CSV_SYNTAX", "severity": "error", "phase": "data",
            "span": {"source": "data", "line": 4, "column": 8, "record": 3},
            "message": "Data does not match the CSV grammar."})
    );
    let json_diagnostic = Diagnostic::new(DiagnosticCode::JsonSyntax, DiagnosticPhase::Data)
        .with_span(DataPosition::new(1, 2).unwrap())
        .unwrap();
    assert_eq!(
        serde_json::to_value(json_diagnostic).unwrap()["span"],
        json!({"source": "data", "line": 1, "column": 2})
    );
}

#[test]
fn invalid_positions_and_inapplicable_metadata_are_rejected() {
    for source in [
        span(0, 1),
        span(1, 0),
        SourceSpan {
            end_column: 0,
            ..span(1, 1)
        },
        SourceSpan {
            end_line: 0,
            ..span(1, 1)
        },
        SourceSpan {
            line: 2,
            ..span(1, 1)
        },
        SourceSpan {
            column: 3,
            ..span(1, 1)
        },
    ] {
        assert_eq!(
            Diagnostic::new(DiagnosticCode::Syntax, DiagnosticPhase::Parse).with_span(source),
            Err(DiagnosticBuildError::InvalidPosition)
        );
    }
    // Empty ranges and multiline exclusive ends are valid source positions.
    for source in [
        SourceSpan {
            end_column: 1,
            ..span(1, 1)
        },
        SourceSpan {
            end_line: 2,
            end_column: 1,
            ..span(1, 5)
        },
    ] {
        assert!(
            Diagnostic::new(DiagnosticCode::Syntax, DiagnosticPhase::Parse)
                .with_span(source)
                .is_ok()
        );
    }
    assert_eq!(
        DataPosition::new(0, 1),
        Err(DiagnosticBuildError::InvalidPosition)
    );
    assert_eq!(
        DataPosition::new(1, 0),
        Err(DiagnosticBuildError::InvalidPosition)
    );
    assert_eq!(
        DataPosition::new(1, 1).unwrap().with_record(0),
        Err(DiagnosticBuildError::InvalidPosition)
    );
    assert_eq!(
        Diagnostic::new(DiagnosticCode::Type, DiagnosticPhase::Check).with_path(vec![0]),
        Err(DiagnosticBuildError::PathOutsideEvaluation)
    );
    assert_eq!(
        Diagnostic::new(DiagnosticCode::Type, DiagnosticPhase::Evaluate).with_cycle(&["a"]),
        Err(DiagnosticBuildError::CycleForOtherCode)
    );
}

#[test]
fn check_result_has_the_section_12_2_shape_and_warnings_do_not_block() {
    assert_eq!(
        serde_json::to_value(CheckResult::new(vec![], trial())).unwrap(),
        json!({
            "ok": true, "language": "1.0", "executionDialect": "1.0",
            "approvalChecked": false, "requires": "1.0", "diagnostics": [], "omitted": 0,
            "trial": {"fuel": 18342, "nodes": 41, "outcome": "ok"}
        })
    );
    assert!(CheckResult::new(
        vec![Diagnostic::new(
            DiagnosticCode::Unused,
            DiagnosticPhase::Check
        )],
        trial()
    )
    .ok());
    assert!(!CheckResult::new(
        vec![Diagnostic::new(
            DiagnosticCode::Syntax,
            DiagnosticPhase::Parse
        )],
        trial()
    )
    .ok());
}

#[test]
fn check_result_sorts_by_start_position_then_severity_and_preserves_ties() {
    let input = vec![
        Diagnostic::new(DiagnosticCode::Encoding, DiagnosticPhase::Parse)
            .with_subject("no position warning"),
        diagnostic(DiagnosticCode::Syntax, 3, 1, "later"),
        diagnostic(DiagnosticCode::Unused, 2, 3, "warning"),
        diagnostic(DiagnosticCode::Arity, 2, 3, "first error")
            .with_span(SourceSpan {
                end_column: 100,
                ..span(2, 3)
            })
            .unwrap(),
        diagnostic(DiagnosticCode::ArgType, 2, 3, "second error"),
        diagnostic(DiagnosticCode::Syntax, 2, 2, "earlier column"),
        diagnostic(DiagnosticCode::Syntax, 1, 20, "earlier line"),
        Diagnostic::new(DiagnosticCode::Syntax, DiagnosticPhase::Parse)
            .with_subject("no position error"),
    ];
    let result = serde_json::to_value(CheckResult::new(input, trial())).unwrap();
    let subjects: Vec<_> = result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["subject"].as_str().unwrap())
        .collect();
    assert_eq!(
        subjects,
        [
            "earlier line",
            "earlier column",
            "first error",
            "second error",
            "warning",
            "later",
            "no position error",
            "no position warning"
        ]
    );
}

#[test]
fn diagnostics_are_sorted_before_the_cap_and_omitted_errors_still_block() {
    for count in [49, 50, 51, 60] {
        let input = (1..=count)
            .rev()
            .map(|line| diagnostic(DiagnosticCode::Unused, line, 1, "unused"))
            .collect();
        let result = CheckResult::new(input, trial());
        assert!(result.ok());
        assert_eq!(result.diagnostics().len(), (count as usize).min(50));
        assert_eq!(result.omitted(), (count as usize).saturating_sub(50));
        for (index, diagnostic) in result.diagnostics().iter().enumerate() {
            assert_eq!(diagnostic.span(), Some(span(index as u32 + 1, 1).into()));
        }
    }
    let mut input: Vec<_> = (1..=50)
        .map(|line| diagnostic(DiagnosticCode::Unused, line, 1, "unused"))
        .collect();
    input.push(diagnostic(DiagnosticCode::Syntax, 100, 1, "omitted error"));
    let result = CheckResult::new(input, trial());
    assert!(!result.ok());
    assert_eq!(result.omitted(), 1);
    assert_eq!(result.diagnostics().len(), 50);
    assert!(result
        .diagnostics()
        .iter()
        .all(|d| d.severity() == Severity::Warning));
}

#[test]
fn all_trial_abort_codes_have_the_exact_wire_spelling() {
    for (outcome, code) in [
        (TrialOutcome::Fuel, "E_FUEL"),
        (TrialOutcome::LimitString, "E_LIMIT_STRING"),
        (TrialOutcome::LimitList, "E_LIMIT_LIST"),
        (TrialOutcome::LimitRecord, "E_LIMIT_RECORD"),
        (TrialOutcome::LimitDepth, "E_LIMIT_DEPTH"),
        (TrialOutcome::LimitValue, "E_LIMIT_VALUE"),
        (TrialOutcome::LimitAlloc, "E_LIMIT_ALLOC"),
        (TrialOutcome::LimitNodes, "E_LIMIT_NODES"),
        (TrialOutcome::LimitText, "E_LIMIT_TEXT"),
        (TrialOutcome::LimitLayout, "E_LIMIT_LAYOUT"),
    ] {
        let trial = TrialResult { outcome, ..trial() };
        let result = CheckResult::new(vec![], trial);
        assert!(!result.ok());
        assert_eq!(result.trial(), trial);
        assert_eq!(serde_json::to_value(trial).unwrap()["outcome"], code);
        assert_eq!(
            serde_json::from_value::<TrialResult>(json!({"fuel":18342,"nodes":41,"outcome":code}))
                .unwrap(),
            trial
        );
    }
    assert!(serde_json::from_str::<TrialOutcome>("\"E_TYPE\"").is_err());
}

#[test]
fn every_section_12_4_code_produces_a_safe_diagnostic_and_round_trips() {
    // Independent inventory from §12.4, including both codes in combined rows,
    // plus E_UNSLOTTED_READ, a tool row reading `view` or `now` outside a Slot.
    let expected = "E_SOURCE_TOO_LARGE E_SYNTAX E_UNTERMINATED_STRING E_BAD_ESCAPE E_NUMBER_LITERAL E_UNCLOSED_TAG E_MISMATCHED_CLOSE E_DUPLICATE_PROP E_DUPLICATE_KEY E_NONASSOC E_MIXED_COALESCE E_DECLARATION_INCOMPLETE E_NESTING E_LIMIT_DECLARATIONS
E_UNKNOWN_NAME E_UNKNOWN_STATE E_DUPLICATE_NAME E_RESERVED_NAME E_HOST_VALUE_UNAVAILABLE E_REFERENCE_UNAVAILABLE E_REFERENCE_LIMIT E_CYCLE E_UNKNOWN_FUNCTION E_ARITY E_ARG_TYPE E_LITERAL_REQUIRED E_EACH_BINDER E_EACH_NESTING E_UNKNOWN_COMPONENT E_UNKNOWN_PROP E_MISSING_PROP E_ENUM E_PROP_TYPE E_PLACEMENT E_UNSLOTTED_READ E_BINDING E_ACTION_REQUIRED E_ACTION_PLACEMENT E_LIMIT_STEPS E_SITE_PLACEMENT E_SITE_LIMIT E_SITE_ARGS E_ENVELOPE_MEMBER E_REFRESH_RANGE E_UNKNOWN_OPERATION E_OPERATION_KIND E_WRITE_NOT_ALLOWED E_REPLY_FORBIDDEN E_LIMIT_TAGS E_UNSUPPORTED_VERSION W_UNUSED W_ENCODING
E_TYPE E_ARG E_DIV_ZERO E_OVERFLOW E_DOMAIN E_NUMBER_PARSE E_DATE_PARSE E_SCHEME E_STATE_TYPE
E_FUEL E_LIMIT_STRING E_LIMIT_LIST E_LIMIT_RECORD E_LIMIT_DEPTH E_LIMIT_VALUE E_LIMIT_ALLOC E_LIMIT_NODES E_LIMIT_TEXT E_LIMIT_LAYOUT
E_DATA_ENCODING E_JSON_SYNTAX E_JSON_DUPLICATE_KEY E_CSV_SYNTAX E_DATA_LIMIT W_NUMBER_ROUNDED W_NUMBER_RANGE W_JSON_SURROGATE W_CELL_TYPE W_CSV_HEADER_RENAMED W_CSV_SHORT_ROW W_CSV_LONG_ROW W_CSV_UNKNOWN_COLUMN E_RENDER_LIMIT";
    let actual: Vec<_> = DiagnosticCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect();
    assert_eq!(actual, expected.split_whitespace().collect::<Vec<_>>());
    for code in DiagnosticCode::ALL {
        let diagnostic = Diagnostic::new(*code, DiagnosticPhase::Check);
        assert_eq!(diagnostic.code(), *code);
        assert!(!diagnostic.message().is_empty());
        assert!(diagnostic.message().chars().count() <= 300);
        assert_eq!(
            diagnostic.severity(),
            if code.as_str().starts_with("W_") {
                Severity::Warning
            } else {
                Severity::Error
            }
        );
        let serialized = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(serialized["code"], code.as_str());
        assert_eq!(
            serde_json::from_value::<DiagnosticCode>(serialized["code"].clone()).unwrap(),
            *code
        );
        assert_eq!(serialized.as_object().unwrap().len(), 4);
    }
    assert!(
        serde_json::from_value::<DiagnosticCode>(Json::String("E_NEW_UNKNOWN_CODE".into()))
            .is_err()
    );
}
