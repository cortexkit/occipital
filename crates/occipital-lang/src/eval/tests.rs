//! Expected values, codes and fuel here are derived by hand from the language
//! specification's tables (§4.2, §5, §9.2–§9.5), never recorded from output.

use cortexkit_rich_ui_syntax::{Kind, Mode, Node};

use super::snapshot::{parse_keys, snapshot, SnapshotSource};
use super::tree::{MarkdownKind, TreeNode, TreeStep, TreeValue};
use super::*;
use crate::data::{self, Format, Options};
use crate::prepare::types::TypeContext;
use crate::{ActionOperation, DisplayText, ReferenceRecord, ReferenceState};

fn program(source: &str) -> Resolved {
    crate::prepare::prepare(source.as_bytes(), Mode::Program, &TypeContext::default()).unwrap()
}

fn run(source: &str) -> Evaluation {
    run_with(source, &Inputs::new(Profile::ShowUi))
}

fn run_with(source: &str, inputs: &Inputs) -> Evaluation {
    evaluate(&program(source), inputs)
}

fn first_interpolation(nodes: &[Node]) -> Option<&Node> {
    nodes.iter().find_map(|node| match node.kind {
        Kind::Interpolation { .. } => Some(node),
        _ => first_interpolation(&node.children),
    })
}

/// Evaluate the expression of `{source}` alone, returning its value and the
/// fuel it was charged.
fn expression(source: &str) -> (Value, u64) {
    let program = program(&format!("{{{source}}}"));
    let inputs = Inputs::new(Profile::ShowUi);
    let mut evaluator = Evaluator::new(&program, &inputs);
    let node = first_interpolation(&program.parsed.document.nodes)
        .unwrap_or_else(|| panic!("{source} must parse as one interpolation"));
    let value = evaluator
        .expr(&node.children[0])
        .unwrap_or_else(|abort| panic!("{source} aborted: {abort:?}"));
    (value, evaluator.meter.fuel)
}

fn value(source: &str) -> Value {
    expression(source).0
}

fn shown(value: &Value) -> String {
    match value.kind() {
        ValueKind::Number(number) => number.to_string(),
        ValueKind::String(text) => format!("{:?}", text.as_str()),
        ValueKind::Boolean(value) => value.to_string(),
        ValueKind::Null => "null".into(),
        ValueKind::Missing => "missing".into(),
        ValueKind::Error(error) => error.code.to_string(),
        ValueKind::List(items) => format!(
            "[{}]",
            items.iter().map(shown).collect::<Vec<_>>().join(", ")
        ),
        ValueKind::Record(record) => format!(
            "{{{}}}",
            record
                .fields()
                .iter()
                .map(|(key, value)| format!("{}: {}", key.as_str(), shown(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => format!("{other:?}"),
    }
}

fn check(source: &str, expected: &str) {
    assert_eq!(shown(&value(source)), expected, "{source}");
}

fn text(node: &TreeNode) -> &str {
    match node {
        TreeNode::Text(text) => text.as_str(),
        other => panic!("expected text, found {other:?}"),
    }
}

fn paragraph(node: &TreeNode) -> &[TreeNode] {
    match node {
        TreeNode::Markdown {
            kind: MarkdownKind::Paragraph,
            children,
        } => children,
        other => panic!("expected a paragraph, found {other:?}"),
    }
}

fn blocks(evaluation: &Evaluation) -> &[TreeNode] {
    &evaluation
        .tree
        .as_ref()
        .unwrap_or_else(|| panic!("aborted: {:?}", evaluation.abort))
        .blocks
}

fn codes(evaluation: &Evaluation) -> Vec<DiagnosticCode> {
    evaluation
        .diagnostics
        .iter()
        .map(Diagnostic::code)
        .collect()
}

/// Declarations `name0 = first` and `name{k} = name{k-1} + name{k-1}` (or
/// a list of the two) up to `last`, then a blank line and `body`.
fn doubling(name: &str, first: &str, step: &str, last: u32, body: &str) -> String {
    let mut source = format!("export const {name}0 = {first}\n");
    for k in 1..=last {
        let previous = format!("{name}{}", k - 1);
        let expression = step.replace("P", &previous);
        source.push_str(&format!("export const {name}{k} = {expression}\n"));
    }
    source.push('\n');
    source.push_str(body);
    source
}

// §5.1 operators and the §4.2 worked values.

#[test]
fn power_binds_tighter_than_unary_minus() {
    // -(2 ** 2): unary 1 + power 1 + two literals 2 + the power's three
    // multiplications (2 = 10b: square, multiply; square) = 7.
    let (result, fuel) = expression("-2 ** 2");
    assert_eq!(shown(&result), "-4");
    assert_eq!(fuel, 7);
    check("(-2) ** 2", "4");
    check("-2 ** 2 == -4", "true");
    check("2 ** 3 ** 2", "512");
}

#[test]
fn decimal_worked_values_hold_as_expressions() {
    // ==, + and three literals: 5. Number equality charges nothing more.
    let (result, fuel) = expression("0.1 + 0.2 == 0.3");
    assert_eq!(shown(&result), "true");
    assert_eq!(fuel, 5);
    check("-7 % 3", "-1");
    check("7 % -3", "1");
    check("0.1 ** -19 == 10000000000000000000", "true");
    check("Round(2.675, 2)", "2.68");
    check("1 / 0", "E_DIV_ZERO");
    check("1 % 0", "E_DIV_ZERO");
    check("2 ** 0.5", "E_DOMAIN");
    check("2 ** 1025", "E_ARG");
    check("99999999999999999999 + 1", "E_OVERFLOW");
    check("1 / 3 * 3", "0.999999999999999999");
}

#[test]
fn arithmetic_on_absent_values_is_a_type_error() {
    for source in [
        "null + 1",
        "1 + null",
        "null - 1",
        "null * 2",
        "2 / null",
        "null % 2",
        "null ** 2",
        "-null",
        "{}.x + 1",
        "{}.x * 1.9",
    ] {
        check(source, "E_TYPE");
    }
    check("(null ?? 0) * 1.9", "0");
}

#[test]
fn plus_concatenates_when_either_side_is_a_string() {
    check(r#""a" + 1"#, r#""a1""#);
    check(r#"1.50 + "x""#, r#""1.5x""#);
    check(r#"true + """#, r#""true""#);
    check(r#"null + "a""#, r#""a""#);
    check(r#""a" + {}.x"#, r#""a""#);
    check(r#""a" + [1]"#, "E_TYPE");
    check(r#"{} + "a""#, "E_TYPE");
    check("true + 1", "E_TYPE");
    // Concatenation: + 1, two literals 2, and ⌈5/8⌉ = 1 for the result.
    assert_eq!(expression(r#""ab" + "cde""#).1, 4);
}

#[test]
fn conditions_accept_booleans_and_absent_values_only() {
    check("!null", "true");
    check("!true", "false");
    check("!0", "E_TYPE");
    check(r#"!"""#, "E_TYPE");
    check("null || true", "true");
    check("false && true", "false");
    check(r#"false || "default""#, "E_TYPE");
    check("0 ? 1 : 2", "E_TYPE");
    check("[] ? 1 : 2", "E_TYPE");
    check("null ? 1 : 2", "2");
    check("(1 / 0) ? 1 : 2", "E_DIV_ZERO");
}

#[test]
fn only_needed_operands_and_branches_are_evaluated() {
    // A skipped right side and an untaken branch cost nothing and produce
    // no error: && 1 + false 1 = 2; ?: 1 + true 1 + chosen literal 1 = 3.
    let (result, fuel) = expression("false && (1 / 0 == 1)");
    assert_eq!((shown(&result).as_str(), fuel), ("false", 2));
    assert_eq!(expression("true || (1 / 0 == 1)").1, 2);
    assert_eq!(expression("true ? 1 : 1 / 0").1, 3);
    check("true ? 1 : 1 / 0", "1");
    check("false ? 1 / 0 : 2", "2");
    check("null ?? 2", "2");
    check("1 ?? 1 / 0", "1");
    // ?? does not catch errors: an error is not absent.
    check("(1 / 0) ?? 2", "E_DIV_ZERO");
}

#[test]
fn the_first_error_in_evaluation_order_wins_and_stops_the_rest() {
    // The left error returns at once: the right side is never visited.
    let (result, fuel) = expression(r#"(1 / 0) + ("a" * 2)"#);
    assert_eq!(shown(&result), "E_DIV_ZERO");
    // + 1, / 1 and its two literals 2 = 4.
    assert_eq!(fuel, 4);
    check(r#"("a" * 2) + (1 / 0)"#, "E_TYPE");
    check("Round(1 / 0, Sqrt(-1))", "E_DIV_ZERO");
    check("IsError(1 / 0)", "true");
}

#[test]
fn comparisons_follow_section_4_12() {
    check(r#"1 == "1""#, "false");
    check("null == {}.x", "true");
    check("[1, 2] == [1, 2]", "true");
    check("{a: 1, b: 2} == {b: 2, a: 1}", "true");
    check("<Badge>x</Badge> == 1", "E_TYPE");
    check("null < 1", "false");
    check("1 >= null", "false");
    check(r#""a" < "b""#, "true");
    check(r#"1 < "a""#, "E_TYPE");
    check("2 != 2", "false");
}

// §5.2 lookups.

#[test]
fn lookups_return_missing_rather_than_failing_on_shape() {
    check("{a: 1}.a", "1");
    check("{a: 1}.b", "missing");
    check(r#"{a: 1}["a"]"#, "1");
    check("{a: 1}[0]", "missing");
    check("[10, 20][1]", "20");
    check("[10, 20][2]", "missing");
    check("[10, 20][-1]", "missing");
    check("[10, 20][0.5]", "missing");
    check(r#"[10, 20]["a"]"#, "missing");
    check(r#""abc".length"#, "missing");
    check("(5)[0]", "missing");
    check("null.a", "missing");
    check("{}.a.b", "missing");
    check("<Badge>x</Badge>.text", "missing");
    check("(1 / 0).a", "E_DIV_ZERO");
    check("(1 / 0)[0]", "E_DIV_ZERO");
    check("[1][1 / 0]", "E_DIV_ZERO");
}

#[test]
fn member_access_on_a_list_plucks_at_one_fuel_per_element() {
    check("[{a: 1}, {a: 2}, {b: 3}].a", "[1, 2, missing]");
    check("[{a: {b: 1}}, {a: {b: 2}}].a.b", "[1, 2]");
    check("[[{a: 1}], [{a: 2}, {a: 3}]].a", "[[1], [2, 3]]");
    check("[1, null, (1 / 0)].a", "[missing, missing, E_DIV_ZERO]");
    // The list literal costs 13 (1, three records of 1 + 1 + 1, and 3 to
    // build it); the member access adds 1 and the pluck 1 per element, 3.
    assert_eq!(expression("[{a: 1}, {a: 2}, {a: 3}].a").1, 17);
    // One more element costs 3 for its record, 1 more to build the list and
    // 1 more to pluck.
    assert_eq!(
        expression("[{a: 1}, {a: 2}, {a: 3}, {a: 4}].a").1,
        17 + 3 + 1 + 1
    );
    // A record member costs no pluck: 1 + (1 + 1 + 1) = 4.
    assert_eq!(expression("{a: 1}.a").1, 4);
}

// §5.3 evaluation order and memoization.

#[test]
fn named_values_are_evaluated_once_and_charged_at_first_use() {
    let source = "export const a = 1 + 2\nexport const b = a + a\nexport const c = 1 / 0\n\n{b}";
    let evaluation = run(source);
    // Name b 1; b's + 1; first a: name 1 + its + 1 + two literals 2 = 4;
    // second a: only the name, 1. So {b} costs 7. The block value 6 is text
    // in a paragraph of its own: paragraph 1, text 1 + ⌈1/8⌉ = 2. Total 10.
    // The unreached c is never evaluated, so it adds no error.
    assert_eq!(evaluation.fuel, 10);
    assert_eq!(text(&paragraph(&blocks(&evaluation)[0])[0]), "6");
    assert!(evaluation.diagnostics.is_empty());
}

#[test]
fn props_are_evaluated_before_content_and_operands_left_to_right() {
    let source = "export const p = 1 + 1\nexport const c = 2 + 2\n\n\
                  <Badge tone={p == 2 ? \"info\" : \"danger\"}>{c}</Badge>";
    let resolved = program(source);
    let mut inputs = Inputs::new(Profile::ShowUi);
    // The paragraph holding the inline tag 1; the tag 1; ?: 1, == 1, name p
    // 1 + p's expression 3, literal 2 1, then "info" 1. That is 10 when the
    // prop is done. The content's first charge, the name c, is refused.
    inputs.budget = 10;
    let mut evaluator = Evaluator::new(&resolved, &inputs);
    assert_eq!(evaluator.body(), Err(Abort::Fuel));
    assert_eq!(evaluator.meter.fuel, 10);
    assert!(evaluator.evaluated("p"));
    assert!(!evaluator.evaluated("c"));

    let source = "export const l = 1 + 1\nexport const r = 2 + 2\n\n{l + r}";
    let resolved = program(source);
    // + 1, then name l 1 and its expression 3: 5. The name r is refused.
    inputs.budget = 5;
    let mut evaluator = Evaluator::new(&resolved, &inputs);
    assert_eq!(evaluator.body(), Err(Abort::Fuel));
    assert!(evaluator.evaluated("l"));
    assert!(!evaluator.evaluated("r"));
}

#[test]
fn body_blocks_report_errors_in_source_order() {
    let evaluation = run("{\"a\" * 2}\n\n{1 / 0}");
    assert_eq!(
        codes(&evaluation),
        [DiagnosticCode::Type, DiagnosticCode::DivZero]
    );
    let spans: Vec<_> = evaluation
        .diagnostics
        .iter()
        .map(|d| match d.span() {
            Some(crate::DiagnosticSpan::Source(span)) => span.line,
            other => panic!("runtime errors carry their source span: {other:?}"),
        })
        .collect();
    assert_eq!(spans, [1, 3]);
}

// §9.5: why a doubling program stops.

#[test]
fn the_string_doubling_program_stops_before_building_s14() {
    let source = doubling("s", r#""abcdefgh""#, "P + P", 14, "{s14}");
    let evaluation = run(&source);
    assert_eq!(evaluation.abort, Some(Abort::LimitString));
    assert_eq!(
        evaluation.diagnostics.last().map(Diagnostic::code),
        Some(DiagnosticCode::LimitString)
    );
    assert!(evaluation.tree.is_none() && evaluation.demands.is_empty());
    // Fuel up to the step before the abort (§9.2, §5.3). Reading s_k the
    // first time costs F(k): F(0) = name 1 + literal 1 = 2, and for k ≥ 1
    // F(k) = name 1 + `+` 1 + F(k-1) + the memoized second name 1 + the
    // produced string ⌈8·2^k / 8⌉ = 2^k. So F(13) = 2 + 13·3 + (2^14 − 2)
    // = 16,423. Then s14: name 1, `+` 1, F(13), second name 1 = 16,426,
    // and its 131,072-scalar result is refused before it is charged.
    assert_eq!(evaluation.fuel, 16_426);
    // No string over 65,536 scalars was ever built: s13 is the longest.
    assert_eq!(evaluation.longest_string, 65_536);
    // Allocation 16 + 4L per string: s0 48, and s_k 16 + 32·2^k for k in
    // 1..=13, so 48 + 13·16 + 32·(2^14 − 2) = 524,480 (about 0.5 MiB).
    assert_eq!(evaluation.allocation, 524_480);
}

#[test]
fn the_largest_string_within_the_limit_is_built_and_drawn() {
    let source = doubling("s", r#""abcdefgh""#, "P + P", 13, "{s13}");
    let evaluation = run(&source);
    // F(13) = 16,423 (see above), then the paragraph 1 and the text node
    // 1 + ⌈65,536 / 8⌉ = 8,193.
    assert_eq!(evaluation.fuel, 16_423 + 1 + 8_193);
    assert_eq!((evaluation.nodes, evaluation.text), (2, 65_536));
    let text = text(&paragraph(&blocks(&evaluation)[0])[0]);
    assert_eq!(text.chars().count(), 65_536);
    assert!(text.starts_with("abcdefghabcdefgh"));
}

#[test]
fn inserting_t10_stops_before_the_2001st_node() {
    let badge = r#"<Badge tone="info">x</Badge>"#;
    let evaluation = run(&doubling("t", badge, "[P, P]", 10, "{t10}"));
    assert_eq!(evaluation.abort, Some(Abort::LimitNodes));
    // Every node added so far fits the limit; the refused one is the 2,001st.
    assert_eq!(evaluation.nodes, 2_000);
    assert!(evaluation.tree.is_none());
    assert_eq!(Abort::LimitNodes.outcome(), crate::TrialOutcome::LimitNodes);
    assert_eq!(codes(&evaluation), [DiagnosticCode::LimitNodes]);

    // t9 is 512 badges of two nodes each, in one paragraph: 1,025 nodes.
    let evaluation = run(&doubling("t", badge, "[P, P]", 9, "{t9}"));
    assert_eq!(evaluation.abort, None);
    assert_eq!(evaluation.nodes, 1 + 512 * 2);
    let badges = paragraph(&blocks(&evaluation)[0]);
    assert_eq!(badges.len(), 512);
    let TreeNode::Component {
        name,
        props,
        children,
    } = &badges[0]
    else {
        panic!("a badge is a component node: {:?}", badges[0]);
    };
    assert_eq!(name.as_str(), "Badge");
    assert_eq!(props[0].0.as_str(), "tone");
    assert_eq!(props[0].1, TreeValue::String(DisplayText::new("info")));
    assert_eq!(text(&children[0]), "x");
}

#[test]
fn counting_t20_stops_at_the_logical_size_limit_before_any_expansion() {
    let badge = r#"<Badge tone="info">x</Badge>"#;
    let evaluation = run(&doubling("t", badge, "[P, P]", 20, "{Count(t20)}"));
    // Which t_k first passes 4 MiB depends on how a component's text child
    // counts toward its logical size, which the spec leaves open, so only
    // the code and the absence of any tree node are asserted.
    assert_eq!(evaluation.abort, Some(Abort::LimitValue));
    assert_eq!(evaluation.nodes, 0);
    assert!(evaluation.fuel < 1_000);
}

// §9.4 limits and §9.2 fuel.

#[test]
fn exhausted_fuel_aborts_at_the_refused_charge() {
    let mut inputs = Inputs::new(Profile::ShowUi);
    // Two `+` and three literals: 5. Adding the paragraph is refused.
    inputs.budget = 5;
    let evaluation = run_with("{1 + 2 + 3}", &inputs);
    assert_eq!(evaluation.abort, Some(Abort::Fuel));
    assert_eq!(evaluation.fuel, 5);
    assert_eq!(codes(&evaluation), [DiagnosticCode::Fuel]);
    inputs.budget = 8;
    assert_eq!(run_with("{1 + 2 + 3}", &inputs).abort, None);
}

#[test]
fn created_lists_and_records_abort_at_their_element_limits() {
    assert_eq!(run("{Count(Range(0, 10000))}").abort, None);
    assert_eq!(
        run("{Count(Range(0, 10001))}").abort,
        Some(Abort::LimitList)
    );
    let record = |fields: usize| {
        let fields: Vec<_> = (0..fields).map(|i| format!("a{i}: 0")).collect();
        format!("{{Count(Keys({{{}}}))}}", fields.join(", "))
    };
    let evaluation = run(&record(1_024));
    assert_eq!(text(&paragraph(&blocks(&evaluation)[0])[0]), "1024");
    let evaluation = run(&record(1_025));
    assert_eq!(evaluation.abort, Some(Abort::LimitRecord));
}

#[test]
fn value_nesting_aborts_past_32_levels() {
    let nested = |levels: usize| format!("{{Count({}{})}}", "[".repeat(levels), "]".repeat(levels));
    assert_eq!(run(&nested(32)).abort, None);
    assert_eq!(run(&nested(33)).abort, Some(Abort::LimitDepth));
}

#[test]
fn allocation_per_evaluation_aborts_past_16_mib() {
    // Each template builds a new 513-scalar string (16 + 4·513 bytes) and a
    // number, but keeps only the number, so the result list stays small
    // while the evaluation's allocation passes 16 MiB near element 8,000.
    let source = doubling(
        "s",
        r#""abcdefgh""#,
        "P + P",
        6,
        r#"{Count(Each(Range(0, 10000), "i", Len(s6 + "x")))}"#,
    );
    let evaluation = run(&source);
    assert_eq!(evaluation.abort, Some(Abort::LimitAlloc));
    assert!(evaluation.allocation <= 16 * 1024 * 1024);
    assert!(evaluation.fuel < FUEL_BUDGET);
}

#[test]
fn output_text_aborts_past_262144_scalars() {
    let source = doubling(
        "s",
        r#""abcdefgh""#,
        "P + P",
        13,
        "{[s13, s13, s13, s13, s13]}",
    );
    let evaluation = run(&source);
    assert_eq!(evaluation.abort, Some(Abort::LimitText));
    // Four copies fit exactly; the fifth would pass the limit.
    assert_eq!(evaluation.text, 4 * 65_536);
}

#[test]
fn output_depth_aborts_past_32_levels() {
    let mut inputs = Inputs::new(Profile::ShowUi);
    let card = |levels: usize| {
        let empty = || match Value::record(vec![]).unwrap().kind() {
            ValueKind::Record(record) => record.clone(),
            _ => unreachable!(),
        };
        let mut value = Value::string("x").unwrap();
        for _ in 0..levels {
            value = Value::component("Card", empty(), vec![value]).unwrap();
        }
        value
    };
    // 31 cards and their text: 32 levels.
    inputs.input = card(31);
    let evaluation = run_with("{input}", &inputs);
    assert_eq!((evaluation.abort, evaluation.nodes), (None, 32));
    inputs.input = card(32);
    let evaluation = run_with("{input}", &inputs);
    assert_eq!(evaluation.abort, Some(Abort::LimitDepth));
    assert_eq!(evaluation.nodes, 32);
}

// §5.5 Each.

#[test]
fn each_maps_a_template_over_a_list() {
    // Call 1; list literal 1 + 3 + 3 = 7; one per element 3; the template
    // `n * 2` costs 3 per element: 9. Total 20.
    let (result, fuel) = expression(r#"Each([1, 2, 3], "n", n * 2)"#);
    assert_eq!((shown(&result).as_str(), fuel), ("[2, 4, 6]", 20));
    check(r#"Each(null, "n", n)"#, "[]");
    check(r#"Each({}.x, "n", n)"#, "[]");
    check(r#"Each(5, "n", n)"#, "E_TYPE");
    check(r#"Each(1 / 0, "n", n)"#, "E_DIV_ZERO");
    check(
        r#"Each([1, 2], "x", Each([10], "y", x + y))"#,
        "[[11], [12]]",
    );
}

#[test]
fn each_refuses_invalid_binders_without_running_the_template() {
    for source in [
        r#"Each([1], "data", 1 / 0)"#,
        r#"Each([1], "Item", 1 / 0)"#,
        r#"Each([1], "now", 1 / 0)"#,
        r#"Each([1], "1x", 1 / 0)"#,
        r#"Each([1], "a b", 1 / 0)"#,
        r#"Each([1], "x" + "", 1 / 0)"#,
    ] {
        check(source, "E_EACH_BINDER");
    }
    check(r#"Each([1], "x", Each([2], "x", x))"#, "[E_EACH_BINDER]");
    let evaluation =
        run("export const a = 1\nexport let $b = 2\n\n{Each([1], \"a\", 1)} {Each([1], \"b\", 1)}");
    assert_eq!(codes(&evaluation), [DiagnosticCode::EachBinder; 2]);
}

#[test]
fn each_nests_at_most_four_deep() {
    check(
        r#"Each([1], "a", Each([2], "b", Each([3], "c", Each([4], "d", a + b + c + d))))"#,
        "[[[[10]]]]",
    );
    check(
        r#"Each([1], "a", Each([1], "b", Each([1], "c", Each([1], "d", Each([1], "e", e)))))"#,
        "[[[[E_EACH_NESTING]]]]",
    );
}

#[test]
fn a_binder_is_not_visible_inside_a_named_value() {
    let evaluation = run("export const f = x\n\n{Each([1], \"x\", f)}");
    assert_eq!(codes(&evaluation), [DiagnosticCode::UnknownName]);
}

// §5.6 and §3.12: building the tree.

#[test]
fn inserted_values_follow_the_interpolation_table() {
    let evaluation = run(r#"Total: {[1, [2, null], "a", true, {}.x]} **{"**b**"}**"#);
    let inline = paragraph(&blocks(&evaluation)[0]);
    let texts: Vec<_> = inline[..5].iter().map(text).collect();
    assert_eq!(texts, ["Total: ", "1", "2", "a", "true"]);
    let TreeNode::Markdown {
        kind: MarkdownKind::Strong,
        children,
    } = &inline[6]
    else {
        panic!("interpolation sits inside emphasis: {inline:?}");
    };
    // A string is literal text: its asterisks are not Markdown.
    assert_eq!(text(&children[0]), "**b**");

    assert!(blocks(&run("{null}")).is_empty());
    let evaluation = run("{1 / 0}\n\n{{a: 1}}\n\n{\"alone\"}");
    let blocks = blocks(&evaluation);
    assert_eq!(
        paragraph(&blocks[0]),
        [TreeNode::ErrorMarker(DiagnosticCode::DivZero)]
    );
    assert_eq!(
        paragraph(&blocks[1]),
        [TreeNode::ErrorMarker(DiagnosticCode::Type)]
    );
    assert_eq!(text(&paragraph(&blocks[2])[0]), "alone");
    assert_eq!(
        codes(&evaluation),
        [DiagnosticCode::DivZero, DiagnosticCode::Type]
    );
    assert!(evaluation
        .diagnostics
        .iter()
        .all(|d| d.phase() == DiagnosticPhase::Evaluate && d.span().is_some()));
}

#[test]
fn every_tree_string_is_sanitized() {
    let source = "Bell\u{7} text `code\u{1b}`\n\n{\"a\\u{7}b\\tc\\nd\\u{85}\"}\n\n<Badge tone=\"info\" text={\"t\\u{9f}\"} />";
    let evaluation = run(source);
    let blocks = blocks(&evaluation);
    let first = paragraph(&blocks[0]);
    assert_eq!(text(&first[0]), "Bell\u{FFFD} text ");
    let TreeNode::Markdown {
        kind: MarkdownKind::Code(code),
        ..
    } = &first[1]
    else {
        panic!("a code span follows the text: {first:?}");
    };
    assert_eq!(code.as_str(), "code\u{FFFD}");
    // Tab and line feed are kept; other C0 and C1 controls are replaced.
    assert_eq!(text(&paragraph(&blocks[1])[0]), "a\u{FFFD}b\tc\nd\u{FFFD}");
    let TreeNode::Component { props, .. } = &paragraph(&blocks[2])[0] else {
        panic!("an inline badge on its own line sits in a paragraph");
    };
    let TreeValue::String(prop) = &props[1].1 else {
        panic!("the text prop is a string: {props:?}");
    };
    assert_eq!(prop.as_str(), "t\u{FFFD}");
}

#[test]
fn props_are_checked_against_the_catalog_when_placed() {
    let evaluation = run("<Badge tone={1 / 0}>x</Badge>");
    assert_eq!(
        paragraph(&blocks(&evaluation)[0]),
        [TreeNode::ErrorCell {
            name: DisplayText::new("Badge"),
            code: DiagnosticCode::DivZero,
        }]
    );
    assert_eq!(codes(&evaluation), [DiagnosticCode::DivZero]);

    // A computed string outside the tone vocabulary has the wrong type.
    let evaluation = run("export const t = \"pur\" + \"ple\"\n\n<Badge tone={t}>x</Badge>");
    assert_eq!(
        paragraph(&blocks(&evaluation)[0]),
        [TreeNode::ErrorCell {
            name: DisplayText::new("Badge"),
            code: DiagnosticCode::PropType,
        }]
    );

    // An absent prop is not given, so the catalog default applies.
    let evaluation = run("<Badge tone={null}>x</Badge>");
    let TreeNode::Component {
        props, children, ..
    } = &paragraph(&blocks(&evaluation)[0])[0]
    else {
        panic!("an absent prop does not fail the component");
    };
    assert!(props.is_empty());
    assert_eq!(text(&children[0]), "x");

    // A required prop not given draws an error cell.
    let evaluation = run("<CardHeader title={null} />");
    assert_eq!(
        blocks(&evaluation),
        [TreeNode::ErrorCell {
            name: DisplayText::new("CardHeader"),
            code: DiagnosticCode::MissingProp,
        }]
    );
}

/// A Slot waiting on absent data draws `waiting: true` with no children, and
/// its content is never evaluated, so content that would fail on the missing
/// data costs nothing. That holds for a missing value too, such as a field
/// read on a record that hasn't arrived, alone or at the end of a `??` chain.
/// Once the data is there the content draws, and `needs` itself is never in
/// the tree.
#[test]
fn a_slot_evaluates_its_content_only_once_its_needs_arrive() {
    let row = Inputs::new(Profile::ToolRow);
    for needs in [
        "null",
        "(view ?? null)",
        "view.record",
        "view.record.path",
        "view.completion.path ?? view.record.path",
    ] {
        let evaluation = run_with(
            &format!(
                "<Slot width=\"short\" needs={{{needs}}}><Badge tone={{1 / 0}}>x</Badge></Slot>"
            ),
            &row,
        );
        assert_eq!(
            paragraph(&blocks(&evaluation)[0]),
            [TreeNode::Component {
                name: DisplayText::new("Slot"),
                props: vec![
                    (
                        DisplayText::new("width"),
                        TreeValue::String(DisplayText::new("short"))
                    ),
                    (DisplayText::new("waiting"), TreeValue::Boolean(true)),
                ],
                children: vec![],
            }],
            "{needs}"
        );
        assert!(
            codes(&evaluation).is_empty(),
            "{needs}: {:?}",
            codes(&evaluation)
        );
    }
    for slot in [
        "<Slot width=\"short\" needs={7}><Badge>x</Badge></Slot>",
        "<Slot width=\"short\"><Badge>x</Badge></Slot>",
    ] {
        let evaluation = run_with(slot, &row);
        let TreeNode::Component {
            props, children, ..
        } = &paragraph(&blocks(&evaluation)[0])[0]
        else {
            panic!("{slot} draws a Slot");
        };
        assert_eq!(
            props,
            &[(
                DisplayText::new("width"),
                TreeValue::String(DisplayText::new("short"))
            )],
            "{slot}"
        );
        assert_eq!(children.len(), 1, "{slot}");
    }
}

#[test]
fn actions_freeze_their_arguments_and_bindings_name_their_state() {
    let source = "export let $n = 1\n\n<Button label=\"Go\" action={Set($n, $n + 1)} />\n\n\
                  <Slider value={$n} min={0} max={10} />\n\n<Button label=\"Bad\" action={Compose(5)} />";
    let evaluation = run(source);
    let blocks = blocks(&evaluation);
    let TreeNode::Component { props, .. } = &blocks[0] else {
        panic!("a button is a block component: {:?}", blocks[0]);
    };
    assert_eq!(
        props[1].1,
        TreeValue::Action(vec![TreeStep {
            operation: ActionOperation::Set,
            arguments: vec![
                TreeValue::String(DisplayText::new("n")),
                TreeValue::Number(Decimal::from_integer(2).unwrap()),
            ],
        }])
    );
    let TreeNode::Component { props, .. } = &blocks[1] else {
        panic!("a slider is a block component");
    };
    assert_eq!(
        props.last().unwrap().1,
        TreeValue::Binding {
            state: DisplayText::new("n"),
            kind: "number",
        }
    );
    // An action prop that is an error leaves the control disabled; the
    // component itself still draws.
    let TreeNode::Component { props, .. } = &blocks[2] else {
        panic!("a bad action does not make an error cell");
    };
    assert_eq!(props[1].1, TreeValue::Error(DiagnosticCode::Type));
    check(r#"Open("ftp://example.com")"#, "E_SCHEME");
    assert!(matches!(
        value(r#"Compose("x")"#).kind(),
        ValueKind::Action(_)
    ));
}

// §4.8 snapshots and §10.2 Parse.

fn record(state: ReferenceState, reason: Option<&str>) -> ReferenceRecord {
    ReferenceRecord {
        state,
        reason: reason.map(str::to_owned),
        tool: Some("bash".into()),
        bytes: Some(4),
        call_key: None,
    }
}

fn keys(snapshot: &Value) -> Vec<String> {
    match snapshot.kind() {
        ValueKind::Snapshot(snapshot) => snapshot.members.keys().map(str::to_owned).collect(),
        other => panic!("not a snapshot: {other:?}"),
    }
}

#[test]
fn snapshot_members_follow_section_4_8() {
    let bytes = b"\xEF\xBB\xBFab\x07";
    let resolved = snapshot(
        "r1",
        &record(ReferenceState::Resolved, None),
        Some(bytes),
        None,
    );
    assert_eq!(keys(&resolved), ["text", "state", "tool", "bytes"]);
    let ValueKind::Snapshot(members) = resolved.kind() else {
        unreachable!()
    };
    // The byte-order mark is removed; controls stay in the value (§10.1).
    assert_eq!(shown(&members.members.get("text")), r#""ab\u{7}""#);
    assert_eq!(shown(&members.members.get("state")), r#""resolved""#);
    assert_eq!(shown(&members.members.get("bytes")), "4");
    assert_eq!(shown(&members.members.get("value")), "missing");
    assert_eq!(shown(&members.members.get("rows")), "missing");

    let unresolved = snapshot(
        "r2",
        &record(ReferenceState::Unresolved, Some("not_a_tool_output")),
        Some(bytes),
        None,
    );
    assert_eq!(keys(&unresolved), ["state", "reason", "tool", "bytes"]);
    for (state, name) in [
        (ReferenceState::TooLarge, "too_large"),
        (ReferenceState::Held, "held"),
    ] {
        let value = snapshot("r3", &record(state, Some("ignored")), Some(bytes), None);
        assert_eq!(keys(&value), ["state", "tool", "bytes"]);
        let ValueKind::Snapshot(members) = value.kind() else {
            unreachable!()
        };
        assert_eq!(shown(&members.members.get("state")), format!("{name:?}"));
    }

    let mut inputs = Inputs::new(Profile::ShowUi);
    inputs.data = resolved;
    let evaluation = run_with(
        "{data.state} {data.tool} {data.reason} {$$tc(9).state}",
        &inputs,
    );
    let texts: Vec<_> = paragraph(&blocks(&evaluation)[0])
        .iter()
        .map(text)
        .collect();
    assert_eq!(texts, ["resolved", " ", "bash", " ", " ", "unresolved"]);
}

fn prepared(bytes: &[u8], format: Format, options: &Options) -> data::Prepared {
    let span = SourceSpan {
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 2,
    };
    data::prepare(bytes, format, options, span, &mut |_| Ok::<(), ()>(())).unwrap()
}

fn parse_inputs(bytes: &[u8]) -> Inputs {
    let mut inputs = Inputs::new(Profile::ShowUi);
    let record = record(ReferenceState::Resolved, None);
    inputs
        .references
        .insert("1".into(), snapshot("r1", &record, Some(bytes), None));
    inputs.prepared.insert(
        ParseKey {
            source: SnapshotSource::Reference("1".into()),
            format: Format::Json,
            options: Options::default(),
        },
        prepared(bytes, Format::Json, &Options::default()),
    );
    inputs
}

#[test]
fn parse_reads_prepared_data_at_the_cost_of_a_lookup() {
    let source = r#"{Sum(Parse($$tc(1), "json").rows.n)}"#;
    let evaluation = run_with(source, &parse_inputs(br#"[{"n": 1}, {"n": 2}]"#));
    assert_eq!(text(&paragraph(&blocks(&evaluation)[0])[0]), "3");
    assert_eq!(
        parse_keys(&program(source)),
        [ParseKey {
            source: SnapshotSource::Reference("1".into()),
            format: Format::Json,
            options: Options::default(),
        }]
    );

    // Count 1 + 1; .rows 1; Parse 1 (the lookup); the reference 1: 5. The
    // paragraph 1 and a text of at most 8 scalars 2: 8, whatever the size
    // of the data that was prepared before evaluation.
    let source = r#"{Count(Parse($$tc(1), "json").rows)}"#;
    let small = run_with(source, &parse_inputs(br#"[1, 2]"#));
    let rows = format!("[{}]", vec!["1"; 1000].join(","));
    let large = run_with(source, &parse_inputs(rows.as_bytes()));
    assert_eq!(text(&paragraph(&blocks(&small)[0])[0]), "2");
    assert_eq!(text(&paragraph(&blocks(&large)[0])[0]), "1000");
    assert_eq!((small.fuel, large.fuel), (8, 8));

    // A failed parse keeps text; value and rows are the parse's error.
    let failed = parse_inputs(b"[1,");
    let evaluation = run_with(
        r#"{Parse($$tc(1), "json").rows} {Parse($$tc(1), "json").text}"#,
        &failed,
    );
    let inline = paragraph(&blocks(&evaluation)[0]);
    assert_eq!(inline[0], TreeNode::ErrorMarker(DiagnosticCode::JsonSyntax));
    assert_eq!(text(&inline[2]), "[1,");
}

#[test]
fn parse_keys_come_from_literal_formats_and_options() {
    let source = r#"{Parse($$tc(2), "csv", {delimiter: ";", header: false, columns: {a: "number"}})}{Parse(data, "json")}"#;
    assert_eq!(
        parse_keys(&program(source)),
        [
            ParseKey {
                source: SnapshotSource::Reference("2".into()),
                format: Format::Csv,
                options: Options {
                    columns: vec![("a".into(), data::ColumnType::Number)],
                    delimiter: data::Delimiter::Semicolon,
                    header: false,
                },
            },
            ParseKey {
                source: SnapshotSource::Data,
                format: Format::Json,
                options: Options::default(),
            },
        ]
    );
    let inputs = parse_inputs(b"[]");
    for source in [
        r#"{Parse($$tc(1), "xml")}"#,
        r#"{Parse($$tc(1), "json", {delimiter: ";"})}"#,
        r#"{Parse($$tc(1), "csv", {quote: "'"})}"#,
    ] {
        assert_eq!(
            codes(&run_with(source, &inputs)),
            [DiagnosticCode::Arg],
            "{source}"
        );
    }
    // Without a prepared result the snapshot is returned as it is.
    let evaluation = run_with(r#"{Parse($$tc(1), "csv").value ?? "none"}"#, &inputs);
    assert_eq!(text(&paragraph(&blocks(&evaluation)[0])[0]), "none");
}

// §8: the demand set.

#[test]
fn reached_sites_join_the_demand_set_and_read_their_envelopes() {
    let source = "export const q = Query(\"status\", {id: 1}, {running: null})\n\
                  export const w = Watch(\"tail\", {})\n\n\
                  {q.status} {q.value.running ?? \"unknown\"} {q.stale}";
    let evaluation = run(source);
    let texts: Vec<_> = paragraph(&blocks(&evaluation)[0])
        .iter()
        .map(text)
        .collect();
    assert_eq!(texts, ["loading", " ", "unknown", " ", "false"]);
    // The unreached watch is never demanded.
    assert_eq!(evaluation.demands.len(), 1);
    let demand = &evaluation.demands[0];
    assert_eq!((&*demand.site, &*demand.constructor), ("q", "Query"));
    assert_eq!(
        demand.arguments.iter().map(shown).collect::<Vec<_>>(),
        [r#""status""#, "{id: 1}", "{running: null}"]
    );

    let mut inputs = Inputs::new(Profile::ShowUi);
    let members = match Value::record(vec![
        (
            ScalarString::new("value"),
            Value::record(vec![(ScalarString::new("running"), Value::boolean(true))]).unwrap(),
        ),
        (ScalarString::new("status"), Value::string("ready").unwrap()),
    ])
    .unwrap()
    .kind()
    {
        ValueKind::Record(record) => record.clone(),
        _ => unreachable!(),
    };
    inputs.sites.insert(
        "q".into(),
        Value::site_envelope(crate::SiteEnvelope {
            name: "q".into(),
            members,
        }),
    );
    let evaluation = run_with(source, &inputs);
    let texts: Vec<_> = paragraph(&blocks(&evaluation)[0])
        .iter()
        .map(text)
        .collect();
    assert_eq!(texts[..3], ["ready", " ", "true"]);

    // An abort dispatches nothing.
    let evaluation = run(&format!("{source}\n\n{{Range(0, 10001)}}"));
    assert_eq!(evaluation.abort, Some(Abort::LimitList));
    assert!(evaluation.demands.is_empty());
}

#[test]
fn runtime_diagnostics_list_fifty_and_count_the_rest() {
    let evaluation = run(r#"{Each(Range(0, 60), "i", 1 / 0)}"#);
    assert_eq!(evaluation.abort, None);
    assert_eq!(evaluation.diagnostics.len(), 50);
    assert_eq!(evaluation.omitted, 10);
}

#[test]
fn state_reads_the_current_value_or_the_declared_one() {
    let source = "export let $n = -1.5\nexport let $tags = [\"a\"]\n\n{$n} {Count($tags)}";
    let resolved = program(source);
    let initial = initial_state(&resolved);
    assert_eq!(
        initial
            .iter()
            .map(|(name, value)| format!("{name}={}", shown(value)))
            .collect::<Vec<_>>(),
        ["n=-1.5", r#"tags=["a"]"#]
    );
    let evaluation = evaluate(&resolved, &Inputs::new(Profile::ShowUi));
    let texts: Vec<_> = paragraph(&blocks(&evaluation)[0])
        .iter()
        .map(text)
        .collect();
    assert_eq!(texts, ["-1.5", " ", "1"]);
    let mut inputs = Inputs::new(Profile::ShowUi);
    inputs
        .state
        .insert("n".into(), Value::number(Decimal::from_integer(7).unwrap()));
    let evaluation = evaluate(&resolved, &inputs);
    assert_eq!(text(&paragraph(&blocks(&evaluation)[0])[0]), "7");
}

#[test]
fn unsupported_expression_is_literal_in_reply_and_refused_in_program() {
    for mode in [Mode::Reply, Mode::Program] {
        let mut program = program("{1 / 0}");
        program.parsed.document.mode = mode;
        let inputs = Inputs::new(if mode == Mode::Reply {
            Profile::Reply
        } else {
            Profile::ShowUi
        });
        let mut evaluator = Evaluator::new(&program, &inputs);
        let node = &first_interpolation(&program.parsed.document.nodes)
            .unwrap()
            .children[0];
        let value = evaluator.unsupported(node).unwrap();
        if mode == Mode::Reply {
            assert_eq!(shown(&value), "\"1 / 0\"");
        } else {
            assert_eq!(shown(&value), "E_UNSUPPORTED_VERSION");
        }
        assert!(evaluator.demands.is_empty());
        assert!(evaluator.diagnostics.is_empty());
    }
}

#[test]
fn unsupported_markdown_source_is_literal_in_reply_and_refused_in_program() {
    for mode in [Mode::Reply, Mode::Program] {
        let mut program = program("*do not interpret*\n");
        program.parsed.document.mode = mode;
        let inputs = Inputs::new(if mode == Mode::Reply {
            Profile::Reply
        } else {
            Profile::ShowUi
        });
        let mut evaluator = Evaluator::new(&program, &inputs);
        let node = &program.parsed.document.nodes[0];
        let mut out = vec![];
        evaluator.markdown_source(node, None, &mut out, 1).unwrap();
        if mode == Mode::Reply {
            assert_eq!(
                out,
                vec![TreeNode::Text(DisplayText::new("*do not interpret*\n"))]
            );
            assert!(evaluator.diagnostics.is_empty());
        } else {
            assert_eq!(
                out,
                vec![TreeNode::ErrorMarker(DiagnosticCode::UnsupportedVersion)]
            );
            assert_eq!(evaluator.diagnostics.len(), 1);
            assert_eq!(
                evaluator.diagnostics[0].code(),
                DiagnosticCode::UnsupportedVersion
            );
        }
    }
}

#[test]
fn unsupported_markdown_content_is_literal_in_reply_and_refused_in_program() {
    for mode in [Mode::Reply, Mode::Program] {
        let mut program = program("*do not interpret*\n");
        program.parsed.document.mode = mode;
        let inputs = Inputs::new(if mode == Mode::Reply {
            Profile::Reply
        } else {
            Profile::ShowUi
        });
        let mut evaluator = Evaluator::new(&program, &inputs);
        let value = evaluator
            .markdown_content(&program.parsed.document.nodes[0], None)
            .unwrap();
        if mode == Mode::Reply {
            assert_eq!(shown(&value), "\"*do not interpret*\\n\"");
        } else {
            assert_eq!(shown(&value), "E_UNSUPPORTED_VERSION");
        }
        assert!(evaluator.demands.is_empty());
    }
}
