//! Every §12.4 code produced by a real crate call, in one place.
//!
//! Each recipe asserts that it produces its own code, so a recipe cannot be
//! satisfied by another one's output; the union must then be the whole
//! inventory. The recipes follow the rules of the sections named beside them.

use std::collections::BTreeSet;

use cortexkit_rich_ui_syntax::Mode;
use serde_json::json;

use crate::actions::{Activation, Runtime};
use crate::data::{ColumnType, Format, Options};
use crate::eval::{self, Inputs};
use crate::prepare::{self, types::TypeContext};
use crate::{
    CheckContext, Diagnostic, DiagnosticCode as Code, Operation, Profile, ScalarString, SourceSpan,
    Value,
};

struct Coverage(BTreeSet<&'static str>);

impl Coverage {
    fn expect(&mut self, code: Code, produced: &[Diagnostic], recipe: &str) {
        assert!(
            produced.iter().any(|d| d.code() == code),
            "{code} not produced by {recipe:?}: {:?}",
            produced.iter().map(Diagnostic::code).collect::<Vec<_>>()
        );
        self.0.insert(code.as_str());
    }
}

/// A checker context with one `Query` and one `Watch` operation (§8.3, §8.5).
fn context(profile: Profile) -> CheckContext {
    let schema = |value| serde_json::from_value(value).unwrap();
    let mut context = CheckContext::new(profile);
    context.registry.0.insert(
        "status".into(),
        Operation::Query {
            arguments: schema(
                json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}),
            ),
            result: schema(json!({"type":"object"})),
        },
    );
    context.registry.0.insert(
        "events".into(),
        Operation::Watch {
            arguments: schema(json!({"type":"object"})),
            item: schema(json!({"type":"string"})),
            resume: true,
            replay: false,
        },
    );
    context
}

fn checked(source: &[u8], context: &CheckContext) -> Vec<Diagnostic> {
    crate::check::check(source, context).unwrap().diagnostics
}

fn evaluated(source: &str, inputs: &Inputs) -> Vec<Diagnostic> {
    let program = prepare::prepare(source.as_bytes(), Mode::Program, &TypeContext::default())
        .expect("preparation is within budget");
    eval::evaluate(&program, inputs).diagnostics
}

/// `name0 = first` and `name{k} = step` with P standing for `name{k-1}`.
fn doubling(name: &str, first: &str, step: &str, last: u32, body: &str) -> String {
    let mut source = format!("export const {name}0 = {first}\n");
    for k in 1..=last {
        let expression = step.replace("P", &format!("{name}{}", k - 1));
        source.push_str(&format!("export const {name}{k} = {expression}\n"));
    }
    format!("{source}\n{body}")
}

fn data(bytes: &[u8], format: Format, options: &Options) -> Vec<Diagnostic> {
    let span = SourceSpan {
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 2,
    };
    crate::data::prepare(bytes, format, options, span, &mut |_| Ok::<(), ()>(()))
        .unwrap()
        .diagnostics
}

#[test]
fn every_section_12_4_code_is_produced_by_a_real_call() {
    let mut covered = Coverage(BTreeSet::new());
    let show_ui = context(Profile::ShowUi);

    // §3 source and grammar, and §5.7 checks, through the program checker.
    let nested = format!("{{{}1{}}}", "(".repeat(66), ")".repeat(66));
    let declarations: String = (0..257)
        .map(|i| format!("export const n{i} = 1\n"))
        .collect();
    let mut each = "1".to_owned();
    for name in ["a", "b", "c", "d", "e"] {
        each = format!("Each([], \"{name}\", {each})");
    }
    let steps = vec!["Compose(\"x\")"; 17].join(", ");
    let sites: String = (0..17)
        .map(|i| format!("export const q{i} = Query(\"status\", {{id: \"a\"}}, {{}})\n"))
        .collect();
    let program_cases: Vec<(Code, String)> = vec![
        (Code::Syntax, "{ @ }".into()),
        (Code::UnterminatedString, "{\"unterminated}".into()),
        (Code::BadEscape, "{\"bad\\q\"}".into()),
        (Code::NumberLiteral, "{01}".into()),
        (Code::UnclosedTag, "<Badge>x".into()),
        (Code::MismatchedClose, "<Badge>x</Text>".into()),
        (
            Code::DuplicateProp,
            "<Badge tone=\"info\" tone=\"warning\" />".into(),
        ),
        (Code::DuplicateKey, "{{a: 1, a: 2}}".into()),
        (Code::Nonassoc, "{1 < 2 < 3}".into()),
        (Code::MixedCoalesce, "{true ?? false || true}".into()),
        (Code::DeclarationIncomplete, "export const a =\n".into()),
        (Code::Nesting, nested),
        (Code::LimitDeclarations, declarations),
        (Code::UnknownName, "{typo}".into()),
        (Code::UnknownState, "{$typo}".into()),
        (
            Code::DuplicateName,
            "export const x = 1\nexport const x = 2\n\n{x}".into(),
        ),
        (Code::ReservedName, "export const input = 1".into()),
        (Code::HostValueUnavailable, "{input}".into()),
        (Code::ReferenceLimit, "$$tc(1) ".repeat(65)),
        (
            Code::Cycle,
            "export const a = b\nexport const b = a\n\n{a}".into(),
        ),
        (Code::UnknownFunction, "{NoSuchFunction()}".into()),
        (Code::Arity, "{Round()}".into()),
        (Code::ArgType, "{Round(true)}".into()),
        (Code::LiteralRequired, "<Image image={File(data)} />".into()),
        (Code::EachBinder, "{Each([], \"Bad\", 1)}".into()),
        (Code::EachNesting, format!("{{{each}}}")),
        (Code::UnknownComponent, "<Gauge />".into()),
        (Code::UnknownProp, "<Badge variant=\"x\">x</Badge>".into()),
        (Code::MissingProp, "<TextContent />".into()),
        (Code::Enum, "<Badge tone=\"red\">x</Badge>".into()),
        (Code::PropType, "<TextContent text={1} />".into()),
        (Code::Placement, "text <Divider />".into()),
        (Code::Binding, "<Slider min={0} max={1} value={1} />".into()),
        (Code::ActionRequired, "<Button label=\"x\" />".into()),
        (Code::ActionPlacement, "{Compose(\"x\")}".into()),
        (
            Code::LimitSteps,
            format!("<Button label=\"x\" action={{Action([{steps}])}} />"),
        ),
        (
            Code::SitePlacement,
            "{Query(\"status\", {id: \"a\"}, {})}".into(),
        ),
        (Code::SiteLimit, sites),
        (
            Code::SiteArgs,
            "export const q = Query(\"status\", {}, {})\n\n{q.status}".into(),
        ),
        (
            Code::EnvelopeMember,
            "export const q = Query(\"status\", {id: \"a\"}, {})\n\n{q.running}".into(),
        ),
        (
            Code::RefreshRange,
            "export const q = Query(\"status\", {id: \"a\"}, {}, 1)\n\n{q.status}".into(),
        ),
        (
            Code::UnknownOperation,
            "export const q = Watch(\"missing\", {})\n\n{q.status}".into(),
        ),
        (
            Code::OperationKind,
            "export const q = Watch(\"status\", {id: \"a\"})\n\n{q.status}".into(),
        ),
        (Code::WriteNotAllowed, "{Run(\"x\")}".into()),
        (Code::Unused, "export const x = 1".into()),
    ];
    for (code, source) in &program_cases {
        covered.expect(*code, &checked(source.as_bytes(), &show_ui), source);
    }
    covered.expect(
        Code::SourceTooLarge,
        &checked(&vec![b'x'; 65_537], &show_ui),
        "65,537 bytes",
    );
    covered.expect(
        Code::Encoding,
        &checked(b"bad \xff encoding", &show_ui),
        "invalid UTF-8",
    );
    let mut newer = context(Profile::ShowUi);
    newer.execution_dialect = "2.0".into();
    covered.expect(
        Code::UnsupportedVersion,
        &checked(b"", &newer),
        "dialect 2.0",
    );
    covered.expect(
        Code::ReferenceUnavailable,
        &checked(b"$$tc(1)", &context(Profile::ToolRow)),
        "a reference in a tool row",
    );
    let mut tool_row = context(Profile::ToolRow);
    tool_row.catalog = crate::CatalogVersion::Tool1_1;
    covered.expect(
        Code::UnslottedRead,
        &checked(b"<Badge text={view.call.phase} />", &tool_row),
        "the view read outside a Slot in a tool row",
    );
    let reply = context(Profile::Reply);
    covered.expect(
        Code::ReplyForbidden,
        &checked(b"<Badge text={name} />", &reply),
        "a name in a reply",
    );
    covered.expect(
        Code::LimitTags,
        &checked("<Badge>x</Badge> ".repeat(65).as_bytes(), &reply),
        "65 reply tags",
    );

    // §4–§6 runtime errors, reported where an error value is drawn.
    let inputs = Inputs::new(Profile::ShowUi);
    for (code, source) in [
        (Code::Type, "{null * 2}"),
        (Code::Arg, "{2 ** 1025}"),
        (Code::DivZero, "{1 / 0}"),
        (Code::Overflow, "{99999999999999999999 + 1}"),
        (Code::Domain, "{2 ** 0.5}"),
        (Code::NumberParse, "{Number(\"1x\")}"),
        (Code::DateParse, "{ParseDate(\"2025\")}"),
        (
            Code::Scheme,
            "<Button label=\"Go\" action={Open(\"ftp://example.com\")} />",
        ),
    ] {
        covered.expect(code, &evaluated(source, &inputs), source);
    }

    // §9.4 aborts.
    let badge = r#"<Badge tone="info">x</Badge>"#;
    let string = r#""abcdefgh""#;
    for (code, source) in [
        (
            Code::LimitString,
            doubling("s", string, "P + P", 14, "{s14}"),
        ),
        (Code::LimitList, "{Count(Range(0, 10001))}".into()),
        (
            Code::LimitRecord,
            format!(
                "{{Count(Keys({{{}}}))}}",
                (0..1_025)
                    .map(|i| format!("a{i}: 0"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        (
            Code::LimitDepth,
            format!("{{Count({}{})}}", "[".repeat(33), "]".repeat(33)),
        ),
        (
            Code::LimitValue,
            doubling("t", badge, "[P, P]", 20, "{Count(t20)}"),
        ),
        (
            Code::LimitAlloc,
            doubling(
                "s",
                string,
                "P + P",
                6,
                r#"{Count(Each(Range(0, 10000), "i", Len(s6 + "x")))}"#,
            ),
        ),
        (
            Code::LimitNodes,
            doubling("t", badge, "[P, P]", 10, "{t10}"),
        ),
        (
            Code::LimitText,
            doubling("s", string, "P + P", 13, "{[s13, s13, s13, s13, s13]}"),
        ),
    ] {
        covered.expect(
            code,
            &evaluated(&source, &inputs),
            &source[..source.len().min(40)],
        );
    }
    let mut starved = Inputs::new(Profile::ShowUi);
    starved.budget = 5;
    covered.expect(Code::Fuel, &evaluated("{1 + 2 + 3}", &starved), "budget 5");
    // 250 Tables in rows form need 102,500 page visits (§9.4 reservation).
    let record = format!(
        "{{{}}}",
        (0..8)
            .map(|i| format!("c{i}: {i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let columns = (0..8)
        .map(|i| format!("\"c{i}\""))
        .collect::<Vec<_>>()
        .join(",");
    let tables = format!(
        "export const rows = [{}]\n\n{}",
        vec![record; 50].join(","),
        vec![format!("<Table columns={{[{columns}]}} data={{rows}} displayRows={{50}} />"); 250]
            .join("\n\n")
    );
    let trial = crate::check::trial::check_program(
        tables.as_bytes(),
        &CheckContext::new(Profile::ShowUi),
        &Inputs::new(Profile::ShowUi),
    )
    .unwrap();
    covered.expect(Code::LimitLayout, trial.diagnostics(), "250 tables");

    // §7.3: a Set whose value has the wrong type for its state.
    let program = prepare::prepare(
        b"export let $b = 7\n\n<Button label=\"Go\" action={Set($b, input.bad)} />",
        Mode::Program,
        &TypeContext::default(),
    )
    .unwrap();
    let mut tool = Inputs::new(Profile::ToolPage);
    tool.input = Value::record(vec![(
        ScalarString::new("bad"),
        Value::string("wrong").unwrap(),
    )])
    .unwrap();
    let evaluation = eval::evaluate(&program, &tool);
    let mut runtime = Runtime::new(42, eval::initial_state(&program));
    runtime.commit(1, evaluation.controls.clone());
    let control = evaluation.controls.keys().next().unwrap().clone();
    let result = runtime
        .activate(&Activation {
            instance_epoch: 42,
            revision: 1,
            control,
            id: 1,
        })
        .unwrap();
    covered.expect(Code::StateType, &result.diagnostics, "Set($b, input.bad)");

    // §10.3 and §10.4 data codes.
    let typed = Options {
        columns: vec![
            ("n".into(), ColumnType::Number),
            ("missing".into(), ColumnType::Boolean),
        ],
        ..Options::default()
    };
    let plain = Options::default();
    let large = format!("[\"{}\"]", "x".repeat(66_000));
    for (code, bytes, format, options) in [
        (
            Code::DataEncoding,
            &b"[\"a\",\xFF]"[..],
            Format::Json,
            &plain,
        ),
        (Code::JsonSyntax, b"[\"a\" \"b\"]", Format::Json, &plain),
        (
            Code::JsonDuplicateKey,
            b"{\"a\":1,\"a\":2}",
            Format::Json,
            &plain,
        ),
        (Code::CsvSyntax, b"a\n\"b\"c\n", Format::Csv, &plain),
        (Code::DataLimit, large.as_bytes(), Format::Json, &plain),
        (
            Code::NumberRounded,
            b"[1.0000000000000000001]",
            Format::Json,
            &plain,
        ),
        (Code::NumberRange, b"[1e20]", Format::Json, &plain),
        (Code::JsonSurrogate, b"[\"a\\ud800\"]", Format::Json, &plain),
        (Code::CellType, b"[{\"n\":\"a\"}]", Format::Json, &typed),
        (Code::CsvHeaderRenamed, b"a,a\n1,2\n", Format::Csv, &plain),
        (Code::CsvShortRow, b"a,b\n1\n", Format::Csv, &plain),
        (Code::CsvLongRow, b"a\n1,2\n", Format::Csv, &plain),
        (Code::CsvUnknownColumn, b"n\n1\n", Format::Csv, &typed),
    ] {
        covered.expect(
            code,
            &data(bytes, format, options),
            &String::from_utf8_lossy(&bytes[..bytes.len().min(40)]),
        );
    }

    // Catalog renderer caps (§12.4 E_RENDER_LIMIT): nine evaluated series.
    covered.expect(
        Code::RenderLimit,
        &evaluated(
            "export const ns = Each([1, 2, 3, 4, 5, 6, 7, 8, 9], \"v\", <Series category=\"a\" values={[1]} />)\n\n<Chart kind=\"line\" labels={[\"a\"]} series={ns} />",
            &inputs,
        ),
        "nine evaluated series",
    );

    let missing: Vec<_> = Code::ALL
        .iter()
        .filter(|code| !covered.0.contains(code.as_str()))
        .collect();
    assert!(missing.is_empty(), "codes without a producer: {missing:?}");
    assert_eq!(covered.0.len(), Code::ALL.len());
}
