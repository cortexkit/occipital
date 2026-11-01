use super::*;
use crate::eval::{self, Evaluation, Inputs};
use crate::prepare::{self, resolve::Resolved, types::TypeContext};
use crate::{ActionStep, Decimal, HandleIdentity, Profile, ScalarString, SiteEnvelope};
use cortexkit_rich_ui_syntax::Mode;

fn program(source: &str) -> Resolved {
    prepare::prepare(source.as_bytes(), Mode::Program, &TypeContext::default()).unwrap()
}

fn evaluate(program: &Resolved, inputs: &Inputs) -> Evaluation {
    let evaluation = eval::evaluate(program, inputs);
    assert!(evaluation.abort.is_none(), "{:?}", evaluation.diagnostics);
    evaluation
}

fn fixture(source: &str) -> (Resolved, Inputs, Runtime, Evaluation) {
    let program = program(source);
    let mut inputs = Inputs::new(Profile::ShowUi);
    inputs.state = eval::initial_state(&program);
    let runtime = Runtime::new(42, inputs.state.clone());
    let evaluation = evaluate(&program, &inputs);
    (program, inputs, runtime, evaluation)
}

fn install(runtime: &mut Runtime, revision: u64, evaluation: &Evaluation) -> Activation {
    runtime.commit(revision, evaluation.controls.clone());
    Activation {
        instance_epoch: 42,
        revision,
        control: evaluation
            .controls
            .keys()
            .next()
            .expect("enabled control")
            .clone(),
        id: 1,
    }
}

fn number(n: i128) -> Value {
    Value::number(Decimal::from_integer(n).unwrap())
}

fn string(text: &str) -> Value {
    Value::string(text).unwrap()
}

fn assert_number(runtime: &Runtime, name: &str, expected: i128) {
    assert!(
        matches!(runtime.state[name].kind(), ValueKind::Number(n) if *n == Decimal::from_integer(expected).unwrap())
    );
}

fn codes(result: &LocalResult) -> Vec<DiagnosticCode> {
    result.diagnostics.iter().map(Diagnostic::code).collect()
}

#[test]
fn evaluation_is_inert_and_every_step_uses_the_build_snapshot() {
    let (_, inputs, mut runtime, evaluation) = fixture(
        "export let $a = 7\nexport let $b = 0\n\n<Button label=\"Go\" action={Action([Set($a, 1), Set($b, $a)])} />",
    );
    assert_number(&runtime, "a", 7);
    assert_number(&runtime, "b", 0);
    assert!(matches!(inputs.state["a"].kind(), ValueKind::Number(n) if n.to_string() == "7"));
    let event = install(&mut runtime, 3, &evaluation);
    let result = runtime.activate(&event).unwrap();
    assert_number(&runtime, "a", 1);
    assert_number(&runtime, "b", 7);
    assert!(
        result.handoffs.is_empty() && result.intents.is_empty() && result.diagnostics.is_empty()
    );
}

#[test]
fn activation_selects_committed_not_newly_evaluated_actions() {
    let (program, mut inputs, mut runtime, first) =
        fixture("export let $n = 1\n\n<Button label=\"Go\" action={Set($n, $n + 1)} />");
    let mut event = install(&mut runtime, 3, &first);
    inputs.state.insert("n".into(), number(20));
    let unmeasured = evaluate(&program, &inputs);
    runtime.activate(&event).unwrap();
    assert_number(&runtime, "n", 2);
    runtime.commit(4, unmeasured.controls);
    event.revision = 4;
    event.id = 2;
    runtime.activate(&event).unwrap();
    assert_number(&runtime, "n", 21);
}

#[test]
fn stale_epoch_revision_and_missing_control_are_rejected() {
    let (_, _, mut runtime, evaluation) =
        fixture("export let $n = 1\n\n<Button label=\"Go\" action={Set($n, 2)} />");
    let event = install(&mut runtime, 3, &evaluation);
    let mut stale = event.clone();
    stale.instance_epoch = 41;
    assert_eq!(runtime.activate(&stale).unwrap_err(), Rejected::Stale);
    stale = event.clone();
    stale.revision = 2;
    assert_eq!(runtime.activate(&stale).unwrap_err(), Rejected::Stale);
    stale = event.clone();
    stale.control.0.push(PathPart::Child(999));
    assert_eq!(runtime.activate(&stale).unwrap_err(), Rejected::Disabled);
    assert_number(&runtime, "n", 1);
    runtime.commit(4, evaluation.controls);
    assert_eq!(runtime.activate(&event).unwrap_err(), Rejected::Stale);
}

#[test]
fn repeated_activation_ids_are_rejected_even_after_tree_replacement() {
    let (_, _, mut runtime, evaluation) =
        fixture("export let $n = 1\n\n<Button label=\"Go\" action={Set($n, 2)} />");
    let mut event = install(&mut runtime, 3, &evaluation);
    runtime.activate(&event).unwrap();
    assert_eq!(runtime.activate(&event).unwrap_err(), Rejected::Repeated);
    runtime.commit(4, evaluation.controls);
    event.revision = 4;
    assert_eq!(runtime.activate(&event).unwrap_err(), Rejected::Repeated);
}

#[test]
fn busy_controls_wait_for_host_handoffs_but_not_query_answers() {
    let (_, _, mut runtime, evaluation) =
        fixture("<Button label=\"Go\" action={Compose(\"hello\")} />");
    let event = install(&mut runtime, 3, &evaluation);
    let result = runtime.activate(&event).unwrap();
    assert!(
        matches!(result.handoffs.as_slice(), [Handoff::Compose(text)] if text.as_str() == "hello")
    );
    let mut next = event.clone();
    next.id = 2;
    assert_eq!(runtime.activate(&next).unwrap_err(), Rejected::Busy);
    let mut wrong = event.clone();
    wrong.revision = 2;
    runtime.handoffs_complete(&wrong);
    assert_eq!(runtime.activate(&next).unwrap_err(), Rejected::Busy);
    runtime.handoffs_complete(&event);
    assert!(runtime.activate(&next).is_ok());
    runtime.commit(4, evaluation.controls);
    next.revision = 4;
    next.id = 3;
    runtime.handoffs_complete(&event);
    assert!(runtime.activate(&next).is_ok());
}

#[test]
fn wrong_typed_set_keeps_prior_writes_and_drops_all_remaining_steps() {
    // An unknown host-data field can only be type-checked at activation.
    let program = program("export let $a = 0\nexport let $b = 7\n\n<Button label=\"Go\" action={Action([Set($a, 1), Set($b, input.bad), Set($a, 2), Compose(\"never\")])} />");
    let mut runtime = Runtime::new(42, eval::initial_state(&program));
    let mut inputs = Inputs::new(Profile::ToolPage);
    inputs.input = Value::record(vec![(ScalarString::new("bad"), string("wrong"))]).unwrap();
    let evaluation = evaluate(&program, &inputs);
    assert_eq!(evaluation.controls.len(), 1);
    let event = install(&mut runtime, 3, &evaluation);
    let result = runtime.activate(&event).unwrap();
    assert_eq!(codes(&result), [DiagnosticCode::StateType]);
    assert_number(&runtime, "a", 1);
    assert_number(&runtime, "b", 7);
    assert!(result.handoffs.is_empty() && result.intents.is_empty());
}

#[test]
fn reset_uses_declared_initial_value_and_types_do_not_follow_current_null() {
    let (_, _, mut runtime, evaluation) = fixture(
        "export let $n = 7\n\n<Button label=\"Go\" action={Action([Set($n, null), Reset($n)])} />",
    );
    let event = install(&mut runtime, 3, &evaluation);
    let result = runtime.activate(&event).unwrap();
    assert_number(&runtime, "n", 7);
    assert!(result.diagnostics.is_empty());
    runtime.assign(&string("n"), Some(&Value::null())).unwrap();
    assert_eq!(
        runtime.assign(&string("n"), Some(&string("wrong"))),
        Err(DiagnosticCode::StateType)
    );
    runtime.assign(&string("n"), Some(&number(9))).unwrap();
    assert_number(&runtime, "n", 9);
}

#[test]
fn refresh_and_load_more_are_ordered_site_names_not_old_identities() {
    let (_, _, mut runtime, evaluation) = fixture(
        "export let $n = 0\nexport const q = Query(\"q\", {n: $n}, 0)\nexport const w = Watch(\"w\", {})\nexport const p = PagedList(\"p\", {})\n\n<Button label=\"Go\" action={Action([Refresh(q), Set($n, 9), LoadMore(p), Refresh(w), Refresh(p)])} />",
    );
    let mut event = install(&mut runtime, 3, &evaluation);
    let result = runtime.activate(&event).unwrap();
    assert_eq!(
        result.intents,
        [
            SiteIntent::Refresh("q".into()),
            SiteIntent::LoadMore("p".into()),
            SiteIntent::Refresh("w".into()),
            SiteIntent::Refresh("p".into())
        ]
    );
    assert_number(&runtime, "n", 9);
    assert!(result.handoffs.is_empty() && result.diagnostics.is_empty());
    // No host handoff is outstanding; a query answer is not needed to retap.
    event.id = 2;
    assert!(runtime.activate(&event).is_ok());
}

#[test]
fn disabled_absent_and_error_actions_never_become_activatable_controls() {
    let (_, _, _, evaluation) = fixture(
        "<Button label=\"disabled\" disabled={true} action={Compose(\"no\")} />\n\n<Button label=\"absent\" action={null} />\n\n<Button label=\"error\" action={Compose(1)} />\n\n<Button label=\"missing\" />",
    );
    assert!(evaluation.controls.is_empty());
}

#[test]
fn controls_in_named_props_and_repeated_components_have_distinct_paths() {
    let (_, _, mut runtime, evaluation) = fixture(
        "export const b = <Button label=\"A\" action={Compose(\"A\")} />\n\n<Buttons buttons={[b, <Button label=\"B\" action={Compose(\"B\")} />, b]} />",
    );
    let expected: Vec<_> = (0..3)
        .map(|i| {
            ControlId(vec![
                PathPart::Child(0),
                PathPart::Prop("buttons".into()),
                PathPart::Index(i),
                PathPart::Prop("action".into()),
            ])
        })
        .collect();
    assert_eq!(
        evaluation.controls.keys().cloned().collect::<Vec<_>>(),
        expected
    );
    runtime.commit(3, evaluation.controls);
    for (i, text) in ["A", "B", "A"].into_iter().enumerate() {
        let result = runtime
            .activate(&Activation {
                instance_epoch: 42,
                revision: 3,
                control: expected[i].clone(),
                id: i as u64,
            })
            .unwrap();
        assert!(
            matches!(result.handoffs.as_slice(), [Handoff::Compose(value)] if value.as_str() == text)
        );
    }
}

#[test]
fn set_keeps_lossless_text_while_compose_handoffs_are_sanitized() {
    let (_, _, mut runtime, evaluation) = fixture(
        "export let $s = \"\"\n\n<Button label=\"Go\" action={Action([Set($s, \"a\\u{0}b\"), Compose(\"a\\u{0}b\")])} />",
    );
    let event = install(&mut runtime, 3, &evaluation);
    let result = runtime.activate(&event).unwrap();
    assert!(
        matches!(runtime.state["s"].kind(), ValueKind::String(text) if text.as_str() == "a\0b")
    );
    assert!(
        matches!(result.handoffs.as_slice(), [Handoff::Compose(text)] if text.as_str() == "a\u{fffd}b")
    );
}

#[test]
fn compose_limit_counts_scalars_not_utf8_bytes() {
    for (count, expected) in [(8192, None), (8193, Some(DiagnosticCode::Arg))] {
        let source = format!(
            "<Button label=\"Go\" action={{Compose(\"{}\")}} />",
            "😀".repeat(count)
        );
        let (_, _, mut runtime, evaluation) = fixture(&source);
        if let Some(code) = expected {
            assert!(evaluation.controls.is_empty());
            assert_eq!(
                evaluation
                    .diagnostics
                    .iter()
                    .map(Diagnostic::code)
                    .collect::<Vec<_>>(),
                [code]
            );
        } else {
            let event = install(&mut runtime, 3, &evaluation);
            let result = runtime.activate(&event).unwrap();
            assert!(
                matches!(result.handoffs.as_slice(), [Handoff::Compose(text)] if text.as_str().chars().count() == 8192)
            );
        }
    }
}

#[test]
fn open_addresses_are_canonical_sheet_handoffs_and_invalid_targets_disable() {
    let (_, _, mut runtime, evaluation) =
        fixture("<Button label=\"Go\" action={Open(\"HTTPS://EXAMPLE.COM:443/b\")} />");
    let event = install(&mut runtime, 3, &evaluation);
    let result = runtime.activate(&event).unwrap();
    assert!(
        matches!(result.handoffs.as_slice(), [Handoff::AddressSheet(address)] if address.as_str() == "https://example.com/b" && address.display().as_str() == "https://example.com/b")
    );
    for (target, code) in [
        ("\"file:///etc/passwd\"", DiagnosticCode::Scheme),
        ("1", DiagnosticCode::Type),
    ] {
        let (_, _, _, evaluation) = fixture(&format!(
            "<Button label=\"Go\" action={{Open({target})}} />"
        ));
        assert!(evaluation.controls.is_empty());
        assert_eq!(
            evaluation
                .diagnostics
                .iter()
                .map(Diagnostic::code)
                .collect::<Vec<_>>(),
            [code]
        );
    }
}

#[test]
fn offline_address_actions_draw_text_and_cannot_run_even_their_local_steps() {
    let (program, mut inputs, mut runtime, _) = fixture(
        "export let $n = 0\n\n<Button label=\"Go\" action={Action([Set($n, 1), Open(\"https://example.com\")])} />",
    );
    inputs.network = false;
    let evaluation = evaluate(&program, &inputs);
    assert!(evaluation.controls.is_empty());
    assert_eq!(
        evaluation.tree.as_ref().unwrap().blocks,
        [TreeNode::Text(DisplayText::new("https://example.com/"))]
    );
    runtime.commit(3, evaluation.controls);
    let event = Activation {
        instance_epoch: 42,
        revision: 3,
        control: ControlId(vec![PathPart::Child(0), PathPart::Prop("action".into())]),
        id: 1,
    };
    assert_eq!(runtime.activate(&event).unwrap_err(), Rejected::Disabled);
    assert_number(&runtime, "n", 0);
}

#[test]
fn offline_handle_open_keeps_its_scoped_identity_and_uses_the_viewer() {
    let program = program("<Button label=\"Go\" action={Open(input.target)} />");
    let mut inputs = Inputs::new(Profile::ToolPage);
    inputs.network = false;
    let handle = Handle {
        record_id: "record".into(),
        identity: HandleIdentity::Resolved {
            kind: "File".into(),
            scoped_resource_id: "computer\0session\0scope\0file".into(),
            version: "v1".into(),
        },
    };
    inputs.input = Value::record(vec![(
        ScalarString::new("target"),
        Value::handle(handle.clone()),
    )])
    .unwrap();
    let evaluation = evaluate(&program, &inputs);
    let mut runtime = Runtime::new(42, BTreeMap::new());
    let event = install(&mut runtime, 3, &evaluation);
    let result = runtime.activate(&event).unwrap();
    assert!(
        matches!(result.handoffs.as_slice(), [Handoff::OpenHandle(value)] if value.record_id == handle.record_id && value.identity == handle.identity)
    );
}

#[test]
fn nested_actions_enforce_the_total_sixteen_step_limit() {
    for count in [16, 17] {
        let steps = std::iter::repeat_n("Compose(\"x\")", count - 1)
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!(
            "<Button label=\"Go\" action={{Action([Action([{steps}]), Compose(\"last\")])}} />"
        );
        let (_, _, mut runtime, evaluation) = fixture(&source);
        if count == 16 {
            let event = install(&mut runtime, 3, &evaluation);
            let result = runtime.activate(&event).unwrap();
            assert_eq!(result.handoffs.len(), 16);
            assert!(
                matches!(result.handoffs.last(), Some(Handoff::Compose(text)) if text.as_str() == "last")
            );
        } else {
            assert!(evaluation.controls.is_empty());
            assert_eq!(
                evaluation
                    .diagnostics
                    .iter()
                    .map(Diagnostic::code)
                    .collect::<Vec<_>>(),
                [DiagnosticCode::LimitSteps]
            );
        }
    }
}

#[test]
fn action_values_in_text_and_data_props_report_action_placement() {
    for source in ["{Compose(\"x\")}", "<Table data={[{x: Compose(\"x\")}]} />"] {
        let (_, _, _, evaluation) = fixture(source);
        assert!(evaluation.controls.is_empty());
        assert!(evaluation
            .diagnostics
            .iter()
            .any(|d| d.code() == DiagnosticCode::ActionPlacement));
    }
}

#[test]
fn reserved_write_calls_cannot_build_any_control_in_any_profile() {
    for profile in [
        Profile::ShowUi,
        Profile::ToolPage,
        Profile::ToolRow,
        Profile::Reply,
    ] {
        for call in ["Run", "Mutation"] {
            let program = program(&format!(
                "<Button label=\"Go\" action={{{call}(\"write\", {{}})}} />"
            ));
            let evaluation = evaluate(&program, &Inputs::new(profile));
            assert!(evaluation.controls.is_empty());
            assert_eq!(
                evaluation
                    .diagnostics
                    .iter()
                    .map(Diagnostic::code)
                    .collect::<Vec<_>>(),
                [DiagnosticCode::WriteNotAllowed]
            );
        }
    }
}

#[test]
fn state_types_accept_only_declared_shapes_and_data() {
    let data = Value::record(vec![(
        ScalarString::new("a"),
        Value::list(vec![
            number(1),
            string("x"),
            Value::boolean(true),
            Value::null(),
        ])
        .unwrap(),
    )])
    .unwrap();
    assert!(accepts(ValueType::Record, &data));
    assert!(accepts(ValueType::List, &Value::list(vec![data]).unwrap()));
    for kind in [
        ValueType::Number,
        ValueType::String,
        ValueType::Boolean,
        ValueType::Null,
    ] {
        assert!(accepts(kind, &Value::null()));
        assert!(!accepts(kind, &Value::list(vec![]).unwrap()));
    }
    assert!(accepts(ValueType::Null, &string("x")));
    assert!(!accepts(ValueType::Number, &string("x")));
    assert!(!accepts(ValueType::List, &Value::null()));
    assert!(!accepts(ValueType::Record, &Value::missing()));
    let action = Value::action(vec![ActionStep {
        operation: ActionOperation::Compose,
        arguments: vec![string("x")].into(),
    }])
    .unwrap();
    assert!(!accepts(
        ValueType::List,
        &Value::list(vec![action]).unwrap()
    ));
    let site = Value::site_envelope(SiteEnvelope {
        name: "q".into(),
        members: match Value::record(vec![]).unwrap().kind() {
            ValueKind::Record(record) => record.clone(),
            _ => unreachable!(),
        },
    });
    assert!(!accepts(ValueType::List, &Value::list(vec![site]).unwrap()));
}

#[test]
fn oversized_state_assignment_fails_without_later_handoffs() {
    let (program, mut inputs, mut runtime, _) = fixture(
        "export let $s = \"\"\n\n<Button label=\"Go\" action={Action([Set($s, input.text), Compose(\"never\")])} />",
    );
    inputs.input = Value::record(vec![(
        ScalarString::new("text"),
        string(&"é".repeat(32_768)),
    )])
    .unwrap();
    let evaluation = evaluate(&program, &inputs);
    let event = install(&mut runtime, 3, &evaluation);
    let result = runtime.activate(&event).unwrap();
    assert_eq!(codes(&result), [DiagnosticCode::StateType]);
    assert!(result.handoffs.is_empty());
    assert!(
        matches!(runtime.state["s"].kind(), ValueKind::String(text) if text.as_str().is_empty())
    );
}

#[test]
fn state_limit_includes_ordered_encoding_overhead_and_accepts_exactly_64_kib() {
    let mut runtime = Runtime::new(42, BTreeMap::from([("s".into(), string(""))]));
    // One record field containing one string, including all codec tags and
    // delimiters. ASCII payload scalars each add exactly one encoded byte.
    let overhead = r#"["record",[["s",["string",""]]]]"#.len();
    let payload = "x".repeat(65_536 - overhead);
    runtime
        .assign(&string("s"), Some(&string(&payload)))
        .unwrap();
    assert!(
        matches!(runtime.state["s"].kind(), ValueKind::String(text) if text.as_str() == payload)
    );
    assert_eq!(
        runtime.assign(&string("s"), Some(&string(&(payload.clone() + "x")))),
        Err(DiagnosticCode::StateType)
    );
    assert!(
        matches!(runtime.state["s"].kind(), ValueKind::String(text) if text.as_str() == payload)
    );
}

#[test]
fn disabled_controls_still_consume_their_frozen_slot_before_nested_siblings() {
    let (_, _, mut runtime, evaluation) = fixture(
        "<Card><Buttons buttons={[<Button label=\"off\" disabled={true} action={Compose(\"never\")} />, <Button label=\"prop\" action={Compose(\"from prop\")} />]} /><Button label=\"child\" action={Compose(\"from child\")} /></Card>",
    );
    assert_eq!(evaluation.controls.len(), 2);
    runtime.commit(3, evaluation.controls.clone());
    for (id, path) in evaluation.controls.keys().enumerate() {
        let result = runtime
            .activate(&Activation {
                instance_epoch: 42,
                revision: 3,
                control: path.clone(),
                id: id as u64,
            })
            .unwrap();
        let expected = if path
            .0
            .iter()
            .any(|part| *part == PathPart::Prop("buttons".into()))
        {
            "from prop"
        } else {
            "from child"
        };
        assert!(
            matches!(result.handoffs.as_slice(), [Handoff::Compose(text)] if text.as_str() == expected)
        );
    }
}
