use std::cell::RefCell;

use super::*;
use crate::DisplayText;

struct Fixed;

impl Metrics for Fixed {
    fn l(&self, style: TextStyle) -> Decimal {
        d(match style {
            TextStyle::Body | TextStyle::Mono => 20,
            TextStyle::Subheadline => 18,
            TextStyle::Footnote | TextStyle::MonoFootnote => 14,
            TextStyle::Caption1 => 12,
            TextStyle::Headline => 22,
            TextStyle::Title2 => 30,
            TextStyle::Title3 => 26,
        })
    }
    fn t(&self, text: &str, style: TextStyle, width: Decimal, max_lines: Option<u32>) -> Decimal {
        if text.is_empty() {
            return d(0);
        }
        let lines = self
            .tw(text, style)
            .checked_div(width.max(d(1)))
            .unwrap()
            .ceil()
            .unwrap();
        lines
            .min(d(max_lines.unwrap_or(u32::MAX) as i128))
            .checked_mul(self.l(style))
            .unwrap()
    }
    fn tw(&self, text: &str, _: TextStyle) -> Decimal {
        d(text.chars().count() as i128 * 6)
    }
}

fn s(text: &str) -> TreeValue {
    TreeValue::String(DisplayText::new(text))
}
fn n(value: i128) -> TreeValue {
    TreeValue::Number(d(value))
}
fn values(values: Vec<TreeValue>) -> TreeValue {
    TreeValue::List(values)
}
fn node(name: &str, props: Vec<(&str, TreeValue)>, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::Component {
        name: DisplayText::new(name),
        props: props
            .into_iter()
            .map(|(k, v)| (DisplayText::new(k), v))
            .collect(),
        children,
    }
}
fn boxed(node: TreeNode) -> TreeValue {
    TreeValue::Node(Box::new(node))
}
fn context(placement: Placement) -> LayoutContext {
    LayoutContext {
        width: d(200),
        content_size_category: "body-normal".into(),
        network: Network::Unknown,
        placement,
    }
}
fn revision(tree: &Tree) -> MeasureRevision<'_> {
    MeasureRevision {
        instance_epoch: 1,
        revision: 7,
        state_hash: "state-digest",
        tree,
        first: None,
    }
}
fn run(node: TreeNode, placement: Placement) -> Measurement {
    measure(
        &revision(&Tree { blocks: vec![node] }),
        &context(placement),
        &Fixed,
    )
    .unwrap()
}
fn height(node: TreeNode, expected: i128) {
    for placement in [Placement::Row, Placement::Page] {
        assert_eq!(
            run(node.clone(), placement).height,
            Height::Known(d(expected))
        );
    }
}
fn button(label: &str) -> TreeNode {
    node("Button", vec![("label", s(label))], vec![])
}

/// A Slot's box is `lines` body lines tall and its width class counted in
/// the digit zero (6 units each in these tests' fixed metrics), whatever its
/// content: a tall child is measured, for its box, but clipped by the Slot.
#[test]
fn a_slot_keeps_its_fixed_box_whatever_its_content() {
    let tall = node(
        "TextContent",
        vec![("text", s(&"word ".repeat(200)))],
        vec![],
    );
    for (lines, content, expected) in [
        (1, vec![], 20),
        (2, vec![tall.clone()], 40),
        (
            3,
            vec![node("Badge", vec![("text", s("Done"))], vec![])],
            60,
        ),
    ] {
        height(
            node(
                "Slot",
                vec![("width", s("short")), ("lines", n(lines))],
                content,
            ),
            expected,
        );
    }
    // A waiting Slot has no children and the same box.
    height(
        node(
            "Slot",
            vec![
                ("width", s("medium")),
                ("waiting", TreeValue::Boolean(true)),
            ],
            vec![],
        ),
        20,
    );
    let measured = run(
        node("Slot", vec![("width", s("short"))], vec![tall]),
        Placement::Row,
    );
    let slot = measured.boxes.iter().find(|b| b.path == "/0").unwrap();
    assert_eq!((slot.width, slot.height), (d(72), d(20)), "12 zeros wide");
    let child = measured.boxes.iter().find(|b| b.path == "/0/0").unwrap();
    assert_eq!(
        child.width,
        d(72),
        "the content is laid out at the Slot's width"
    );
    assert!(child.height > d(20), "and clipped by it");
    // medium and long cap at the offered width; fill takes it.
    for (width, expected) in [("medium", 144), ("long", 200), ("fill", 200)] {
        let measured = run(
            node("Slot", vec![("width", s(width))], vec![]),
            Placement::Page,
        );
        assert_eq!(measured.boxes[0].width, d(expected), "{width}");
    }
}

/// In a row Card, short, medium and long Slots take their own width and a fill
/// Slot shares what is left, like any other flexible child.
#[test]
fn slots_in_a_row_card_take_their_width_class() {
    let row = node(
        "Card",
        vec![
            ("appearance", s("clear")),
            ("direction", s("row")),
            ("gap", s("none")),
        ],
        vec![
            node("Slot", vec![("width", s("short"))], vec![]),
            node("Slot", vec![("width", s("fill"))], vec![]),
        ],
    );
    let measured = run(row, Placement::Row);
    let width = |path: &str| {
        measured
            .boxes
            .iter()
            .find(|b| b.path == path)
            .unwrap()
            .width
    };
    assert_eq!((width("/0/0"), width("/0/1")), (d(72), d(128)));
    assert_eq!(measured.height, Height::Known(d(20)));
}

/// A Slot keeps its width beside a code wider than the row: the code is cut to
/// what the Slot leaves, wherever the Slot sits in the row.
#[test]
fn a_slot_keeps_its_width_beside_a_long_code() {
    let code = node("InlineCode", vec![("code", s(&"x".repeat(200)))], vec![]);
    let slot = node("Slot", vec![("width", s("short"))], vec![]);
    for (children, code_at, slot_at) in [
        (vec![code.clone(), slot.clone()], "/0/0", "/0/1"),
        (vec![slot, code], "/0/1", "/0/0"),
    ] {
        let row = node(
            "Card",
            vec![
                ("appearance", s("clear")),
                ("direction", s("row")),
                ("gap", s("s")),
            ],
            children,
        );
        let measured = run(row, Placement::Row);
        let width = |path: &str| {
            measured
                .boxes
                .iter()
                .find(|b| b.path == path)
                .unwrap()
                .width
        };
        // 200 wide, a gap of 4 and a short Slot of 12 zeros at 6 each.
        assert_eq!((width(slot_at), width(code_at)), (d(72), d(124)));
    }
}

/// The clock, the agent and the model are one body line tall and have fixed
/// widths, counted in widths of the digit zero, whatever they hold, because
/// the app draws and cuts their text. An open-item link is prose, measured by
/// the app.
#[test]
fn the_app_drawn_tool_parts_have_fixed_boxes() {
    let gap = crate::catalog::Catalog::shared()
        .layout_constant("gap.s")
        .unwrap();
    for (part, props, width) in [
        ("Elapsed", vec![("start", n(0))], d(42)),
        (
            "Agent",
            vec![("id", s("an-agent-with-a-very-long-name-indeed"))],
            d(20).checked_add(gap).unwrap().checked_add(d(96)).unwrap(),
        ),
        ("Model", vec![("id", s("m"))], d(144)),
    ] {
        let measured = run(node(part, props, vec![]), Placement::Row);
        assert_eq!(measured.height, Height::Known(d(20)), "{part}");
        let part_box = measured.boxes.iter().find(|b| b.path == "/0/0");
        let part_box = part_box.or_else(|| measured.boxes.iter().find(|b| b.path == "/0"));
        assert_eq!(part_box.unwrap().width, width, "{part}");
    }
    let link = run(
        node(
            "OpenItem",
            vec![("id", s("ask_1")), ("label", s("The ask"))],
            vec![],
        ),
        Placement::Page,
    );
    assert_eq!(link.height, Height::ProseDeferred);
}

#[test]
fn formula_heights_are_hand_computed_in_both_placements() {
    let cases = vec![
        ("Badge", vec![], vec![], 16),
        (
            "Callout",
            vec![("title", s("title")), ("description", s("body"))],
            vec![],
            70,
        ),
        ("Col", vec![], vec![], 0),
        (
            "LinkCard",
            vec![
                ("title", s("title")),
                ("description", s("desc")),
                ("href", s("https://example.org/")),
            ],
            vec![],
            80,
        ),
        (
            "Citation",
            vec![("source", TreeValue::Handle(DisplayText::new("source")))],
            vec![],
            16,
        ),
        ("Image", vec![("caption", s("caption"))], vec![], 168),
        (
            "ImageGroup",
            vec![
                ("images", values(vec![s("a"), s("b"), s("c")])),
                ("caption", s("caption")),
            ],
            vec![],
            169,
        ),
        ("Map", vec![("caption", s("caption"))], vec![], 131),
        ("Chart", vec![("kind", s("line"))], vec![], 120),
        ("Series", vec![], vec![], 0),
        (
            "Card",
            vec![],
            vec![node("Divider", vec![], vec![]), button("go")],
            93,
        ),
        (
            "CardHeader",
            vec![("title", s("Title")), ("subtitle", s("Subtitle"))],
            vec![],
            46,
        ),
        ("TextContent", vec![("text", s("Text"))], vec![], 20),
        ("InlineCode", vec![("code", s("x"))], vec![], 18),
        (
            "CodeBlock",
            vec![
                ("codeString", s("a\nb\nc\n")),
                ("language", s("rs")),
                ("maxLines", n(2)),
            ],
            vec![],
            90,
        ),
        (
            "FormControl",
            vec![
                ("label", s("label")),
                ("hint", s("hint")),
                ("input", boxed(node("Input", vec![("lines", n(3))], vec![]))),
            ],
            vec![],
            116,
        ),
        ("Input", vec![("lines", n(3))], vec![], 80),
        ("Select", vec![], vec![], 44),
        ("SelectItem", vec![], vec![], 0),
        ("Slider", vec![], vec![], 44),
        ("Toggle", vec![("label", s("Toggle"))], vec![], 44),
        ("Button", vec![("label", s("go"))], vec![], 44),
        (
            "Buttons",
            vec![(
                "buttons",
                values(vec![boxed(button("go")), boxed(button("go"))]),
            )],
            vec![],
            44,
        ),
        (
            "Tabs",
            vec![],
            vec![
                node("TabItem", vec![], vec![node("Divider", vec![], vec![])]),
                node("TabItem", vec![], vec![button("go")]),
            ],
            92,
        ),
        (
            "TabItem",
            vec![],
            vec![node("Divider", vec![], vec![]), button("go")],
            69,
        ),
        ("ListItem", vec![], vec![], 50),
        ("Stat", vec![("caption", s("caption"))], vec![], 60),
        ("Progress", vec![("label", s("label"))], vec![], 22),
        ("Divider", vec![], vec![], 17),
        (
            "ReferenceChip",
            vec![("label", s("tool output"))],
            vec![],
            16,
        ),
        ("ErrorMarker", vec![("code", s("E_TYPE"))], vec![], 16),
        ("ErrorCell", vec![], vec![], 42),
        ("TaskCheckbox", vec![], vec![], 20),
        ("LoadingRoot", vec![], vec![], 68),
        ("ProgramFallback", vec![("fallback", s("text"))], vec![], 96),
        ("ArgumentsShortened", vec![], vec![], 14),
        ("ReplyShortened", vec![], vec![], 22),
    ];
    for (name, props, children, expected) in cases {
        height(node(name, props, children), expected);
    }
}

#[test]
fn viewport_heights_use_reserved_rows_not_data_length() {
    for (name, row, page) in [
        ("Table", 356, 1556),
        ("List", 526, 2526),
        ("LiveOutput", 186, 746),
    ] {
        for state in ["ready", "loading", "error"] {
            let value = if state == "error" {
                TreeValue::Error(DiagnosticCode::Type)
            } else {
                TreeValue::Snapshot {
                    record_id: DisplayText::new("data"),
                    state: DisplayText::new(state),
                    value: None,
                }
            };
            let input = if name == "LiveOutput" {
                "source"
            } else {
                "data"
            };
            let node = node(name, vec![("displayRows", n(50)), (input, value)], vec![]);
            assert_eq!(
                run(node.clone(), Placement::Row).height,
                Height::Known(d(row)),
                "{name} {state}"
            );
            assert_eq!(
                run(node, Placement::Page).height,
                Height::Known(d(page)),
                "{name} {state}"
            );
        }
    }
}

#[test]
fn late_inputs_and_error_states_never_change_formula_boxes() {
    for (name, input, extra, expected) in [
        ("Image", "image", vec![], 150),
        ("Map", "places", vec![], 113),
        (
            "Chart",
            "data",
            vec![("kind", s("line")), ("x", s("x")), ("y", s("y"))],
            120,
        ),
        ("Citation", "source", vec![], 16),
        ("Table", "data", vec![], 236),
    ] {
        for placement in [Placement::Row, Placement::Page] {
            for state in ["ready", "loading", "held", "unavailable", "error"] {
                let value = if state == "error" {
                    TreeValue::Error(DiagnosticCode::JsonSyntax)
                } else {
                    TreeValue::Snapshot {
                        record_id: DisplayText::new("same-source"),
                        state: DisplayText::new(state),
                        value: Some(Box::new(values(vec![]))),
                    }
                };
                let mut props = extra.clone();
                props.push((input, value));
                let measured = run(node(name, props, vec![]), placement);
                assert_eq!(
                    measured.height,
                    Height::Known(d(expected)),
                    "{name} {state}"
                );
                let state = if state == "error" {
                    RenderState::Error(DiagnosticCode::JsonSyntax)
                } else if state == "loading" {
                    RenderState::Loading
                } else if state == "held" {
                    RenderState::Held
                } else if state == "unavailable" || name == "Map" {
                    RenderState::Unavailable
                } else {
                    RenderState::Ready
                };
                assert_eq!(measured.boxes.last().unwrap().state, state);
            }
        }
    }
}

#[test]
fn prose_and_app_owned_measurements_have_explicit_seams() {
    let error = TreeNode::ErrorCell {
        name: DisplayText::new("Image"),
        code: DiagnosticCode::Type,
    };
    let measured = run(error, Placement::Row);
    assert_eq!(measured.height, Height::Known(d(42)));
    assert_eq!(
        measured.boxes[0].state,
        RenderState::Error(DiagnosticCode::Type)
    );
    for deferred in [
        node("Markdown", vec![], vec![]),
        node("Styled", vec![], vec![]),
        node(
            "Callout",
            vec![],
            vec![TreeNode::Text(DisplayText::new("body"))],
        ),
    ] {
        assert_eq!(run(deferred, Placement::Row).height, Height::ProseDeferred);
    }
    assert_eq!(
        measure(
            &revision(&Tree {
                blocks: vec![node("GenericToolRow", vec![], vec![])]
            }),
            &context(Placement::Row),
            &Fixed
        ),
        Err(MeasureError::GenericToolRow)
    );
}

#[test]
fn empty_stacks_and_top_level_gaps_follow_clear_column_rules() {
    height(node("Card", vec![], vec![]), 24);
    height(node("Card", vec![("direction", s("row"))], vec![]), 24);
    height(
        node(
            "Card",
            vec![("direction", s("row")), ("wrap", TreeValue::Boolean(true))],
            vec![],
        ),
        24,
    );
    height(node("TabItem", vec![], vec![]), 0);
    for (count, expected) in [(0, 0), (1, 17), (2, 42), (3, 67)] {
        let tree = Tree {
            blocks: vec![node("Divider", vec![], vec![]); count],
        };
        assert_eq!(
            measure(&revision(&tree), &context(Placement::Page), &Fixed)
                .unwrap()
                .height,
            Height::Known(d(expected))
        );
    }
}

#[test]
fn card_rows_use_intrinsic_widths_floor_and_greedy_wrap() {
    let text = || {
        node(
            "TextContent",
            vec![("text", s("a".repeat(30).as_str()))],
            vec![],
        )
    };
    let card = |props: Vec<(&str, TreeValue)>, children| {
        let mut p = vec![("direction", s("row")), ("appearance", s("clear"))];
        p.extend(props);
        node("Card", p, children)
    };
    // Fixed button 44, gap 8, flexible share 148: 180 units of text wraps twice.
    height(card(vec![], vec![button("go"), text()]), 44);
    let measured = run(card(vec![], vec![button("go"), text()]), Placement::Row);
    assert_eq!(
        measured
            .boxes
            .iter()
            .find(|b| b.path == "/0/1")
            .unwrap()
            .width,
        d(148)
    );
    // Two flexible children share 96 each, each needs two lines.
    height(card(vec![], vec![text(), text()]), 40);
    // The two flexible children retain their 80-unit floor despite overflow.
    let measured = run(
        card(vec![], vec![button("long label"), text(), text()]),
        Placement::Row,
    );
    assert_eq!(
        measured
            .boxes
            .iter()
            .find(|b| b.path == "/0/1")
            .unwrap()
            .width,
        d(80)
    );
    assert_eq!(
        measured
            .boxes
            .iter()
            .find(|b| b.path == "/0/2")
            .unwrap()
            .width,
        d(80)
    );
    assert_eq!(measured.height, Height::Known(d(60)));
    // Toggle is 24 + 51 + 8 = 83 wide, leaving 109 to its flexible sibling.
    let measured = run(
        card(
            vec![],
            vec![node("Toggle", vec![("label", s("four"))], vec![]), text()],
        ),
        Placement::Row,
    );
    assert_eq!(
        measured
            .boxes
            .iter()
            .find(|b| b.path == "/0/0")
            .unwrap()
            .width,
        d(83)
    );
    assert_eq!(
        measured
            .boxes
            .iter()
            .find(|b| b.path == "/0/1")
            .unwrap()
            .width,
        d(109)
    );
    // Fixed children that do not fit are cut from the end, not rewrapped.
    let measured = run(
        card(
            vec![],
            vec![button(&"x".repeat(20)), button(&"y".repeat(20))],
        ),
        Placement::Row,
    );
    assert_eq!(
        measured
            .boxes
            .iter()
            .find(|b| b.path == "/0/0")
            .unwrap()
            .width,
        d(152)
    );
    assert_eq!(
        measured
            .boxes
            .iter()
            .find(|b| b.path == "/0/1")
            .unwrap()
            .width,
        d(40)
    );
    // Greedy fixed wrapping gives two 44-high lines with an 8-unit gap.
    height(
        card(
            vec![("wrap", TreeValue::Boolean(true))],
            vec![button(&"x".repeat(20)), button(&"y".repeat(20))],
        ),
        96,
    );
    // A flexible child fits after one short fixed child, then ends that line.
    height(
        card(
            vec![("wrap", TreeValue::Boolean(true))],
            vec![button("go"), text(), button("go")],
        ),
        96,
    );
    // A 152-unit fixed child leaves 40 (<80); the flexible child starts anew.
    height(
        card(
            vec![("wrap", TreeValue::Boolean(true))],
            vec![button(&"x".repeat(20)), text()],
        ),
        72,
    );
}

#[test]
fn buttons_wrap_and_contribute_row_or_column_intrinsic_widths() {
    for (direction, width, expected) in [("row", 92, 44), ("column", 44, 92)] {
        let buttons = node(
            "Buttons",
            vec![
                ("direction", s(direction)),
                (
                    "buttons",
                    values(vec![boxed(button("go")), boxed(button("go"))]),
                ),
            ],
            vec![],
        );
        let measured = run(
            node(
                "Card",
                vec![("appearance", s("clear")), ("direction", s("row"))],
                vec![buttons],
            ),
            Placement::Row,
        );
        assert_eq!(measured.height, Height::Known(d(expected)));
        assert_eq!(
            measured
                .boxes
                .iter()
                .find(|b| b.path == "/0/0")
                .unwrap()
                .width,
            d(width)
        );
    }
    let buttons = node(
        "Buttons",
        vec![(
            "buttons",
            values(vec![
                boxed(button(&"x".repeat(20))),
                boxed(button(&"y".repeat(20))),
            ]),
        )],
        vec![],
    );
    height(buttons, 92);
}

#[test]
fn default_and_optional_formula_branches_are_not_state_dependent() {
    height(
        node(
            "Input",
            vec![("type", s("number")), ("lines", n(6))],
            vec![],
        ),
        44,
    );
    height(node("Progress", vec![], vec![]), 6);
    height(node("Stat", vec![], vec![]), 46);
    height(node("Image", vec![("aspect", s("1:1"))], vec![]), 200);
    height(
        node(
            "ImageGroup",
            vec![("images", values(vec![s("one")]))],
            vec![],
        ),
        150,
    );
    height(
        node(
            "TextContent",
            vec![("size", s("small")), ("text", s("small"))],
            vec![],
        ),
        14,
    );
    height(
        node(
            "TextContent",
            vec![("size", s("large")), ("text", s("large"))],
            vec![],
        ),
        26,
    );
    height(node("CodeBlock", vec![("codeString", s(""))], vec![]), 20);
    height(
        node("CodeBlock", vec![("codeString", s("x\r\ny\n"))], vec![]),
        60,
    );
    let link = node(
        "LinkCard",
        vec![("title", s("Title")), ("href", s("https://a.example/"))],
        vec![],
    );
    height(link.clone(), 60);
    let tree = Tree { blocks: vec![link] };
    let mut cx = context(Placement::Page);
    cx.network = Network::No;
    assert_eq!(
        measure(&revision(&tree), &cx, &Fixed).unwrap().height,
        Height::Known(d(20))
    );
    cx.network = Network::Yes;
    assert_eq!(
        measure(&revision(&tree), &cx, &Fixed).unwrap().height,
        Height::Known(d(60))
    );
    let tree = Tree {
        blocks: vec![node("Chart", vec![], vec![])],
    };
    cx.width = d(400);
    assert_eq!(
        measure(&revision(&tree), &cx, &Fixed).unwrap().height,
        Height::Known(d(225))
    );
}

#[test]
fn decimal_metrics_round_each_child_before_parent_addition() {
    struct Fractional;
    impl Metrics for Fractional {
        fn l(&self, _: TextStyle) -> Decimal {
            Decimal::from_literal("13.25").unwrap()
        }
        fn t(&self, _: &str, _: TextStyle, _: Decimal, _: Option<u32>) -> Decimal {
            Decimal::from_literal("1.01").unwrap()
        }
        fn tw(&self, _: &str, _: TextStyle) -> Decimal {
            Decimal::from_literal("6.01").unwrap()
        }
    }
    let tree = Tree {
        blocks: vec![node(
            "Card",
            vec![],
            vec![node("Badge", vec![], vec![]), node("Badge", vec![], vec![])],
        )],
    };
    let measured = measure(&revision(&tree), &context(Placement::Page), &Fractional).unwrap();
    assert_eq!(measured.height, Height::Known(d(68))); // 24 + 18 + 8 + 18
    assert_eq!(measured.boxes[0].height, d(18));
    assert_eq!(measured.boxes[0].width, d(19));
    let tree = Tree {
        blocks: vec![node(
            "Image",
            vec![("aspect", s("16:9")), ("caption", s("c"))],
            vec![],
        )],
    };
    let mut cx = context(Placement::Page);
    cx.width = Decimal::from_literal("100.1").unwrap();
    // 100.1 * .5625 + 4 + 1.01 = 61.31625, rounded once to 62.
    assert_eq!(
        measure(&revision(&tree), &cx, &Fractional).unwrap().height,
        Height::Known(d(62))
    );
}

#[test]
fn metric_arguments_are_catalog_arguments_and_line_metrics_are_cached() {
    type TextCall = (String, TextStyle, Decimal, Option<u32>);
    #[derive(Default)]
    struct Recording {
        lines: RefCell<Vec<TextStyle>>,
        texts: RefCell<Vec<TextCall>>,
    }
    impl Metrics for Recording {
        fn l(&self, style: TextStyle) -> Decimal {
            self.lines.borrow_mut().push(style);
            Fixed.l(style)
        }
        fn t(&self, text: &str, style: TextStyle, width: Decimal, max: Option<u32>) -> Decimal {
            self.texts
                .borrow_mut()
                .push((text.to_owned(), style, width, max));
            Fixed.t(text, style, width, max)
        }
        fn tw(&self, text: &str, style: TextStyle) -> Decimal {
            Fixed.tw(text, style)
        }
    }
    let metrics = Recording::default();
    let tree = Tree {
        blocks: vec![
            node(
                "Callout",
                vec![("title", s("title")), ("description", s("body"))],
                vec![],
            ),
            node("List", vec![], vec![]),
            node("List", vec![], vec![]),
        ],
    };
    measure(&revision(&tree), &context(Placement::Row), &metrics).unwrap();
    assert_eq!(
        *metrics.texts.borrow(),
        vec![
            ("title".into(), TextStyle::Headline, d(148), Some(3)),
            ("body".into(), TextStyle::Body, d(148), None)
        ]
    );
    assert_eq!(
        metrics
            .lines
            .borrow()
            .iter()
            .filter(|s| **s == TextStyle::Footnote)
            .count(),
        1
    );
    assert_eq!(
        metrics
            .lines
            .borrow()
            .iter()
            .filter(|s| **s == TextStyle::Body)
            .count(),
        1
    );
}

#[test]
fn layout_key_contains_every_catalog_cache_field_and_computed_digest() {
    let tree = Tree { blocks: vec![] };
    let rev = revision(&tree);
    let cx = context(Placement::Row);
    let key = layout_key(&rev, &cx);
    assert_eq!(
        key.catalog_digest,
        include_str!("../../catalog/catalog-v1.digest").trim()
    );
    assert_eq!(key.layout_version, "1");
    assert_eq!(
        (key.instance_epoch, key.revision, key.state_hash.as_str()),
        (1, 7, "state-digest")
    );
    assert_eq!(
        (
            key.width,
            key.content_size_category.as_str(),
            key.network,
            key.placement
        ),
        (d(200), "body-normal", Network::Unknown, Placement::Row)
    );
    let mut next = revision(&tree);
    next.revision += 1;
    assert_ne!(key, layout_key(&next, &cx));
    next.revision = rev.revision;
    next.instance_epoch += 1;
    assert_ne!(key, layout_key(&next, &cx));
    next.instance_epoch = rev.instance_epoch;
    next.state_hash = "new-state";
    assert_ne!(key, layout_key(&next, &cx));
    for changed in [
        LayoutContext {
            width: d(201),
            ..cx.clone()
        },
        LayoutContext {
            content_size_category: "large".into(),
            ..cx.clone()
        },
        LayoutContext {
            network: Network::Yes,
            ..cx.clone()
        },
        LayoutContext {
            placement: Placement::Page,
            ..cx.clone()
        },
    ] {
        assert_ne!(key, layout_key(&rev, &changed));
    }
}

#[test]
fn row_frame_measures_defaults_again_for_each_context_and_adds_open_once() {
    struct FrameMetrics;
    impl Metrics for FrameMetrics {
        fn l(&self, style: TextStyle) -> Decimal {
            match style {
                TextStyle::Headline => d(18),
                TextStyle::Subheadline => d(14),
                _ => Fixed.l(style),
            }
        }
        fn t(&self, text: &str, style: TextStyle, w: Decimal, max: Option<u32>) -> Decimal {
            Fixed.t(text, style, w, max)
        }
        fn tw(&self, text: &str, style: TextStyle) -> Decimal {
            Fixed.tw(text, style)
        }
    }
    let defaults = Tree {
        blocks: vec![node(
            "TextContent",
            vec![("text", s(&"x".repeat(320)))],
            vec![],
        )],
    };
    let loading = Tree {
        blocks: vec![node("LoadingRoot", vec![], vec![])],
    };
    let cx = context(Placement::Row);
    assert_eq!(
        measure(&revision(&defaults), &cx, &FrameMetrics)
            .unwrap()
            .height,
        Height::Known(d(200))
    );
    assert_eq!(
        measure(&revision(&loading), &cx, &FrameMetrics)
            .unwrap()
            .height,
        Height::Known(d(60))
    );
    let frame = Tree {
        blocks: vec![node("RowFrame", vec![], vec![])],
    };
    let mut rev = revision(&frame);
    rev.first = Some(&defaults);
    assert_eq!(
        measure(&rev, &cx, &FrameMetrics).unwrap().height,
        Height::Known(d(244))
    );
    let mut wide = cx.clone();
    wide.width = d(400);
    assert_eq!(
        measure(&rev, &wide, &FrameMetrics).unwrap().height,
        Height::Known(d(144))
    );
    // The baseline is not the already framed height from a previous context.
    assert_eq!(
        measure(&rev, &cx, &FrameMetrics).unwrap().height,
        Height::Known(d(244))
    );
    let tall = Tree {
        blocks: vec![node(
            "TextContent",
            vec![("text", s(&"x".repeat(3200))), ("maxLines", n(100))],
            vec![],
        )],
    };
    rev.first = Some(&tall);
    assert_eq!(
        measure(&rev, &cx, &FrameMetrics).unwrap().height,
        Height::Known(d(524))
    );
}

#[test]
fn chart_and_map_render_caps_change_state_not_height_and_allow_boundary() {
    for (points, series, error) in [(2048, 8, false), (2049, 8, true), (2048, 9, true)] {
        let chart = node(
            "Chart",
            vec![
                ("kind", s("line")),
                ("data", values(vec![TreeValue::Record(vec![]); points])),
                ("x", s("x")),
                ("y", values(vec![s("y"); series])),
            ],
            vec![],
        );
        let result = run(chart, Placement::Page);
        assert_eq!(result.height, Height::Known(d(120)));
        assert_eq!(
            result.boxes.last().unwrap().state,
            if error {
                RenderState::Error(DiagnosticCode::RenderLimit)
            } else {
                RenderState::Ready
            }
        );
    }
    for (markers, error) in [(100, false), (101, true)] {
        let map = node(
            "Map",
            vec![(
                "places",
                TreeValue::Snapshot {
                    record_id: DisplayText::new("map"),
                    state: DisplayText::new("ready"),
                    value: Some(Box::new(values(vec![TreeValue::Record(vec![]); markers]))),
                },
            )],
            vec![],
        );
        let result = run(map, Placement::Row);
        assert_eq!(result.height, Height::Known(d(113)));
        assert_eq!(
            result.boxes.last().unwrap().state,
            if error {
                RenderState::Error(DiagnosticCode::RenderLimit)
            } else {
                RenderState::Unavailable
            }
        );
    }
}

#[test]
fn table_columns_cells_and_viewport_are_bounded_by_catalog() {
    let record = |index| {
        TreeValue::Record(
            (0..9)
                .map(|col| {
                    (
                        DisplayText::new(&format!("c{col}")),
                        if col == 0 {
                            s(&"é".repeat(257))
                        } else if col == 1 {
                            n(1234567)
                        } else {
                            s(&format!("{index}:{col}"))
                        },
                    )
                })
                .collect(),
        )
    };
    let table = node(
        "Table",
        vec![
            ("data", values((0..50).map(record).collect())),
            ("displayRows", n(50)),
        ],
        vec![],
    );
    let row = run(table.clone(), Placement::Row);
    let data = &row.tables[0];
    assert_eq!(data.columns.len(), 8);
    assert_eq!(data.columns[0].key, "c0");
    assert_eq!(data.columns[7].key, "c7");
    assert_eq!(data.cells.len(), 10);
    assert_eq!(data.viewport_rows, 10);
    assert_eq!(data.cells[0][0].as_str(), format!("{}…", "é".repeat(256)));
    assert_eq!(data.cells[0][1].as_str(), "1,234,567");
    assert_eq!(data.cells[9][7].as_str(), "9:7");
    let page = run(table, Placement::Page);
    assert_eq!(page.tables[0].cells.len(), 50);
    assert_eq!(page.tables[0].cells[49][7].as_str(), "49:7");
    let columns = values(vec![TreeValue::Record(vec![
        (DisplayText::new("key"), s("c1")),
        (DisplayText::new("label"), s("A very long header")),
    ])]);
    let result = run(
        node(
            "Table",
            vec![("columns", columns), ("data", values(vec![record(0)]))],
            vec![],
        ),
        Placement::Row,
    );
    assert_eq!(result.tables[0].columns[0].label, "A very long header");
    assert_eq!(result.tables[0].columns[0].width, d(200));
    assert_eq!(result.tables[0].cells[0][0].as_str(), "1,234,567");
    let result = run(
        node(
            "Table",
            vec![(
                "columns",
                values(vec![boxed(node(
                    "Col",
                    vec![("label", s("Col")), ("data", values(vec![s("a"), s("b")]))],
                    vec![],
                ))]),
            )],
            vec![],
        ),
        Placement::Page,
    );
    assert_eq!(result.tables[0].cells[1][0].as_str(), "b");
}

#[test]
fn literal_catalog_overflows_have_exact_check_time_code() {
    let series = std::iter::repeat_n("<Series category=\"a\" values={[1]} />", 9)
        .collect::<Vec<_>>()
        .join(",");
    let chart = format!("<Chart kind=\"line\" labels={{[\"a\"]}} series={{[{series}]}} />");
    let columns = std::iter::repeat_n("\"x\"", 9)
        .collect::<Vec<_>>()
        .join(",");
    let table = format!("<Table data={{[]}} columns={{[{columns}]}} />");
    for source in [
        chart,
        table,
        "<Table data={[]} displayRows={51} />".to_owned(),
    ] {
        let checked = crate::check::check(
            source.as_bytes(),
            &crate::CheckContext::new(crate::Profile::ShowUi),
        )
        .unwrap();
        assert_eq!(
            checked
                .diagnostics
                .iter()
                .map(crate::Diagnostic::code)
                .collect::<Vec<_>>(),
            vec![DiagnosticCode::PropType],
            "{source}: {:?}",
            checked.diagnostics
        );
        assert_eq!(
            checked.diagnostics[0].phase(),
            crate::DiagnosticPhase::Check
        );
    }
}

#[test]
fn evaluated_item_maximum_uses_render_limit() {
    let source = "export const ns = Each([1,2,3,4,5,6,7,8,9], \"v\", <Series category=\"a\" values={[1]} />)\n\n<Chart kind=\"line\" labels={[\"a\"]} series={ns} />";
    let checked = crate::check::check(
        source.as_bytes(),
        &crate::CheckContext::new(crate::Profile::ShowUi),
    )
    .unwrap();
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let evaluated =
        crate::eval::evaluate(&checked, &crate::eval::Inputs::new(crate::Profile::ShowUi));
    assert_eq!(
        evaluated
            .diagnostics
            .iter()
            .map(crate::Diagnostic::code)
            .collect::<Vec<_>>(),
        vec![DiagnosticCode::RenderLimit]
    );
}

#[test]
fn two_hundred_fifty_row_tables_have_exact_known_work_without_admission() {
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
    let table = format!("<Table columns={{[{columns}]}} data={{rows}} displayRows={{50}} />");
    let source = format!(
        "export const rows = [{records}]\n\n{}",
        std::iter::repeat_n(table, 250)
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let checked = crate::check::check(
        source.as_bytes(),
        &crate::CheckContext::new(crate::Profile::ShowUi),
    )
    .unwrap();
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let evaluated =
        crate::eval::evaluate(&checked, &crate::eval::Inputs::new(crate::Profile::ShowUi));
    assert!(
        evaluated.diagnostics.is_empty(),
        "{:?}",
        evaluated.diagnostics
    );
    assert_eq!(evaluated.nodes, 250);
    let tree = evaluated.tree.unwrap();
    assert_eq!(work::known_work(&tree, Placement::Row).unwrap(), 22500);
    assert_eq!(work::known_work(&tree, Placement::Page).unwrap(), 102500);
    for placement in [Placement::Row, Placement::Page] {
        let measurement = measure(&revision(&tree), &context(placement), &Fixed).unwrap();
        assert_eq!(measurement.reservation, ReservationOutcome::Deferred);
        assert_eq!(
            measurement.known_work_visits,
            if placement == Placement::Row {
                22500
            } else {
                102500
            }
        );
    }
}

#[test]
fn known_work_counts_catalog_terms_without_base_nodes_or_text_runs() {
    for (component, expected) in [
        (node("Badge", vec![], vec![]), 2),
        (node("Styled", vec![], vec![]), 1),
        (
            node("Callout", vec![("description", s("prose"))], vec![]),
            2,
        ),
        (node("LinkCard", vec![], vec![]), 4),
        (node("Citation", vec![], vec![]), 1),
        (node("Image", vec![], vec![]), 2),
        (
            node(
                "ImageGroup",
                vec![("images", values(vec![s("a"), s("b")]))],
                vec![],
            ),
            3,
        ),
        (
            node(
                "Map",
                vec![("places", values(vec![TreeValue::Null; 10]))],
                vec![],
            ),
            11,
        ),
        (
            node(
                "Chart",
                vec![
                    ("labels", values(vec![n(0); 10])),
                    (
                        "series",
                        values(vec![boxed(node("Series", vec![], vec![])); 2]),
                    ),
                ],
                vec![],
            ),
            28,
        ),
        (node("Card", vec![], vec![button("go")]), 3),
        (node("CardHeader", vec![], vec![]), 3),
        (
            node("TextContent", vec![("text", s(&"x".repeat(257)))], vec![]),
            3,
        ),
        (node("InlineCode", vec![], vec![]), 1),
        (
            node(
                "CodeBlock",
                vec![("codeString", s("a\nb\nc\n")), ("maxLines", n(2))],
                vec![],
            ),
            4,
        ),
        (
            node(
                "FormControl",
                vec![("input", boxed(node("Input", vec![], vec![])))],
                vec![],
            ),
            5,
        ),
        (
            node(
                "Select",
                vec![(
                    "items",
                    values(vec![boxed(node("SelectItem", vec![], vec![])); 3]),
                )],
                vec![],
            ),
            5,
        ),
        (node("Slider", vec![], vec![]), 2),
        (node("Toggle", vec![], vec![]), 2),
        (button("go"), 2),
        (
            node(
                "Buttons",
                vec![(
                    "buttons",
                    values(vec![boxed(button("a")), boxed(button("b"))]),
                )],
                vec![],
            ),
            5,
        ),
        (
            node(
                "Tabs",
                vec![],
                vec![
                    node("TabItem", vec![], vec![button("a")]),
                    node("TabItem", vec![], vec![button("b")]),
                ],
            ),
            9,
        ),
        (
            node(
                "List",
                vec![("displayRows", n(50))],
                vec![node("ListItem", vec![], vec![]); 50],
            ),
            32,
        ),
        (node("ListItem", vec![], vec![]), 3),
        (node("LiveOutput", vec![("displayRows", n(50))], vec![]), 12),
        (node("Stat", vec![], vec![]), 3),
        (node("Progress", vec![], vec![]), 2),
        (node("Divider", vec![], vec![]), 1),
    ] {
        assert_eq!(
            work::known_work(
                &Tree {
                    blocks: vec![component.clone()]
                },
                Placement::Row
            )
            .unwrap(),
            expected,
            "{component:?}"
        );
    }
}

#[test]
fn citation_indices_reuse_source_identity_and_inline_boxes_round_width() {
    let cite = |id: &str| {
        node(
            "Citation",
            vec![("source", TreeValue::Handle(DisplayText::new(id)))],
            vec![],
        )
    };
    let tree = Tree {
        blocks: vec![cite("one"), cite("two"), cite("one")],
    };
    let result = measure(&revision(&tree), &context(Placement::Row), &Fixed).unwrap();
    assert_eq!(
        result.boxes.iter().map(|b| b.width).collect::<Vec<_>>(),
        vec![d(14), d(14), d(14)]
    );
    let tree = Tree {
        blocks: (0..10)
            .map(|i| cite(&i.to_string()))
            .chain([cite("0")])
            .collect(),
    };
    let result = measure(&revision(&tree), &context(Placement::Row), &Fixed).unwrap();
    assert_eq!(result.boxes[9].width, d(20));
    assert_eq!(result.boxes[10].width, d(14));

    // Planning the later fixed sibling must not assign its index before the
    // nine citations in the earlier flexible Card have been encountered.
    let nested = node(
        "Card",
        vec![],
        (0..9).map(|i| cite(&i.to_string())).collect(),
    );
    let tree = Tree {
        blocks: vec![node(
            "Card",
            vec![("appearance", s("clear")), ("direction", s("row"))],
            vec![nested, cite("tenth")],
        )],
    };
    let result = measure(&revision(&tree), &context(Placement::Row), &Fixed).unwrap();
    assert_eq!(
        result
            .boxes
            .iter()
            .find(|b| b.path == "/0/1")
            .unwrap()
            .width,
        d(20)
    );
}

#[test]
fn only_catalog_min_widths_clamp_standalone_inline_boxes() {
    let mut cx = context(Placement::Row);
    cx.width = d(5);
    for (name, props, width) in [
        (
            "Citation",
            vec![("source", TreeValue::Handle(DisplayText::new("source")))],
            14,
        ),
        ("ErrorMarker", vec![("code", s("E_TYPE"))], 44),
        ("TaskCheckbox", vec![], 20),
        ("Badge", vec![("text", s("label"))], 5),
    ] {
        let tree = Tree {
            blocks: vec![node(name, props, vec![])],
        };
        let measured = measure(&revision(&tree), &cx, &Fixed).unwrap();
        assert_eq!(measured.boxes[0].width, d(width), "{name}");
    }
    let tree = Tree {
        blocks: vec![node(
            "Card",
            vec![("appearance", s("clear")), ("direction", s("row"))],
            vec![node(
                "Citation",
                vec![("source", TreeValue::Handle(DisplayText::new("source")))],
                vec![],
            )],
        )],
    };
    let measured = measure(&revision(&tree), &cx, &Fixed).unwrap();
    assert_eq!(measured.boxes[0].width, d(5));
}

#[test]
fn invalid_metric_values_and_arithmetic_overflow_are_not_silently_rounded() {
    struct Bad;
    impl Metrics for Bad {
        fn l(&self, _: TextStyle) -> Decimal {
            d(-1)
        }
        fn t(&self, _: &str, _: TextStyle, _: Decimal, _: Option<u32>) -> Decimal {
            d(-1)
        }
        fn tw(&self, _: &str, _: TextStyle) -> Decimal {
            d(-1)
        }
    }
    for component in [
        node("Badge", vec![], vec![]),
        node("TextContent", vec![("text", s("x"))], vec![]),
        button("go"),
    ] {
        let tree = Tree {
            blocks: vec![component],
        };
        assert_eq!(
            measure(&revision(&tree), &context(Placement::Row), &Bad),
            Err(MeasureError::Number(NumberError::Arg))
        );
    }
    let mut cx = context(Placement::Page);
    cx.width = Decimal::from_integer(99_999_999_999_999_999_999).unwrap();
    // Adding the caption must report overflow rather than saturating the box.
    let tree = Tree {
        blocks: vec![node(
            "Image",
            vec![("aspect", s("1:1")), ("caption", s("c"))],
            vec![],
        )],
    };
    assert_eq!(
        measure(&revision(&tree), &cx, &Fixed),
        Err(MeasureError::Number(NumberError::Overflow))
    );
}
