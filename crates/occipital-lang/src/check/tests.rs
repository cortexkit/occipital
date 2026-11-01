use super::*;
use crate::{DiagnosticSpan, HostProjection, Operation, Schema, ToolInputs};
use serde_json::json;

fn schema(value: Json) -> Schema {
    serde_json::from_value(value).unwrap()
}

fn context(profile: Profile) -> CheckContext {
    let mut context = CheckContext::new(profile);
    context.registry.0.insert("status".into(), Operation::Query {
        arguments: schema(json!({"type":"object", "properties":{"id":{"type":"string"}}, "required":["id"]})),
        result: schema(json!({"type":"object", "properties":{"running":{"type":"boolean"}}, "required":["running"]})),
    });
    context.registry.0.insert(
        "events".into(),
        Operation::Watch {
            arguments: schema(json!({"type":"object"})),
            item: schema(json!({"type":"string"})),
            resume: true,
            replay: false,
        },
    );
    context.registry.0.insert("lines".into(), Operation::Paged {
        arguments: schema(json!({"type":"object"})),
        page: schema(json!({"type":"object", "properties":{"items":{"type":"array","items":{"type":"string"}}, "next":{"type":"string","nullable":true},"total":{"type":"number"}},"required":["items","next"]})),
        item_key: None,
    });
    context
}

/// A context for a tool template naming the tool catalog 1.1.
fn tool(profile: Profile) -> CheckContext {
    let mut context = context(profile);
    context.catalog = crate::CatalogVersion::Tool1_1;
    context
}

fn codes(source: &str, context: &CheckContext) -> Vec<Code> {
    check(source.as_bytes(), context)
        .unwrap()
        .diagnostics
        .iter()
        .map(Diagnostic::code)
        .collect()
}
fn assert_code(source: &str, code: Code) {
    let result = check(source.as_bytes(), &context(Profile::ShowUi)).unwrap();
    assert!(
        result.diagnostics.iter().any(|d| d.code() == code),
        "{source}: {:?}",
        result.diagnostics
    );
    for d in &result.diagnostics {
        assert!(matches!(d.span(), Some(DiagnosticSpan::Source(_))), "{d:?}");
        assert_eq!(d.phase(), DiagnosticPhase::Check);
    }
}
fn clean(source: &str, profile: Profile) {
    clean_in(source, &context(profile));
}
fn clean_in(source: &str, context: &CheckContext) {
    let diagnostics = check(source.as_bytes(), context).unwrap().diagnostics;
    assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
}

/// A tool row is measured once and never resized, so whatever changes while
/// it is on screen (the call's view, the time) must sit in a Slot's fixed box,
/// whether it is read directly or through declarations.
#[test]
fn a_tool_row_reads_the_view_and_the_time_only_inside_a_slot() {
    let row = tool(Profile::ToolRow);
    let unslotted = |source: &str| {
        codes(source, &row)
            .iter()
            .filter(|code| **code == Code::UnslottedRead)
            .count()
    };
    assert_eq!(unslotted("<Badge text={view.call.phase} />"), 1);
    assert_eq!(unslotted("<Card>{now}</Card>"), 1);
    assert_eq!(
        unslotted("export const phase = view.call.phase\nexport const label = \"Phase \" + phase\n\n<Badge text={label} />"),
        1,
        "through a chain of declarations, reported once where it is drawn"
    );
    assert_eq!(
        unslotted("{view.call.phase == \"running\" ? <Badge text=\"Running\" /> : null}"),
        1,
        "a choice between parts changes the row's height too"
    );
    // A Slot is a block, so it stands on its own lines.
    clean_in(
        "<Slot width=\"short\">\n<Badge text={view.call.phase} />\n</Slot>",
        &row,
    );
    clean_in(
        "export const phase = view.call.phase\n\n<Slot width=\"medium\" lines={2}>\n<Badge text={phase} />\n</Slot>",
        &row,
    );
    clean_in(
        "export const status = <Slot width=\"long\" needs={view.record}><Badge text=\"Done\" /></Slot>\n\n<Card direction=\"row\">\n{status}\n</Card>",
        &row,
    );
    // The input never changes once the row is drawn, and a page is measured
    // again each time it opens.
    clean_in("<Badge text={input.command} />", &row);
    clean_in("<Badge text={view.call.phase} />", &tool(Profile::ToolPage));
    // A row written for catalog 1.0 has no Slot to put things in, so the rule
    // doesn't apply to it.
    clean("<Badge text={view.call.phase} />", Profile::ToolRow);
}

/// Slot belongs to the tool catalog only: replies and show_ui programs are
/// checked against catalog 1.0, which has no Slot.
#[test]
fn slot_is_a_tool_template_component() {
    assert_code("<Slot width=\"short\">x</Slot>", Code::UnknownComponent);
    assert!(
        codes("<Slot width=\"short\">x</Slot>", &context(Profile::ToolRow))
            .contains(&Code::UnknownComponent)
    );
    let row = tool(Profile::ToolRow);
    let wide = codes("<Slot width=\"wide\">x</Slot>", &row);
    assert!(wide.contains(&Code::Enum), "{wide:?}");
    let tall = codes("<Slot width=\"short\" lines={4}>x</Slot>", &row);
    assert!(!tall.is_empty(), "a Slot is 1 to 3 lines");
    let no_width = codes("<Slot>x</Slot>", &row);
    assert!(no_width.contains(&Code::MissingProp), "{no_width:?}");
}

#[test]
fn unknown_components_props_and_required_props() {
    assert_code("<Gauge />", Code::UnknownComponent);
    assert_code("<Badge variant=\"primary\">x</Badge>", Code::UnknownProp);
    assert_code(
        "<LinkCard href=\"https://example.com\" />",
        Code::MissingProp,
    );
    assert_code("<Callout />", Code::MissingProp);
    assert_code("<Table />", Code::MissingProp);
    assert_code("<Chart kind=\"bar\" data={[]} />", Code::MissingProp);
}

#[test]
fn literal_enums_and_shared_vocabularies() {
    for source in [
        "<Badge tone=\"primary\">x</Badge>",
        "<Image image={File(\"x\")} aspect=\"3:2\" />",
        "<Card gap=\"huge\" />",
        "<Chart kind=\"pie\" data={[]} x=\"x\" y=\"y\" />",
    ] {
        assert_code(source, Code::Enum);
    }
    clean("<Styled tone=\"muted\">ok</Styled>", Profile::Reply);
    clean("<Card appearance=\"clear\" gap=\"s\" />", Profile::ShowUi);
}

#[test]
fn prop_types_cover_nested_lists_nodes_rows_handles_and_envelopes() {
    for source in [
        "<TextContent text={42} />",
        "<Table data={[1]} />",
        "<Table data={[]} columns={[true]} />",
        "<Table columns={[<Series category=\"x\" values={[1]} />]} />",
        "<Image image=\"path.png\" />",
        "<Map places={File(\"x\")} />",
        "<Buttons buttons={[<Badge>x</Badge>]} />",
        "<FormControl label=\"x\" input={<Badge>x</Badge>} />",
        "export const q = Query(\"status\", {id: \"a\"}, {})\n\n<LiveOutput source={q} />",
    ] {
        assert_code(source, Code::PropType);
    }
    clean("<Image image={File(\"x.png\")} />", Profile::ShowUi);
    clean(
        "<Table columns={[<Col label=\"x\" data={[1, true, \"a\"]} />]} />",
        Profile::ShowUi,
    );
    clean("<FormControl label=\"Width\" input={<Input value={$width} />} />\nexport let $width = \"4\"", Profile::ShowUi);
}

#[test]
fn required_action_is_not_a_generic_missing_prop() {
    let result = codes("<Button label=\"Go\" />", &context(Profile::ShowUi));
    assert_eq!(result, vec![Code::ActionRequired]);
    assert_code("<Button label=\"Go\" action={42} />", Code::PropType);
    clean("<Button label=\"Go\" action={null} />", Profile::ShowUi);
}

#[test]
fn action_placement_and_step_cap() {
    assert_code("<Button label=\"x\" action={Action([1])} />", Code::ArgType);
    assert_code(
        "<Button label=\"x\" action={Action([Compose(\"x\"), 1])} />",
        Code::ArgType,
    );
    assert_code("{Compose(\"x\")}", Code::ActionPlacement);
    assert_code(
        "<TextContent text={Compose(\"x\")} />",
        Code::ActionPlacement,
    );
    let steps = std::iter::repeat_n("Compose(\"x\")", 17)
        .collect::<Vec<_>>()
        .join(",");
    assert_code(
        &format!("<Button label=\"x\" action={{Action([{steps}])}} />"),
        Code::LimitSteps,
    );
    let steps = std::iter::repeat_n("Compose(\"x\")", 16)
        .collect::<Vec<_>>()
        .join(",");
    clean(
        &format!("<Button label=\"x\" action={{Action([{steps}])}} />"),
        Profile::ShowUi,
    );
}

#[test]
fn binding_requires_bare_state_of_the_declared_kind() {
    for source in [
        "export let $n = \"1\"\n\n<Slider min={0} max={2} value={$n} />",
        "export let $n = 1\n\n<Slider min={0} max={2} value={($n)} />",
        "export let $n = 1\n\n<Slider min={0} max={2} value={$n + 0} />",
        "export let $n = 1\n\n<Toggle label=\"x\" value={$n} />",
        "export let $n = 1\n\n<Input value={$n} />",
        "export let $n = null\n\n<Input type=\"number\" value={$n} />",
    ] {
        assert_code(source, Code::Binding);
    }
    clean(
        "export let $n = 1\n\n<Input type=\"number\" value={$n} step={1} />",
        Profile::ShowUi,
    );
    clean("export let $names = []\n\n<Select multiple value={$names} items={[<SelectItem value=\"x\" label=\"X\" />]} />", Profile::ShowUi);
    assert_code(
        "export let $n = 1\n\n<Input type=\"number\" value={$n} lines={2} />",
        Code::PropType,
    );
    assert_code(
        "export let $n = 1\nexport const kind = \"number\"\n\n<Input type={kind} value={$n} />",
        Code::LiteralRequired,
    );
}

#[test]
fn flow_required_parents_and_child_kinds() {
    for source in [
        "text <Table data={[]} />",
        "<TextContent text=\"x\">content</TextContent>",
        "<Col label=\"x\" data={[]} />",
        "<ListItem title=\"x\" />",
        "<Badge>*not plain*</Badge>",
        "<Badge><Styled>x</Styled></Badge>",
        "<Callout>\n<Card />\n</Callout>",
        "<List>not a ListItem</List>",
        "<Styled>\n# Heading\n</Styled>",
    ] {
        assert_code(source, Code::Placement);
    }
    clean("<Badge>x</Badge>", Profile::ShowUi);
    clean("<Styled>*yes* `code`</Styled>", Profile::ShowUi);
    clean(
        "<Callout>\n# Heading\nSome <Badge>good</Badge> prose.\n</Callout>",
        Profile::ShowUi,
    );
    clean("<List>\n<ListItem title=\"A\" />\n</List>", Profile::ShowUi);
    assert_code(
        "<Tabs>\n<TabItem label=\"one\">x</TabItem>\n</Tabs>",
        Code::PropType,
    );
    clean("<Tabs>\n<TabItem label=\"one\">\nx\n</TabItem>\n<TabItem label=\"two\">\ny\n</TabItem>\n</Tabs>", Profile::ShowUi);
}

#[test]
fn content_alternatives_and_data_forms_are_exclusive() {
    assert_code("<Badge text=\"x\">y</Badge>", Code::PropType);
    assert_code("<Callout description=\"x\">y</Callout>", Code::PropType);
    assert_code(
        "<Table data={[]} columns={[<Col label=\"x\" data={[]} />]} />",
        Code::PropType,
    );
    assert_code("<Chart kind=\"bar\" data={[]} x=\"x\" y=\"y\" labels={[]} series={[<Series category=\"x\" values={[]} />]} />", Code::PropType);
    clean(
        "<Table data={[]} columns={[\"x\", {key: \"y\", label: \"Y\"}]} />",
        Profile::ShowUi,
    );
    assert_code(
        "<Table data={[]} columns={[{key: \"y\", label: 1}]} />",
        Code::PropType,
    );
}

#[test]
fn component_values_get_parent_checks_where_they_are_drawn() {
    clean(
        "export const col = <Col label=\"x\" data={[1]} />\n\n<Table columns={[col]} />",
        Profile::ShowUi,
    );
    assert_code(
        "export const item = <ListItem title=\"x\" />\n\n{item}",
        Code::Placement,
    );
    clean(
        "export const item = <ListItem title=\"x\" />\n\n<List>\n{item}\n</List>",
        Profile::ShowUi,
    );
}

#[test]
fn literal_and_statically_known_bounds_use_prop_type() {
    for source in [
        "<Table data={[]} displayRows={0} />", "<TextContent text=\"x\" maxLines={101} />",
        "export let $n = 0\n\n<Slider min={0} max={1} step={0} value={$n} />",
        "export const count = 1 + 50\n\n<Table data={[]} displayRows={count} />",
        "<Table data={[]} columns={[\"a\",\"b\",\"c\",\"d\",\"e\",\"f\",\"g\",\"h\",\"i\"]} />",
        "export const cols = [\"a\",\"b\",\"c\",\"d\",\"e\",\"f\",\"g\",\"h\",\"i\"]\n\n<Table data={[]} columns={cols} />",
    ] { assert_code(source, Code::PropType); }
    let nine = std::iter::repeat_n("<Series category=\"x\" values={[]} />", 9)
        .collect::<Vec<_>>()
        .join(",");
    assert_code(
        &format!("<Chart kind=\"bar\" labels={{[]}} series={{[{nine}]}} />"),
        Code::PropType,
    );
    clean("<Progress value={-200} />", Profile::ShowUi);
    clean("<Table data={[]} displayRows={50} />", Profile::ShowUi);
    assert!(codes(
        "<Table data={[]} displayRows={11} />",
        &context(Profile::Reply)
    )
    .contains(&Code::PropType));
}

#[test]
fn computed_scalar_range_uses_evaluate_prop_type() {
    let zero = crate::Value::number(Decimal::ONE.checked_sub(Decimal::ONE).unwrap());
    let diagnostic = catalog::evaluated_bounds("Slider", "step", Profile::ShowUi, &zero).unwrap();
    assert_eq!(diagnostic.code(), Code::PropType);
    assert_eq!(diagnostic.phase(), DiagnosticPhase::Evaluate);
    assert!(catalog::evaluated_bounds(
        "Slider",
        "step",
        Profile::ShowUi,
        &crate::Value::number(Decimal::ONE)
    )
    .is_none());
}

#[test]
fn computed_items_overflow_uses_evaluate_render_limit() {
    let props = crate::Value::record(vec![
        (
            crate::ScalarString::new("category"),
            crate::Value::string("x").unwrap(),
        ),
        (
            crate::ScalarString::new("values"),
            crate::Value::list(vec![]).unwrap(),
        ),
    ])
    .unwrap();
    let crate::ValueKind::Record(props) = props.kind() else {
        unreachable!()
    };
    let series = crate::Value::component("Series", props.clone(), vec![]).unwrap();
    let computed = crate::Value::list(vec![series.clone(); 9]).unwrap();
    let diagnostic =
        catalog::evaluated_bounds("Chart", "series", Profile::ShowUi, &computed).unwrap();
    assert_eq!(diagnostic.code(), Code::RenderLimit);
    assert_eq!(diagnostic.phase(), DiagnosticPhase::Evaluate);
    let valid = crate::Value::list(vec![series; 8]).unwrap();
    assert!(catalog::evaluated_bounds("Chart", "series", Profile::ShowUi, &valid).is_none());
}

#[test]
fn host_availability_and_reference_policy_match_every_profile() {
    for (profile, available) in [
        (Profile::ShowUi, ["data", "now", "zone"].as_slice()),
        (
            Profile::ToolRow,
            ["input", "view", "now", "zone"].as_slice(),
        ),
        (
            Profile::ToolPage,
            ["input", "view", "now", "zone"].as_slice(),
        ),
    ] {
        for host in ["data", "input", "view", "now", "zone"] {
            let found = codes(&format!("{{{host}}}"), &context(profile));
            assert_eq!(
                found.contains(&Code::HostValueUnavailable),
                !available.contains(&host),
                "{profile:?} {host}: {found:?}"
            );
        }
        let found = codes("$$tc(1)", &context(profile));
        assert_eq!(
            found.contains(&Code::ReferenceUnavailable),
            profile != Profile::ShowUi
        );
    }
    clean("$$tc(1)", Profile::Reply);
    for profile in [Profile::ToolRow, Profile::ToolPage] {
        assert!(
            codes("<Image image={File(\"x\")} />", &context(profile)).contains(&Code::Placement)
        );
    }
}

#[test]
fn sites_need_whole_named_declarations_and_an_admitting_profile() {
    assert_code("{Query(\"status\", {id: \"a\"}, {})}", Code::SitePlacement);
    assert_code(
        "export const q = Count([Query(\"status\", {id: \"a\"}, {})])\n\n{q}",
        Code::SitePlacement,
    );
    let source = "export const q = Query(\"status\", {id: \"a\"}, {})\n\n{q.status}";
    assert!(codes(source, &context(Profile::ToolRow)).contains(&Code::SitePlacement));
    clean(source, Profile::ShowUi);
    clean(source, Profile::ToolPage);
}

#[test]
fn site_total_watch_and_page_caps_have_boundaries() {
    for (constructor, operation, limit) in [
        ("Query", "status", 16),
        ("Watch", "events", 2),
        ("PagedList", "lines", 2),
    ] {
        let source = |n| {
            (0..n)
                .map(|i| {
                    format!(
                        "export const q{i} = {constructor}(\"{operation}\", {{id: \"a\"}}{})\n",
                        if constructor == "Query" { ", {}" } else { "" }
                    )
                })
                .collect::<String>()
        };
        assert!(!codes(&source(limit), &context(Profile::ShowUi)).contains(&Code::SiteLimit));
        assert!(codes(&source(limit + 1), &context(Profile::ShowUi)).contains(&Code::SiteLimit));
    }
}

#[test]
fn operation_membership_kind_literal_and_refresh_checks() {
    assert_code(
        "export const q = Query(\"missing\", {}, {})\n\n{q.status}",
        Code::UnknownOperation,
    );
    assert_code(
        "export const q = Query(\"events\", {}, {})\n\n{q.status}",
        Code::OperationKind,
    );
    assert_code(
        "export const op = \"status\"\nexport const q = Query(op, {}, {})\n\n{q.status}",
        Code::LiteralRequired,
    );
    assert_code(
        "export const q = Query(\"status\", {id: \"a\"}, {}, 1)\n\n{q.status}",
        Code::RefreshRange,
    );
    clean(
        "export const n = 1\nexport const q = Query(\"status\", {id: \"a\"}, {}, n)\n\n{q.status}",
        Profile::ShowUi,
    );
    assert_code(
        "export const p = PagedList(\"lines\", {}, 201)\n\n{p.status}",
        Code::SiteArgs,
    );
    clean(
        "export const q = Query(\"status\", {id: \"a\"}, {}, 2)\n\n{q.status}",
        Profile::ShowUi,
    );
    clean(
        "export const q = Query(\"status\", {id: \"a\"}, {}, 3600)\n\n{q.status}",
        Profile::ShowUi,
    );
}

#[test]
fn literal_refresh_above_3600_is_refused() {
    let source = "export const q = Query(\"status\", {id: \"a\"}, {}, 3601)\n\n{q.status}";
    assert_eq!(
        codes(source, &context(Profile::ShowUi)),
        vec![Code::RefreshRange]
    );
}

#[test]
fn paged_list_accepts_exactly_200_items_per_page() {
    clean(
        "export const p = PagedList(\"lines\", {}, 200)\n\n{p.status}",
        Profile::ShowUi,
    );
}

#[test]
fn site_args_must_be_data_schema_complete_and_at_most_8k_canonical_bytes() {
    for source in [
        "export const q = Query(\"status\", {}, {})\n\n{q.status}",
        "export const q = Query(\"status\", {id: 42}, {})\n\n{q.status}",
        "export const q = Query(\"status\", {id: File(\"x\")}, {})\n\n{q.status}",
        "export const q = Query(\"status\", {id: \"x\"}, $$tc(1))\n\n{q.status}",
    ] {
        assert_code(source, Code::SiteArgs);
    }
    for (length, invalid) in [(8183, false), (8184, true)] {
        let source = format!(
            "export const q = Query(\"status\", {{id: \"{}\"}}, {{}})\n\n{{q.status}}",
            "x".repeat(length)
        );
        assert_eq!(
            codes(&source, &context(Profile::ShowUi)).contains(&Code::SiteArgs),
            invalid
        );
    }
}

#[test]
fn registry_schemas_type_query_watch_and_page_envelopes() {
    assert_code(
        "export const q = Query(\"status\", {id: \"a\"}, {})\n\n{Round(q.value.running)}",
        Code::ArgType,
    );
    assert_code(
        "export const q = Query(\"status\", {id: \"a\"}, {})\n\n{q.running}",
        Code::EnvelopeMember,
    );
    assert_code(
        "export const w = Watch(\"events\", {})\n\n{Round(First(w.value))}",
        Code::ArgType,
    );
    assert_code(
        "export const p = PagedList(\"lines\", {})\n\n{Round(First(p.items))}",
        Code::ArgType,
    );
    clean("export const p = PagedList(\"lines\", {})\n\n<List more={p}>\n{Each(p.items, \"line\", <ListItem title={line} />)}\n</List>", Profile::ShowUi);
}

#[test]
fn reserved_writes_are_rejected_in_every_profile_without_authority() {
    for profile in [
        Profile::Reply,
        Profile::ShowUi,
        Profile::ToolRow,
        Profile::ToolPage,
    ] {
        for call in ["Mutation(\"write\", {})", "Run(\"write\")"] {
            let source = format!("<Badge text={{{call}}} />");
            let found = codes(&source, &context(profile));
            assert!(
                found.contains(&Code::WriteNotAllowed),
                "{profile:?}: {source}"
            );
            assert_eq!(
                found
                    .iter()
                    .filter(|code| **code == Code::WriteNotAllowed)
                    .count(),
                1,
                "{profile:?}: {source}"
            );
        }
    }
}

#[test]
fn reserved_writes_are_refused_when_name_errors_skip_type_inference() {
    for profile in [Profile::ShowUi, Profile::ToolRow, Profile::ToolPage] {
        for call in ["Mutation(\"write\", {})", "Run(\"write\")"] {
            let source = format!("{{missing}}\n\n{{{call}}}");
            let result = check(source.as_bytes(), &context(profile)).unwrap();
            assert!(
                result.node_types.is_empty(),
                "type inference must not have run"
            );
            assert_eq!(
                result
                    .diagnostics
                    .iter()
                    .map(Diagnostic::code)
                    .collect::<Vec<_>>(),
                vec![Code::UnknownName, Code::WriteNotAllowed],
                "{profile:?}: {source}"
            );
        }
    }
}

#[test]
fn reply_value_grammar_accepts_only_section_3_7_forms() {
    for source in [
        "<Table data={[{x: -1, y: null, z: [true, \"s\"]}]} />",
        "<Table data={Parse($$tc(1), \"json\", {columns: {x: \"number\"}})} />",
        "<Image image={Artifact(\"art_1\")} />",
        "<ImageGroup images={[Artifact(\"art_1\"), $$tc(1)]} />",
    ] {
        clean(source, Profile::Reply);
    }
    for source in [
        "<Button label=\"x\" />",
        "<Badge text={name} />",
        "<Badge text={$state} />",
        "<Badge text={1 + 2} />",
        "<Badge text={(1)} />",
        "<Badge text={Upper(\"x\")} />",
        "<Table columns={[<Col label=\"x\" data={[]} />]} />",
        "<Table data={Parse(data, \"json\")} />",
        "<Button label=\"x\" action={Compose(\"x\")} />",
    ] {
        assert!(
            codes(source, &context(Profile::Reply)).contains(&Code::ReplyForbidden),
            "{source}"
        );
    }
}

#[test]
fn reply_example_16_11_keeps_exact_two_diagnostics() {
    let result = check(b"export const total = 12\n\nThe total is {total}. <Badge tone=\"success\">Stored</Badge> <Badge tone={total > 0 ? \"success\" : \"neutral\"}>Computed</Badge> <Gauge value={12} />", &context(Profile::Reply)).unwrap();
    let found: Vec<_> = result.diagnostics.iter().map(Diagnostic::code).collect();
    assert_eq!(found, vec![Code::ReplyForbidden, Code::UnknownComponent]);
    assert_eq!(
        serde_json::to_value(&result.diagnostics[1]).unwrap()["subject"],
        "Gauge"
    );
}

#[test]
fn host_projection_schemas_validate_values_and_feed_inference() {
    let mut context = context(Profile::ToolPage);
    context.tool = Some(ToolInputs {
        input: HostProjection {
            value: json!({"command":"pwd"}),
            schema: schema(
                json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
            ),
        },
        view: HostProjection {
            value: json!({"phase":"running","exitCode":null}),
            schema: schema(
                json!({"type":"object","properties":{"phase":{"type":"string","enum":["running","finished"]},"exitCode":{"type":"number","nullable":true}},"required":["phase","exitCode"]}),
            ),
        },
        truncated: false,
        truncated_fields: vec![],
    });
    assert_eq!(
        codes("<TextContent text={input.command} />", &context),
        vec![]
    );
    assert!(codes("{Round(input.command)}", &context).contains(&Code::ArgType));
    context.tool.as_mut().unwrap().view.value = json!({"phase":"unknown","exitCode":null});
    let diagnostics = check(b"", &context).unwrap().diagnostics;
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code(), Code::ArgType);
    assert_eq!(
        serde_json::to_value(&diagnostics[0]).unwrap()["subject"],
        "view"
    );
}

#[test]
fn unused_follows_body_reachability_including_state_and_actions() {
    assert_eq!(
        codes(
            "export const a = 1\nexport const b = a\n",
            &context(Profile::ShowUi)
        ),
        vec![Code::Unused, Code::Unused]
    );
    clean(
        "export const a = 1\nexport const b = a\n\n{b}",
        Profile::ShowUi,
    );
    clean(
        "export let $a = 1\n\n<Button label=\"Set\" action={Set($a, 2)} />",
        Profile::ShowUi,
    );
}

#[test]
fn errors_suppress_unused_warnings_after_declarations_are_resolved() {
    for (source, expected) in [
        ("export const unused = 1\n\n{missing}", Code::UnknownName),
        (
            "export const unused = 1\n\n<Unknown />",
            Code::UnknownComponent,
        ),
    ] {
        let result = check(source.as_bytes(), &context(Profile::ShowUi)).unwrap();
        assert_eq!(result.declarations.len(), 1);
        assert_eq!(result.declarations[0].name, "unused");
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity() == crate::Severity::Error));
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code() == Code::Unused),
            "{source}: {:?}",
            result.diagnostics
        );
        assert_eq!(
            result
                .diagnostics
                .iter()
                .map(Diagnostic::code)
                .collect::<Vec<_>>(),
            vec![expected]
        );
    }
}

#[test]
fn unsupported_execution_dialect_has_a_check_diagnostic() {
    let mut context = context(Profile::ShowUi);
    context.execution_dialect = "2.0".into();
    assert_eq!(codes("", &context), vec![Code::UnsupportedVersion]);
}

#[test]
fn every_section_12_4_check_code_is_emitted_by_a_real_check() {
    let ordinary = [
        (Code::UnknownName, "{typo}"),
        (Code::UnknownState, "{$typo}"),
        (
            Code::DuplicateName,
            "export const x = 1\nexport const x = 2",
        ),
        (Code::ReservedName, "export const input = 1"),
        (Code::HostValueUnavailable, "{input}"),
        (Code::Cycle, "export const a = b\nexport const b = a\n\n{a}"),
        (Code::UnknownFunction, "{NoSuchFunction()}"),
        (Code::Arity, "{Round()}"),
        (Code::ArgType, "{Round(true)}"),
        (Code::LiteralRequired, "<Image image={File(data)} />"),
        (Code::EachBinder, "{Each([], \"Bad\", 1)}"),
        (Code::UnknownComponent, "<Gauge />"),
        (Code::UnknownProp, "<Badge variant=\"x\">x</Badge>"),
        (Code::MissingProp, "<TextContent />"),
        (Code::Enum, "<Badge tone=\"red\">x</Badge>"),
        (Code::PropType, "<TextContent text={1} />"),
        (Code::Placement, "text <Divider />"),
        (Code::Binding, "<Slider min={0} max={1} value={1} />"),
        (Code::ActionRequired, "<Button label=\"x\" />"),
        (Code::ActionPlacement, "{Compose(\"x\")}"),
        (Code::SitePlacement, "{Query(\"status\", {id: \"a\"}, {})}"),
        (
            Code::SiteArgs,
            "export const q = Query(\"status\", {}, {})\n\n{q.status}",
        ),
        (
            Code::EnvelopeMember,
            "export const q = Query(\"status\", {id: \"a\"}, {})\n\n{q.running}",
        ),
        (
            Code::RefreshRange,
            "export const q = Query(\"status\", {id: \"a\"}, {}, 1)\n\n{q.status}",
        ),
        (
            Code::UnknownOperation,
            "export const q = Watch(\"missing\", {})\n\n{q.status}",
        ),
        (
            Code::OperationKind,
            "export const q = Watch(\"status\", {id: \"a\"})\n\n{q.status}",
        ),
        (Code::WriteNotAllowed, "{Run(\"x\")}"),
        (Code::Unused, "export const x = 1"),
    ];
    let mut covered = vec![];
    for (code, source) in ordinary {
        assert!(
            codes(source, &context(Profile::ShowUi)).contains(&code),
            "{code}: {source}"
        );
        covered.push(code);
    }
    let mut nested = "1".to_owned();
    for name in ["a", "b", "c", "d", "e"] {
        nested = format!("Each([], \"{name}\", {nested})");
    }
    let steps = std::iter::repeat_n("Compose(\"x\")", 17)
        .collect::<Vec<_>>()
        .join(",");
    let sites = (0..17)
        .map(|i| format!("export const q{i} = Query(\"status\", {{id: \"a\"}}, {{}})\n"))
        .collect::<String>();
    let refs = "$$tc(1) ".repeat(65);
    let tags = "<Badge>x</Badge> ".repeat(65);
    for (code, source, profile) in [
        (Code::EachNesting, format!("{{{nested}}}"), Profile::ShowUi),
        (
            Code::LimitSteps,
            format!("<Button label=\"x\" action={{Action([{steps}])}} />"),
            Profile::ShowUi,
        ),
        (Code::SiteLimit, sites, Profile::ShowUi),
        (Code::ReferenceLimit, refs, Profile::ShowUi),
        (
            Code::ReferenceUnavailable,
            "$$tc(1)".into(),
            Profile::ToolRow,
        ),
        (
            Code::ReplyForbidden,
            "<Badge text={name} />".into(),
            Profile::Reply,
        ),
        (Code::LimitTags, tags, Profile::Reply),
    ] {
        assert!(
            codes(&source, &context(profile)).contains(&code),
            "{code}: {source}"
        );
        covered.push(code);
    }
    let mut newer = context(Profile::ShowUi);
    newer.execution_dialect = "2.0".into();
    assert_eq!(codes("", &newer), vec![Code::UnsupportedVersion]);
    covered.push(Code::UnsupportedVersion);
    assert!(check(b"bad \xff encoding", &context(Profile::ShowUi))
        .unwrap()
        .diagnostics
        .iter()
        .any(|d| d.code() == Code::Encoding));
    covered.push(Code::Encoding);
    assert_eq!(covered.len(), 37);
}

#[test]
fn aliased_site_args_cannot_bypass_the_static_size_limit() {
    let shared = "x".repeat(4096);
    let aliases = std::iter::repeat_n("chunk", 20)
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("export const chunk = \"{shared}\"\nexport const q = Query(\"status\", {{id: \"a\", extra: [{aliases}]}}, {{}})\n\n{{q.status}}");
    assert_eq!(
        codes(&source, &context(Profile::ShowUi)),
        vec![Code::SiteArgs]
    );
}

#[test]
fn schema_known_column_fields_are_checked_even_without_static_values() {
    let mut context = context(Profile::ToolPage);
    context.tool = Some(ToolInputs {
        input: HostProjection {
            value: json!({"key":"x","label":1}),
            schema: schema(
                json!({"type":"object","properties":{"key":{"type":"string"},"label":{"type":"number"}},"required":["key","label"]}),
            ),
        },
        view: HostProjection {
            value: json!({}),
            schema: Schema::default(),
        },
        truncated: false,
        truncated_fields: vec![],
    });
    assert!(codes(
        "<Table data={[]} columns={[{key: input.key, label: input.label}]} />",
        &context
    )
    .contains(&Code::PropType));
}

#[test]
fn registry_enum_schema_rejects_a_disjoint_host_enum() {
    let mut context = context(Profile::ToolPage);
    context.registry.0.insert("mode".into(), Operation::Query {
        arguments: schema(json!({"type":"object","properties":{"mode":{"type":"string","enum":["expected"]}},"required":["mode"]})),
        result: Schema::default(),
    });
    context.tool = Some(ToolInputs {
        input: HostProjection {
            value: json!({"mode":"other"}),
            schema: schema(
                json!({"type":"object","properties":{"mode":{"type":"string","enum":["other"]}},"required":["mode"]}),
            ),
        },
        view: HostProjection {
            value: json!({}),
            schema: Schema::default(),
        },
        truncated: false,
        truncated_fields: vec![],
    });
    assert!(codes(
        "export const q = Query(\"mode\", {mode: input.mode}, {})\n\n{q.status}",
        &context
    )
    .contains(&Code::SiteArgs));
}

#[test]
fn opaque_reply_write_span_composes_byte_offsets_into_scalar_positions() {
    let result = check(
        "é😀\n<Badge text={\n Run(\"x\")\n} />".as_bytes(),
        &context(Profile::Reply),
    )
    .unwrap();
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|d| d.code() == Code::WriteNotAllowed)
        .unwrap();
    assert_eq!(
        diagnostic.span(),
        Some(DiagnosticSpan::Source(crate::SourceSpan {
            line: 3,
            column: 2,
            end_line: 3,
            end_column: 10
        }))
    );
}
