use cortexkit_rich_ui_syntax::{parse, Expression, Kind, Mode, Node};

fn text(nodes: &[Node]) -> String {
    let mut result = String::new();
    for node in nodes {
        if let Kind::Text(value) = &node.kind {
            result.push_str(value);
        }
        result.push_str(&text(&node.children));
    }
    result
}

#[test]
fn syntax_decodes_section_3_4_entities() {
    // Entity decoding belongs to the pinned syntax dependency. In particular,
    // references beside text must not bypass its Markdown decoding path.
    for (source, expected) in [
        (
            "&amp; &NotEqualTilde; &#65; &#x1F600; &#xD800; &#1114112; &bogus; &#xQ;",
            "& ≂\u{338} A 😀 � � &bogus; &#xQ;",
        ),
        ("<Badge>&amp; &#65; &#xD800;</Badge>", "& A �"),
        ("&amp; $$tc(1) &#65;", "&  A"),
        ("<Badge>&amp; $$tc(1) &#65;</Badge>", "&  A"),
    ] {
        let doc = parse(source, Mode::Reply);
        assert!(doc.diagnostics.is_empty(), "{doc:?}");
        assert_eq!(text(&doc.nodes), expected, "{source}: {doc:?}");
    }
}

#[test]
fn quoted_props_are_string_literals_not_markdown() {
    let doc = parse("<Badge label=\"&amp; &#65;\">&amp;</Badge>", Mode::Reply);
    assert!(doc.diagnostics.is_empty(), "{doc:?}");
    fn find_props(nodes: &[Node]) -> Option<&[cortexkit_rich_ui_syntax::Prop]> {
        nodes.iter().find_map(|node| match &node.kind {
            Kind::Tag { props, .. } => Some(props.as_slice()),
            _ => find_props(&node.children),
        })
    }
    let props = find_props(&doc.nodes).expect("expected a tag");
    assert_eq!(
        props[0].value.kind,
        Kind::Expression(Expression::String("&amp; &#65;".into()))
    );
    assert_eq!(text(&doc.nodes), "&");
}
