use super::*;

fn t(text: &str) -> DisplayText {
    DisplayText::new(text)
}

fn num(text: &str) -> TreeValue {
    TreeValue::Number(decimal::parse(text).unwrap())
}

fn encode(tree: &Tree) -> String {
    serde_json::to_string(&TreeJson(tree)).unwrap()
}

fn decode(json: &str) -> Result<Tree, serde_json::Error> {
    serde_json::from_str::<TreeDe>(json).map(|tree| tree.0)
}

fn markdown(kind: MarkdownKind, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::Markdown { kind, children }
}

/// A tree with every node kind, every Markdown kind and every prop value
/// kind, with props deliberately out of alphabetical order.
fn everything() -> Tree {
    let kinds = [
        MarkdownKind::Paragraph,
        MarkdownKind::Heading(2),
        MarkdownKind::ThematicBreak,
        MarkdownKind::CodeBlock {
            language: t("rust"),
            text: t("fn main() {}\n"),
        },
        MarkdownKind::Code(t("x")),
        MarkdownKind::Quote,
        MarkdownKind::List { start: None },
        MarkdownKind::List { start: Some(3) },
        MarkdownKind::Item,
        MarkdownKind::Task(true),
        MarkdownKind::Task(false),
        MarkdownKind::Table,
        MarkdownKind::TableHead,
        MarkdownKind::TableRow,
        MarkdownKind::TableCell,
        MarkdownKind::Emphasis,
        MarkdownKind::Strong,
        MarkdownKind::Strikethrough,
        MarkdownKind::Link {
            destination: t("https://example.test/a?b"),
            title: t(""),
            image: false,
        },
        MarkdownKind::Link {
            destination: t("x.png"),
            title: t("T"),
            image: true,
        },
        MarkdownKind::SoftBreak,
        MarkdownKind::HardBreak,
    ];
    let mut blocks: Vec<TreeNode> = kinds
        .into_iter()
        .map(|kind| markdown(kind, vec![TreeNode::Text(t("é 🙂"))]))
        .collect();
    let inner = TreeNode::Component {
        name: t("Badge"),
        props: vec![(t("tone"), TreeValue::String(t("info")))],
        children: vec![TreeNode::Text(t("in a prop"))],
    };
    blocks.push(TreeNode::Component {
        name: t("Card"),
        props: vec![
            (t("z"), TreeValue::Null),
            (t("b"), TreeValue::Boolean(true)),
            (t("a"), TreeValue::Boolean(false)),
            (t("s"), TreeValue::String(t("text \"quoted\""))),
            (t("n"), num("-12.5")),
            (t("zero"), num("0")),
            (
                t("l"),
                TreeValue::List(vec![num("1"), TreeValue::Null, TreeValue::List(vec![])]),
            ),
            (
                t("r"),
                TreeValue::Record(vec![
                    (t("y"), num("2")),
                    (t("x"), TreeValue::Record(vec![])),
                ]),
            ),
            (t("node"), TreeValue::Node(Box::new(inner))),
            (
                t("action"),
                TreeValue::Action(vec![
                    TreeStep {
                        operation: ActionOperation::Set,
                        arguments: vec![TreeValue::String(t("page")), num("2")],
                    },
                    TreeStep {
                        operation: ActionOperation::Reset,
                        arguments: vec![TreeValue::String(t("page"))],
                    },
                    TreeStep {
                        operation: ActionOperation::Refresh,
                        arguments: vec![],
                    },
                    TreeStep {
                        operation: ActionOperation::LoadMore,
                        arguments: vec![],
                    },
                    TreeStep {
                        operation: ActionOperation::Compose,
                        arguments: vec![TreeValue::String(t("hi"))],
                    },
                    TreeStep {
                        operation: ActionOperation::Open,
                        arguments: vec![TreeValue::Handle(t("h1"))],
                    },
                ]),
            ),
            (
                t("snap"),
                TreeValue::Snapshot {
                    record_id: t("7"),
                    state: t("resolved"),
                    value: Some(Box::new(TreeValue::Null)),
                },
            ),
            (
                t("loading"),
                TreeValue::Snapshot {
                    record_id: t("8"),
                    state: t("held"),
                    value: None,
                },
            ),
            (t("handle"), TreeValue::Handle(t("art_1"))),
            (t("site"), TreeValue::Site(t("history"))),
            (
                t("value"),
                TreeValue::Binding {
                    state: t("page"),
                    kind: "number",
                },
            ),
            (t("broken"), TreeValue::Error(DiagnosticCode::PropType)),
        ],
        children: vec![
            TreeNode::Text(t("child")),
            TreeNode::Reference(t("7")),
            TreeNode::ErrorMarker(DiagnosticCode::Type),
            TreeNode::ErrorCell {
                name: t("Chart"),
                code: DiagnosticCode::MissingProp,
            },
        ],
    });
    Tree { blocks }
}

#[test]
fn every_node_and_value_kind_round_trips() {
    let tree = everything();
    let json = encode(&tree);
    assert_eq!(decode(&json).unwrap(), tree, "{json}");
    // Every binding type the evaluator writes round-trips.
    for kind in BINDING_TYPES {
        let tree = Tree {
            blocks: vec![TreeNode::Component {
                name: t("Slider"),
                props: vec![(
                    t("value"),
                    TreeValue::Binding {
                        state: t("x"),
                        kind,
                    },
                )],
                children: vec![],
            }],
        };
        assert_eq!(decode(&encode(&tree)).unwrap(), tree, "{kind}");
    }
}

#[test]
fn the_encoding_has_the_tree_abi_shape() {
    let tree = Tree {
        blocks: vec![
            markdown(
                MarkdownKind::Paragraph,
                vec![
                    TreeNode::Text(t("Hi ")),
                    TreeNode::Component {
                        name: t("Badge"),
                        props: vec![
                            (t("tone"), TreeValue::String(t("info"))),
                            (t("count"), num("3")),
                        ],
                        children: vec![TreeNode::Text(t("new"))],
                    },
                    TreeNode::Reference(t("7")),
                ],
            ),
            markdown(MarkdownKind::List { start: Some(2) }, vec![]),
            markdown(MarkdownKind::List { start: None }, vec![]),
            TreeNode::Component {
                name: t("Slider"),
                props: vec![(
                    t("value"),
                    TreeValue::Binding {
                        state: t("page"),
                        kind: "number",
                    },
                )],
                children: vec![],
            },
            TreeNode::ErrorCell {
                name: t("Chart"),
                code: DiagnosticCode::MissingProp,
            },
        ],
    };
    assert_eq!(
        encode(&tree),
        concat!(
            r#"{"m":"root","k":["#,
            r#"{"m":"p","k":[{"t":"Hi "},"#,
            r#"{"c":"Badge","p":{"tone":"info","count":{"number":"3"}},"k":[{"t":"new"}]},"#,
            r#"{"r":"7"}]},"#,
            r#"{"m":"list","ordered":true,"start":2,"k":[]},"#,
            r#"{"m":"list","ordered":false,"k":[]},"#,
            r#"{"c":"Slider","p":{"value":{"binding":{"state":"$page","type":"number"}}},"k":[]},"#,
            r#"{"c":"Chart","e":"E_MISSING_PROP"}"#,
            "]}"
        )
    );
}

#[test]
fn members_may_come_in_any_order() {
    let tree = decode(r#"{"k":[{"k":[],"p":{"b":true,"a":null},"c":"X"}],"m":"root"}"#).unwrap();
    assert_eq!(
        tree.blocks,
        [TreeNode::Component {
            name: t("X"),
            props: vec![
                (t("b"), TreeValue::Boolean(true)),
                (t("a"), TreeValue::Null)
            ],
            children: vec![],
        }]
    );
}

#[test]
fn text_is_sanitized_again_on_decoding() {
    let tree = decode(r#"{"m":"root","k":[{"t":"a\u0007b\tc"}]}"#).unwrap();
    assert_eq!(tree.blocks, [TreeNode::Text(t("a\u{FFFD}b\tc"))]);
}

#[test]
fn refuses_trees_that_are_not_tree_json() {
    let node = |node: &str| format!(r#"{{"m":"root","k":[{node}]}}"#);
    let prop = |value: &str| node(&format!(r#"{{"c":"X","p":{{"v":{value}}},"k":[]}}"#));
    let cases = [
        // The root.
        r#"{"m":"p","k":[]}"#.to_owned(),
        r#"{"m":"root"}"#.to_owned(),
        r#"{"m":"root","k":[],"c":"X"}"#.to_owned(),
        r#"{"m":"root","m":"root","k":[]}"#.to_owned(),
        // Nodes with a member they do not define, without one they need, or
        // with no kind.
        node(r#"{"t":"a","extra":1}"#),
        node(r#"{"t":"a","k":[]}"#),
        node(r#"{"t":null}"#),
        node(r#"{"t":1}"#),
        node(r#"{"c":"X","k":[]}"#),
        node(r#"{"c":"X","p":{}}"#),
        node(r#"{"c":"X","p":{},"k":[],"e":"E_TYPE"}"#),
        node(r#"{"c":"X","e":"E_NOT_A_CODE"}"#),
        node(r#"{"e":"E_TYPE","t":"a"}"#),
        node("{}"),
        node(r#"{"k":[]}"#),
        node(r#"{"m":"p"}"#),
        node(r#"{"m":"unknown","k":[]}"#),
        node(r#"{"m":"p","level":1,"k":[]}"#),
        node(r#"{"m":"h","k":[]}"#),
        node(r#"{"m":"h","level":256,"k":[]}"#),
        node(r#"{"m":"codeblock","text":"x","k":[]}"#),
        node(r#"{"m":"list","k":[]}"#),
        node(r#"{"m":"list","ordered":true,"k":[]}"#),
        node(r#"{"m":"list","ordered":false,"start":1,"k":[]}"#),
        node(r#"{"m":"link","href":"x","title":"","k":[]}"#),
        node(r#"{"m":"task","k":[]}"#),
        // Prop values.
        node(r#"{"c":"X","p":{"a":1,"a":2},"k":[]}"#),
        node(r#"{"c":"X","p":[],"k":[]}"#),
        prop("1"),
        prop("1.5"),
        prop(r#"{"number":1}"#),
        prop(r#"{"number":"1.0"}"#),
        prop(r#"{"number":"x"}"#),
        prop("{}"),
        prop(r#"{"unknown":1}"#),
        prop(r#"{"number":"1","error":"E_TYPE"}"#),
        prop(r#"{"record":{"a":null,"a":null}}"#),
        prop(r#"{"record":[]}"#),
        prop(r#"{"node":{"t":"a","r":"1"}}"#),
        prop(r#"{"action":[{"op":"write","args":[]}]}"#),
        prop(r#"{"action":[{"op":"set"}]}"#),
        prop(r#"{"action":[{"op":"set","args":[],"extra":1}]}"#),
        prop(r#"{"snapshot":{"record_id":"1"}}"#),
        prop(r#"{"snapshot":{"record_id":"1","state":"held","extra":1}}"#),
        prop(r#"{"handleRecord":null}"#),
        prop(r#"{"site":"history"}"#),
        prop(r#"{"binding":{"state":"page","type":"number"}}"#),
        prop(r#"{"binding":{"state":"$page","type":"date"}}"#),
        prop(r#"{"binding":{"state":"$page"}}"#),
        prop(r#"{"error":"E_NOT_A_CODE"}"#),
    ];
    for json in &cases {
        assert!(decode(json).is_err(), "{json}");
    }
}

#[test]
fn node_at_follows_child_indices() {
    let tree = everything();
    let card = tree.blocks.len() - 1;
    assert!(matches!(
        node_at(&tree, &[card]),
        Some(TreeNode::Component { .. })
    ));
    assert!(matches!(
        node_at(&tree, &[card, 1]),
        Some(TreeNode::Reference(_))
    ));
    assert!(matches!(node_at(&tree, &[0, 0]), Some(TreeNode::Text(_))));
    assert_eq!(node_at(&tree, &[]), None);
    assert_eq!(node_at(&tree, &[card + 1]), None);
    assert_eq!(node_at(&tree, &[0, 1]), None);
    // A text node has no children.
    assert_eq!(node_at(&tree, &[0, 0, 0]), None);
}
