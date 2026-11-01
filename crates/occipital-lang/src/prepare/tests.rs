use super::resolve::Resolved;
use super::types::{SiteKind, Type, TypeContext};
use super::*;
use crate::{DiagnosticCode as Code, DiagnosticSpan, Severity};

fn checked(source: &str) -> Resolved {
    prepare(source.as_bytes(), Mode::Program, &TypeContext::default()).unwrap()
}

fn codes(result: &Resolved) -> Vec<Code> {
    result.diagnostics.iter().map(Diagnostic::code).collect()
}

fn clean(source: &str) -> Resolved {
    let result = checked(source);
    assert!(
        result.diagnostics.is_empty(),
        "{source}: {:?}",
        result.diagnostics
    );
    result
}

macro_rules! diagnostic {
    ($name:ident, $source:expr, $code:ident) => {
        #[test]
        fn $name() {
            let result = checked($source);
            assert_eq!(
                codes(&result),
                vec![Code::$code],
                "{:?}",
                result.diagnostics
            );
            let diagnostic = &result.diagnostics[0];
            assert!(matches!(diagnostic.span(), Some(DiagnosticSpan::Source(_))));
            assert_eq!(diagnostic.severity(), Code::$code.severity());
        }
    };
}

diagnostic!(parse_syntax, "{ @ }", Syntax);
diagnostic!(parse_number_literal, "{01}", NumberLiteral);
diagnostic!(parse_unclosed_tag, "<Badge>x", UnclosedTag);
diagnostic!(parse_mismatched_close, "<Badge>x</Text>", MismatchedClose);
diagnostic!(
    parse_duplicate_prop,
    "<Badge tone=\"info\" tone=\"warning\" />",
    DuplicateProp
);
diagnostic!(parse_duplicate_key, "{{a: 1, a: 2}}", DuplicateKey);
diagnostic!(parse_nonassoc, "{1 < 2 < 3}", Nonassoc);
diagnostic!(
    parse_mixed_coalesce,
    "{true ?? false || true}",
    MixedCoalesce
);
diagnostic!(
    parse_declaration_incomplete,
    "export const a =\n",
    DeclarationIncomplete
);
diagnostic!(unknown_name, "{typo}", UnknownName);
diagnostic!(unknown_state, "{$typo}", UnknownState);
diagnostic!(
    duplicate_name_across_state_and_const,
    "export let $a = 0\nexport const a = 1\n",
    DuplicateName
);
diagnostic!(reserved_uppercase, "export const Badge = 1\n", ReservedName);
diagnostic!(reserved_host, "export const input = 1\n", ReservedName);
diagnostic!(
    syntax_keyword_refusal_passes_through,
    "export const true = 1\n",
    ReservedName
);
diagnostic!(unknown_function, "{NotAFunction(1)}", UnknownFunction);
diagnostic!(wrong_arity, "{Round(1, 2, 3)}", Arity);
diagnostic!(argument_type, "{Round(\"x\")}", ArgType);
diagnostic!(operand_type, "{\"x\" * 2}", ArgType);
diagnostic!(
    literal_required_for_state,
    "export let $n = Round(1)\n",
    LiteralRequired
);
diagnostic!(each_invalid_binder, "{Each([1], \"Bad\", 1)}", EachBinder);
diagnostic!(
    each_binder_does_not_escape,
    "{Each([1], \"item\", item)}\n\n{item}",
    UnknownName
);
diagnostic!(state_needs_dollar, "export let $n = 0\n\n{n}", UnknownName);
diagnostic!(
    const_is_not_state,
    "export const n = 0\n\n{$n}",
    UnknownState
);
diagnostic!(
    writes_refused_mutation,
    "{Mutation(\"op\", {})}",
    WriteNotAllowed
);
diagnostic!(writes_refused_run, "{Run(\"op\")}", WriteNotAllowed);

fn assert_syntax_refusal(source: &str, expected: Code) {
    let parsed = parse(source.as_bytes(), Mode::Program).unwrap();
    assert_eq!(
        parsed
            .document
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        vec![expected.as_str()],
        "{source}",
    );
    let result = resolve::resolve(parsed, &TypeContext::default());
    assert_eq!(codes(&result), vec![expected], "{source}");
    assert!(result.declarations.is_empty(), "{source}");
}

#[test]
fn syntax_refuses_reserved_declarations_before_resolution() {
    for source in [
        "export const Upper = 1\n",
        "export let $Upper = 1\n",
        "export const now = 1\n",
        "export let $view = 1\n",
    ] {
        assert_syntax_refusal(source, Code::ReservedName);
    }
}

#[test]
fn syntax_refuses_duplicate_declarations_before_resolution() {
    for source in [
        "export const a = 0\nexport const a = 1\n",
        "export let $a = 0\nexport let $a = 1\n",
        "export const a = 0\nexport let $a = 1\n",
        "export let $a = 0\nexport const a = 1\n",
    ] {
        assert_syntax_refusal(source, Code::DuplicateName);
    }
}

#[test]
fn syntax_refuses_nonliteral_state_before_resolution() {
    for source in [
        "export const next = 1\nexport let $n = next\n",
        "export let $n = $n\n",
        "export const next = 1\nexport let $n = [next]\n",
        "export let $n = Round(1)\n",
    ] {
        assert_syntax_refusal(source, Code::LiteralRequired);
    }
}

#[test]
fn parse_source_too_large_and_exact_boundary() {
    let source = vec![b'x'; 65_537];
    let result = prepare(&source, Mode::Program, &TypeContext::default()).unwrap();
    assert_eq!(codes(&result), vec![Code::SourceTooLarge]);
    assert!(result.parsed.document.fallback);
    assert!(result.declarations.is_empty());
    let boundary = parse(&source[..65_536], Mode::Program).unwrap();
    assert!(!boundary.document.fallback);
    assert!(boundary.diagnostics.is_empty());
}

#[test]
fn parse_encoding_repair_uses_syntax_bytes_api() {
    let parsed = parse(b"\xff\0", Mode::Program).unwrap();
    assert_eq!(
        parsed
            .diagnostics
            .iter()
            .map(Diagnostic::code)
            .collect::<Vec<_>>(),
        vec![Code::Encoding]
    );
    fn text(nodes: &[cortexkit_rich_ui_syntax::Node]) -> String {
        nodes
            .iter()
            .map(|node| {
                let value = if let cortexkit_rich_ui_syntax::Kind::Text(value) = &node.kind {
                    value.as_str()
                } else {
                    ""
                };
                format!("{value}{}", text(&node.children))
            })
            .collect()
    }
    assert_eq!(text(&parsed.document.nodes), "��");
    assert_eq!(parsed.diagnostics[0].severity(), Severity::Warning);
}

#[test]
fn source_positions_are_repaired_scalar_coordinates() {
    let result = checked("\u{feff}é {$nope}\r\n\r{$again}\n{$last}");
    let spans: Vec<_> = result
        .diagnostics
        .iter()
        .map(|d| d.span().unwrap())
        .collect();
    assert_eq!(
        spans,
        vec![
            SourceSpan {
                line: 1,
                column: 4,
                end_line: 1,
                end_column: 9
            }
            .into(),
            SourceSpan {
                line: 3,
                column: 2,
                end_line: 3,
                end_column: 8
            }
            .into(),
            SourceSpan {
                line: 4,
                column: 2,
                end_line: 4,
                end_column: 7
            }
            .into(),
        ]
    );
}

#[test]
fn parse_nesting() {
    let source = format!("{{{}1{}}}", "(".repeat(66), ")".repeat(66));
    assert_eq!(codes(&checked(&source)), vec![Code::Nesting]);
}

#[test]
fn parse_limit_declarations() {
    let source: String = (0..257)
        .map(|i| format!("export const n{i} = 1\n"))
        .collect();
    assert_eq!(codes(&checked(&source)), vec![Code::LimitDeclarations]);
    let boundary: String = (0..256)
        .map(|i| format!("export const n{i} = 1\n"))
        .collect();
    assert_eq!(clean(&boundary).declarations.len(), 256);
}

#[test]
fn syntax_string_errors_pass_through_without_fabrication() {
    for (source, code) in [
        (r#"{"bad\q"}"#, Code::BadEscape),
        ("{\"unterminated}", Code::UnterminatedString),
    ] {
        let native = cortexkit_rich_ui_syntax::parse_bytes(source.as_bytes(), Mode::Program);
        assert_eq!(native.diagnostics[0].code.as_str(), code.as_str());
        assert_eq!(codes(&checked(source)), vec![code]);
    }
    assert_eq!(
        codes(&checked("export const a = \"bad\\q\"\n")),
        vec![Code::BadEscape]
    );
}

#[test]
fn syntax_entity_decoding_and_reference_identity_are_retained() {
    let parsed = parse(b"<Badge>&amp; $$tc(1)</Badge>", Mode::Reply).unwrap();
    assert!(parsed.diagnostics.is_empty());
    let references = parsed.document.references();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].n, "1");
    assert_eq!(references[0].span, Span::new(13, 20));
    let escaped = parse(b"&dollar;&dollar;tc(1) `$$tc(2)`", Mode::Reply).unwrap();
    assert!(escaped.document.references().is_empty());
}

#[test]
fn forward_names_are_resolved_and_typed_in_dependency_order() {
    let result = clean("export const a = b + 1\nexport const b = 2\n\n{a}");
    assert_eq!(result.declarations[0].dependencies, vec![1]);
    assert_eq!(result.declarations[0].ty, Type::Number);
    assert_eq!(result.declarations[1].ty, Type::Number);
    assert_eq!(
        codes(&checked(
            "export const a = Round(b)\nexport const b = \"x\"\n\n{a}"
        )),
        vec![Code::ArgType]
    );
}

fn assert_cycle(source: &str, expected: &[&str]) {
    let result = checked(source);
    assert_eq!(codes(&result), vec![Code::Cycle]);
    let json = serde_json::to_value(&result.diagnostics[0]).unwrap();
    assert_eq!(json["cycle"], serde_json::json!(expected));
    assert_eq!(json["phase"], "check");
}

#[test]
fn cycle_names_are_in_dependency_order() {
    assert_cycle(
        "export const a = c\nexport const b = a\nexport const c = b\n",
        &["a", "c", "b", "a"],
    );
    assert_cycle("export const a = a\n", &["a", "a"]);
}

#[test]
fn cycles_include_unreachable_branches_props_content_templates_sites_and_actions() {
    for expression in [
        "false ? b : 0",
        "<Badge label={b} />",
        "<Badge>{b}</Badge>",
        "Each([1], \"item\", b)",
        "Query(\"op\", {arg: b}, 0)",
        "Compose(b)",
    ] {
        assert_cycle(
            &format!("export const a = {expression}\nexport const b = a\n"),
            &["a", "b", "a"],
        );
    }
}

#[test]
fn state_reads_and_writes_are_not_cycle_edges() {
    let result = clean("export let $page = 0\nexport const next = Set($page, $page + 1)\n\n<Button action={next}>Next</Button>");
    assert!(result
        .declarations
        .iter()
        .all(|declaration| declaration.dependencies.is_empty()));
    assert_eq!(result.declarations[1].ty, Type::Action);
}

#[test]
fn each_scope_collisions_and_nesting() {
    for source in [
        "export const item = 1\n\n{Each([1], \"item\", item)}",
        "{Each([1], \"item\", Each([2], \"item\", item))}",
        "{Each([1], \"data\", 0)}",
        "{Each([1], \"not-an-identifier\", 0)}",
    ] {
        assert_eq!(codes(&checked(source)), vec![Code::EachBinder]);
    }
    let mut expression = "1".to_owned();
    for i in 0..5 {
        expression = format!("Each([1], \"x{i}\", {expression})");
    }
    assert_eq!(
        codes(&checked(&format!("{{{expression}}}"))),
        vec![Code::EachNesting]
    );
    clean("{Each([1], \"item\", item + 1)}");
    assert_eq!(
        codes(&checked("{Each([\"x\"], \"item\", Round(item))}")),
        vec![Code::ArgType]
    );
}

#[test]
fn host_and_site_schema_types_feed_gradual_checking() {
    let mut context = TypeContext::default();
    context.hosts.insert(
        "input".into(),
        Type::Record(vec![("command".into(), Type::String)]),
    );
    context.sites.insert(
        "job".into(),
        Type::Envelope(
            SiteKind::Query,
            vec![(
                "value".into(),
                Type::Record(vec![("running".into(), Type::Boolean)]),
            )],
        ),
    );
    let result = prepare(b"export const job = Query(\"status\", {}, {})\n\n{Round(input.command)}\n\n{Round(job.value.running)}", Mode::Program, &context).unwrap();
    assert_eq!(codes(&result), vec![Code::ArgType, Code::ArgType]);
    assert_eq!(
        codes(&checked(
            "export const job = Query(\"status\", {}, {})\n\n{job.running}"
        )),
        vec![Code::EnvelopeMember]
    );
}

#[test]
fn gradual_types_refuse_only_definite_mismatches() {
    for source in [
        "{Round(data.value.x)}",
        "{Round(input.anything)}",
        "export let $n = null\n\n{Round($n)}",
        "{Round(true ? 1 : \"x\")}",
        "{Round((null ?? 2))}",
        "{1 + \"x\"}",
        "{null + \"x\"}",
        "{Count(null)}",
        "{Round(First([1, 2]))}",
        "{Count([{a: 1}].a)}",
        "export let $key = \"a\"\n\n{Round({a: 1}[$key])}",
        "{Round((true ? [1] : null)[0])}",
        "{Min(null)}",
    ] {
        clean(source);
    }
    for source in [
        "{null + 1}",
        "{false || \"default\"}",
        "{1 ? 2 : 3}",
        "{Round(Upper(\"x\"))}",
        "{Round({a: \"x\"}.a)}",
        "{Round([1][\"key\"])}",
        "{Contains(\"x\", 1)}",
        "{Join([[1]], \",\")}",
    ] {
        assert!(codes(&checked(source)).contains(&Code::ArgType), "{source}");
    }
}

#[test]
fn known_function_results_and_arity_are_checked() {
    for expression in [
        "Count([])",
        "Sum([1, null])",
        "Avg([1])",
        "Min([1])",
        "Max(1, 2)",
        "Filter([], null, \"==\", 1)",
        "Sort([])",
        "First([])",
        "Last([])",
        "Slice(\"abc\", 0, 1)",
        "Range(0, 2)",
        "Contains([], 1)",
        "Keys({a: 1})",
        "Values({a: 1})",
        "Join([1, null], \",\")",
        "IsError(1)",
        "Round(1)",
        "Floor(1)",
        "Ceil(1)",
        "Abs(1)",
        "Sqrt(1)",
        "Div(2, 1)",
        "Clamp(1, 0, 2)",
        "Number(\"1\")",
        "Text(true)",
        "Len(\"é\")",
        "Upper(\"a\")",
        "Lower(\"A\")",
        "Trim(\" x \")",
        "Lines(\"a\")",
        "StartsWith(\"a\", \"a\")",
        "EndsWith(\"a\", \"a\")",
        "Replace(\"a\", \"a\", \"b\")",
        "Split(\"a\", \",\")",
        "Truncate(\"abc\", 2)",
        "FormatNumber(1, null, true)",
        "FormatBytes(1)",
        "FormatDuration(1)",
        "FormatDate(1, \"date\", zone)",
        "FormatRelative(1, now)",
        "ParseDate(\"2026-10\")",
        "Parse($$tc(1), \"json\")",
        "Query(\"op\", {}, null)",
        "Watch(\"op\", {})",
        "PagedList(\"op\", {}, 50)",
        "Action([Compose(\"x\"), Open(\"https://example.com\")])",
    ] {
        clean(&format!("{{{expression}}}"));
    }
    for expression in [
        "Round()",
        "Each([])",
        "Div(1)",
        "Query(\"op\")",
        "Replace(\"a\", \"b\")",
    ] {
        assert_eq!(
            codes(&checked(&format!("{{{expression}}}"))),
            vec![Code::Arity]
        );
    }
    for expression in [
        "Round(Count([]))",
        "Round(Len(\"a\"))",
        "Count(Split(\"a\", \",\"))",
        "Len(FormatBytes(1))",
    ] {
        clean(&format!("{{{expression}}}"));
    }
    for expression in [
        "Round(Text(1))",
        "Count(Round(1))",
        "Len(Count([]))",
        "Sum([\"x\"])",
        "Action([1])",
        "Sum([1, \"x\"])",
        "Min(1, \"x\")",
        "Max([1, \"x\"])",
        "Round(Min([\"x\"]))",
        "Upper(null)",
        "Trim(null)",
        "Replace(null, \"a\", \"b\")",
        "Truncate(null, 2)",
        "LoadMore(Query(\"op\", {}, {}))",
    ] {
        assert_eq!(
            codes(&checked(&format!("{{{expression}}}"))),
            vec![Code::ArgType]
        );
    }
    let json = serde_json::to_value(&checked("{Round(1, 2, 3)}").diagnostics[0]).unwrap();
    assert_eq!(json["subject"], "Round");
    assert_eq!(
        json["signature"],
        "Round(x: number, digits?: number) -> number"
    );
    assert_eq!(
        codes(&checked(
            "export let $n = 1\n\n<Button action={Set($n, \"wrong\")}>Set</Button>"
        )),
        vec![Code::ArgType]
    );
}

#[test]
fn opaque_text_is_not_resolved_and_results_are_deterministic() {
    let source = "`{unknown}`\n\n```text\n{$state}\n```\n\n[link](https://example.com/missing)";
    clean(source);
    let source = "export const a = b\nexport const b = a\n";
    assert_eq!(
        serde_json::to_vec(&checked(source).diagnostics).unwrap(),
        serde_json::to_vec(&checked(source).diagnostics).unwrap()
    );
}
#[test]
fn parse_bound_refuses_before_syntax_work_when_shared_room_is_insufficient() {
    use crate::accounts::{Accounts, Refused, Resources};
    use crate::{AccountScope, SharedAccount};
    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    for fuel in [8_000_000, 8_000_000, 7_500_001] {
        let accounts = Accounts::new(AccountScope::Interface, chat.clone(), app.clone()).unwrap();
        let mut reservation = accounts
            .reserve(Resources {
                fuel,
                ..Resources::default()
            })
            .unwrap();
        reservation.spend(fuel, 0).unwrap();
    }
    let reply = Accounts::new(AccountScope::Reply, chat.clone(), app).unwrap();
    let mut calls = 0;
    let result = super::with_parse_account(&reply, 250_000, || {
        calls += 1;
        super::parse_reserved(b"x", Mode::Reply)
    });
    let Err(error) = result else {
        panic!("parse must be refused")
    };
    assert!(matches!(
        error,
        Refused::Aggregate(crate::AccountExhausted {
            scope: AccountScope::Chat
        })
    ));
    assert_eq!(calls, 0);
    assert_eq!(chat.usage().spent_fuel, 23_500_001);
}

#[test]
fn parse_charges_exactly_two_per_byte() {
    let accounts = crate::accounts::Accounts::isolated(crate::AccountScope::Reply).unwrap();
    super::parse_accounted(b"Hello\n", Mode::Reply, &accounts).unwrap();
    assert_eq!(accounts.chat_handle().usage().spent_fuel, 12);
    assert_eq!(accounts.chat_handle().usage().reserved_fuel, 0);
}

#[test]
fn oversized_source_only_charges_the_bounded_prefix() {
    for (mode, cap) in [(Mode::Program, 65_536), (Mode::Reply, 262_144)] {
        let accounts = crate::accounts::Accounts::isolated(crate::AccountScope::Reply).unwrap();
        let source = vec![b'x'; cap + 19];
        let parsed = super::parse_accounted(&source, mode, &accounts).unwrap();
        assert_eq!(parsed.document.parsed_bytes, cap);
        assert_eq!(accounts.chat_handle().usage().spent_fuel, 2 * cap as u64);
        assert_eq!(accounts.chat_handle().usage().reserved_fuel, 0);
    }
}

#[test]
fn parse_settlement_releases_unused_prefix_reservation() {
    let accounts = crate::accounts::Accounts::isolated(crate::AccountScope::Reply).unwrap();
    super::with_parse_account(&accounts, 10, || {
        assert_eq!(accounts.chat_handle().usage().reserved_fuel, 20);
        super::parse_reserved(b"x", Mode::Reply)
    })
    .unwrap();
    assert_eq!(accounts.chat_handle().usage().spent_fuel, 2);
    assert_eq!(accounts.chat_handle().usage().reserved_fuel, 0);
}

#[test]
fn raw_byte_cap_precedes_utf8_repair_and_parse_fuel_uses_bounded_input() {
    for (mode, cap) in [(Mode::Program, 65_536), (Mode::Reply, 262_144)] {
        for all_invalid in [false, true] {
            let mut source = vec![if all_invalid { 0xff } else { b'a' }; cap + 3];
            source[cap - 1] = 0xff;
            let accounts = crate::accounts::Accounts::isolated(crate::AccountScope::Reply).unwrap();
            let reserved_bytes = super::bounded_source_bytes(&source, cap);
            let parsed = super::with_parse_account(&accounts, reserved_bytes, || {
                assert_eq!(
                    accounts.chat_handle().usage().reserved_fuel,
                    2 * reserved_bytes as u64
                );
                super::parse_reserved(&source, mode)
            })
            .unwrap();
            assert_eq!(
                parsed.document.parsed_bytes,
                if all_invalid { cap * 3 } else { cap + 2 }
            );
            assert_eq!(parsed.accounted_bytes, cap);
            assert_eq!(reserved_bytes, cap);
            assert_eq!(accounts.chat_handle().usage().spent_fuel, 2 * cap as u64);
            assert_eq!(accounts.chat_handle().usage().reserved_fuel, 0);
        }
    }
    let accounts = crate::accounts::Accounts::isolated(crate::AccountScope::Reply).unwrap();
    super::parse_accounted(b"a\xffb", Mode::Reply, &accounts).unwrap();
    assert_eq!(accounts.chat_handle().usage().spent_fuel, 6);
}
#[test]
fn parse_preparation_propagates_settlement_refusal() {
    use crate::accounts::{Accounts, Refused, Resources};
    let accounts = Accounts::reply();
    let mut prior = accounts
        .reserve(Resources {
            fuel: 5_999_998,
            ..Resources::default()
        })
        .unwrap();
    prior.spend(5_999_998, 0).unwrap();
    let result =
        super::with_parse_account(&accounts, 1, || super::parse_reserved(b"xx", Mode::Reply));
    assert!(matches!(
        result,
        Err(Refused::Aggregate(crate::AccountExhausted {
            scope: crate::AccountScope::Reply
        }))
    ));
    assert_eq!(accounts.chat_handle().usage().spent_fuel, 6_000_000);
    assert_eq!(accounts.chat_handle().usage().reserved_fuel, 0);
}

#[test]
fn raw_source_cut_omits_only_complete_scalars_crossing_the_cap() {
    for (source, cap, prefix) in [
        (&b"ab\xc3\xa9c"[..], 3, &b"ab"[..]),
        (&b"ab\xe2\x82\xacc"[..], 4, &b"ab"[..]),
        (&b"ab\xf0\x9f\x98\x80c"[..], 5, &b"ab"[..]),
        (&b"ab\xc3z"[..], 3, &b"ab\xc3"[..]),
        (&b"ab\x80\x80c"[..], 3, &b"ab\x80"[..]),
        (&b"ab\xc3\xa9c"[..], 4, &b"ab\xc3\xa9"[..]),
    ] {
        assert_eq!(super::source_prefix(source, cap), prefix);
    }
    let mut source = vec![b'a'; 65_535];
    source.extend_from_slice("é".as_bytes());
    let parsed = parse(&source, Mode::Program).unwrap();
    assert_eq!(parsed.document.parsed_bytes, 65_535);
    assert_eq!(parsed.accounted_bytes, 65_535);
    assert!(!parsed
        .diagnostics
        .iter()
        .any(|d| d.code() == Code::Encoding));
}

#[test]
fn unsupported_mode_is_a_typed_refusal() {
    assert!(matches!(
        super::unsupported_mode::<()>(),
        Err(crate::accounts::Refused::UnsupportedMode)
    ));
    assert_eq!(
        super::mode_spec(Mode::Program).unwrap(),
        (crate::AccountScope::Interface, 65_536)
    );
    assert_eq!(
        super::mode_spec(Mode::Reply).unwrap(),
        (crate::AccountScope::Reply, 262_144)
    );
}
