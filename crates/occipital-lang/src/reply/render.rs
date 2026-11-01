use std::collections::BTreeMap;

use cortexkit_rich_ui_syntax::{Display as D, DisplayNode, Expression, Kind, Node};

use crate::eval::{
    snapshot::{self, ParseKey},
    tree::{MarkdownKind as M, Tree, TreeNode as T, TreeValue as V},
};
use crate::fallback::reply::{Content, Replacement};
use crate::prepare::resolve::Resolved;
use crate::{CheckContext, DiagnosticSpan, DisplayText, Severity, SourceSpan};

pub(super) fn promotion_fuel(picture: &cortexkit_rich_ui_syntax::Revision) -> u64 {
    fn cost(n: &DisplayNode) -> u64 {
        let scalars = match &n.kind {
            D::Text(s) | D::Literal(s) | D::Code(s) => s.chars().count() as u64,
            _ => 0,
        };
        1 + scalars.div_ceil(8) + n.children.iter().map(|n| cost(n)).sum::<u64>()
    }
    picture.nodes.iter().map(|n| cost(n)).sum()
}

pub(super) fn provisional(picture: &cortexkit_rich_ui_syntax::Revision) -> Tree {
    fn node(n: &DisplayNode) -> T {
        let children = n.children.iter().map(|n| node(n)).collect();
        let kind = match &n.kind {
            D::Text(s) | D::Literal(s) => return T::Text(DisplayText::new(s)),
            D::PendingReference => return T::Reference(DisplayText::new("pending")),
            D::Tag { name, .. } => {
                return T::Component {
                    name: DisplayText::new(name),
                    props: vec![],
                    children,
                }
            }
            D::Paragraph => M::Paragraph,
            D::Heading(n) => M::Heading(*n),
            D::ThematicBreak => M::ThematicBreak,
            D::CodeBlock { language } => M::CodeBlock {
                language: DisplayText::new(language),
                text: DisplayText::new(""),
            },
            D::Quote => M::Quote,
            D::List { start } => M::List { start: *start },
            D::Item => M::Item,
            D::Task(checked) => M::Task(*checked),
            D::Table => M::Table,
            D::TableHead => M::TableHead,
            D::TableRow => M::TableRow,
            D::TableCell => M::TableCell,
            D::Emphasis => M::Emphasis,
            D::Strong => M::Strong,
            D::Strikethrough => M::Strikethrough,
            D::Code(s) => M::Code(DisplayText::new(s)),
            D::Link { image } => M::Link {
                destination: DisplayText::new(""),
                title: DisplayText::new(""),
                image: *image,
            },
            D::SoftBreak => M::SoftBreak,
            D::HardBreak => M::HardBreak,
            _ => return T::Text(DisplayText::new("")),
        };
        T::Markdown { kind, children }
    }
    Tree {
        blocks: picture.nodes.iter().map(|n| node(n)).collect(),
    }
}

/// Catalog failures make an entire tag opaque, including its children (§2.1).
pub(super) fn literalize(program: &mut Resolved, source: &[u8]) {
    fn visit(
        node: &mut Node,
        failures: &[SourceSpan],
        source: &str,
        positions: &crate::prepare::Positions,
    ) {
        let extent = positions.span(node.span);
        if matches!(node.kind, Kind::Tag { .. })
            && failures.iter().any(|span| {
                (span.line, span.column) >= (extent.line, extent.column)
                    && (span.line, span.column) < (extent.end_line, extent.end_column)
            })
        {
            node.kind = Kind::Literal(
                source
                    .get(node.span.start..node.span.end)
                    .unwrap_or("")
                    .to_owned(),
            );
            node.children.clear();
            return;
        }
        for child in &mut node.children {
            visit(child, failures, source, positions);
        }
    }
    let failures: Vec<_> = program
        .diagnostics
        .iter()
        .filter(|d| d.severity() == Severity::Error)
        .filter_map(|d| match d.span() {
            Some(DiagnosticSpan::Source(span)) => Some(span),
            _ => None,
        })
        .collect();
    let source = String::from_utf8_lossy(crate::prepare::source_prefix(
        source,
        cortexkit_rich_ui_syntax::REPLY_BYTES,
    ));
    for node in &mut program.parsed.document.nodes {
        visit(node, &failures, &source, &program.parsed.positions);
    }
}

pub(super) fn literal_spans(program: &Resolved) -> Vec<SourceSpan> {
    fn visit(node: &Node, program: &Resolved, out: &mut Vec<SourceSpan>) {
        if matches!(node.kind, Kind::Literal(_)) {
            out.push(program.parsed.positions.span(node.span));
        }
        for child in &node.children {
            visit(child, program, out);
        }
    }
    let mut out = vec![];
    for node in &program.parsed.document.nodes {
        visit(node, program, &mut out);
    }
    out
}

pub(crate) fn parse_span(program: &Resolved, key: &ParseKey) -> Option<SourceSpan> {
    fn find(node: &Node, key: &ParseKey) -> Option<cortexkit_rich_ui_syntax::Span> {
        if matches!(&node.kind, Kind::Expression(Expression::Call(name)) if name == "Parse")
            && matches!(node.children.len(), 2 | 3)
            && snapshot::parse_key(node).as_ref() == Ok(key)
        {
            return Some(node.span);
        }
        if let Kind::Tag { props, .. } = &node.kind {
            for prop in props {
                if let Some(span) = find(&prop.value, key) {
                    return Some(span);
                }
            }
        }
        node.children.iter().find_map(|node| find(node, key))
    }
    let span = program
        .parsed
        .document
        .nodes
        .iter()
        .find_map(|n| find(n, key))?;
    Some(program.parsed.positions.span(span))
}

pub(super) fn late_states(
    tree: &mut Tree,
    context: &CheckContext,
    fetched: &BTreeMap<u64, Result<Vec<u8>, ()>>,
) {
    fn value(v: &mut V, context: &CheckContext, fetched: &BTreeMap<u64, Result<Vec<u8>, ()>>) {
        match v {
            V::Snapshot {
                record_id, state, ..
            } => {
                if state.as_str() == "resolved"
                    && record_id
                        .as_str()
                        .parse::<u64>()
                        .ok()
                        .is_some_and(|n| !fetched.contains_key(&n))
                {
                    *state = DisplayText::new("loading");
                }
            }
            V::Handle(id) => {
                let ready = context.handles.iter().any(|h| {
                    h.record_id == id.as_str()
                        && h.available
                        && h.resource_id.is_some()
                        && h.version.is_some()
                });
                *v = V::Snapshot {
                    record_id: id.clone(),
                    state: DisplayText::new(if ready { "resolved" } else { "unavailable" }),
                    value: None,
                };
            }
            V::List(values) => {
                for v in values {
                    value(v, context, fetched);
                }
            }
            V::Record(fields) => {
                for (_, v) in fields {
                    value(v, context, fetched);
                }
            }
            V::Node(n) => node(n, context, fetched),
            _ => (),
        }
    }
    fn node(n: &mut T, context: &CheckContext, fetched: &BTreeMap<u64, Result<Vec<u8>, ()>>) {
        match n {
            T::Component {
                props, children, ..
            } => {
                for (_, v) in props {
                    value(v, context, fetched);
                }
                for n in children {
                    node(n, context, fetched);
                }
            }
            T::Markdown { children, .. } => {
                for n in children {
                    node(n, context, fetched);
                }
            }
            T::Reference(id) => {
                let number = id.as_str().parse::<u64>().ok();
                let record = number.and_then(|n| context.references.get(&n));
                let state = match record.map(|r| r.state) {
                    Some(crate::ReferenceState::Resolved) => {
                        match number.and_then(|n| fetched.get(&n)) {
                            Some(Ok(_)) => "resolved",
                            Some(Err(())) => "unavailable",
                            None => "loading",
                        }
                    }
                    Some(crate::ReferenceState::Held) => "held",
                    _ => "unavailable",
                };
                *n = T::Component {
                    name: DisplayText::new("ReferenceChip"),
                    props: vec![(
                        DisplayText::new("source"),
                        V::Snapshot {
                            record_id: id.clone(),
                            state: DisplayText::new(state),
                            value: None,
                        },
                    )],
                    children: vec![],
                };
            }
            _ => (),
        }
    }
    for n in &mut tree.blocks {
        node(n, context, fetched);
    }
}

pub(super) fn render_limits(tree: &Tree, diagnostics: &mut Vec<crate::Diagnostic>) {
    fn visit(node: &T, diagnostics: &mut Vec<crate::Diagnostic>) {
        match node {
            T::Component {
                name,
                props,
                children,
            } => {
                if crate::geometry::render_state(name.as_str(), props)
                    == crate::geometry::RenderState::Error(crate::DiagnosticCode::RenderLimit)
                {
                    diagnostics.push(
                        crate::Diagnostic::new(
                            crate::DiagnosticCode::RenderLimit,
                            crate::DiagnosticPhase::Evaluate,
                        )
                        .with_subject(name.as_str()),
                    );
                }
                for child in children {
                    visit(child, diagnostics);
                }
            }
            T::Markdown { children, .. } => {
                for child in children {
                    visit(child, diagnostics);
                }
            }
            _ => (),
        }
    }
    for node in &tree.blocks {
        visit(node, diagnostics);
    }
}

pub(super) fn plain_text(
    program: &Resolved,
    source: &[u8],
    tree: &Tree,
    context: &CheckContext,
) -> DisplayText {
    fn components<'a>(nodes: &'a [T], out: &mut Vec<&'a T>) {
        for node in nodes {
            match node {
                T::Component { name, children, .. } => {
                    if name.as_str() != "ReferenceChip" {
                        out.push(node);
                    }
                    components(children, out);
                }
                T::Markdown { children, .. } => components(children, out),
                _ => (),
            }
        }
    }
    fn text(nodes: &[T]) -> String {
        let mut out = String::new();
        for node in nodes {
            match node {
                T::Text(s) => out.push_str(s.as_str()),
                T::Markdown {
                    kind: M::Code(s), ..
                } => out.push_str(s.as_str()),
                T::Component { children, .. } | T::Markdown { children, .. } => {
                    out.push_str(&text(children))
                }
                _ => (),
            }
        }
        out
    }
    fn rows(v: &V) -> usize {
        match v {
            V::List(values) => values.len(),
            V::Snapshot { value: Some(v), .. } => rows(v),
            _ => 0,
        }
    }
    struct Owned {
        start: usize,
        end: usize,
        name: String,
        fields: Vec<(String, String)>,
        tool: Option<Option<String>>,
    }
    fn visit(
        node: &Node,
        tree: &[&T],
        index: &mut usize,
        context: &CheckContext,
        out: &mut Vec<Owned>,
        citations: &mut Vec<String>,
    ) {
        match &node.kind {
            Kind::Tag { name, .. } => {
                let Some(T::Component {
                    props, children, ..
                }) = tree.get(*index).copied()
                else {
                    return;
                };
                *index += 1;
                let mut fields = vec![];
                for (key, value) in props {
                    if let V::String(s) = value {
                        fields.push((key.as_str().to_owned(), s.as_str().to_owned()));
                    }
                }
                let get = |key: &str| {
                    props
                        .iter()
                        .find(|(k, _)| k.as_str() == key)
                        .map(|(_, v)| v)
                };
                let body = text(children);
                fields.extend([
                    ("children".into(), body.clone()),
                    ("body".into(), body.clone()),
                ]);
                if name == "Badge" {
                    let label = fields
                        .iter()
                        .find(|(k, _)| k == "text")
                        .map(|(_, v)| v.clone())
                        .unwrap_or(body);
                    fields.push(("label".into(), label));
                }
                let count = if name == "ImageGroup" {
                    get("images").map(rows).unwrap_or(0)
                } else {
                    get(if name == "Map" { "places" } else { "data" })
                        .map(rows)
                        .unwrap_or(0)
                };
                fields.push(("n".into(), count.to_string()));
                fields.push(("N".into(), count.to_string()));
                if name == "Citation" {
                    let identity = match get("source") {
                        Some(V::Snapshot { record_id, .. }) => record_id.as_str().to_owned(),
                        _ => String::new(),
                    };
                    if !citations.contains(&identity) {
                        citations.push(identity.clone());
                    }
                    fields.push((
                        "index".into(),
                        (citations.iter().position(|id| id == &identity).unwrap_or(0) + 1)
                            .to_string(),
                    ));
                }
                out.push(Owned {
                    start: node.span.start,
                    end: node.span.end,
                    name: name.clone(),
                    fields,
                    tool: None,
                });
                // The parent's catalog form already includes its children.
                let mut ignored = vec![];
                for child in &node.children {
                    visit(child, tree, index, context, &mut ignored, citations);
                }
            }
            Kind::Reference { n } => out.push(Owned {
                start: node.span.start,
                end: node.span.end,
                name: String::new(),
                fields: vec![],
                tool: Some(
                    n.parse::<u64>()
                        .ok()
                        .and_then(|n| context.references.get(&n))
                        .and_then(|r| r.tool.clone()),
                ),
            }),
            _ => {
                for child in &node.children {
                    visit(child, tree, index, context, out, citations);
                }
            }
        }
    }
    let mut nodes = vec![];
    components(&tree.blocks, &mut nodes);
    let mut owned = vec![];
    let mut index = 0;
    let mut citations = vec![];
    for node in &program.parsed.document.nodes {
        visit(
            node,
            &nodes,
            &mut index,
            context,
            &mut owned,
            &mut citations,
        );
    }
    // Syntax addresses repaired UTF-8; the fallback scanner addresses raw bytes.
    // Translate just the replacement boundaries in one forward repair pass, so
    // malformed bytes neither shift a later tag nor consume its raw-source cap.
    let mut offsets: BTreeMap<usize, usize> = owned
        .iter()
        .flat_map(|o| [(o.start, 0), (o.end, 0)])
        .collect();
    let (mut repaired, mut raw) = (0, 0);
    for chunk in
        crate::prepare::source_prefix(source, cortexkit_rich_ui_syntax::REPLY_BYTES).utf8_chunks()
    {
        let valid = chunk.valid().len();
        for (offset, position) in offsets.range_mut(repaired..=repaired + valid) {
            *position = raw + (*offset - repaired);
        }
        raw += valid;
        repaired += valid;
        if !chunk.invalid().is_empty() {
            for (_, position) in offsets.range_mut(repaired + 1..=repaired + 3) {
                *position = raw + chunk.invalid().len();
            }
            raw += chunk.invalid().len();
            repaired += 3;
        }
    }
    for o in &mut owned {
        // Every start and end was inserted as a key above, so both lookups hit.
        let (Some(&start), Some(&end)) = (offsets.get(&o.start), offsets.get(&o.end)) else {
            continue;
        };
        o.start = start;
        o.end = end;
    }
    let fields: Vec<Vec<(&str, &str)>> = owned
        .iter()
        .map(|o| {
            o.fields
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect()
        })
        .collect();
    let replacements: Vec<_> = owned
        .iter()
        .zip(&fields)
        .map(|(o, fields)| Replacement {
            start: o.start,
            end: o.end,
            content: if let Some(tool) = &o.tool {
                Content::Reference {
                    tool: tool.as_deref(),
                }
            } else {
                Content::Tag {
                    name: &o.name,
                    fields,
                }
            },
        })
        .collect();
    crate::fallback::reply::plain_text(source, &replacements).text
}

/// A value can pass the static prop type check but fail evaluation (for example,
/// an address with a forbidden scheme). Replies keep that tag's source opaque,
/// rather than adopting the program evaluator's generic error cell (§2.1).
pub(super) fn failed_cells(program: &mut Resolved, tree: &mut Tree, source: &[u8]) -> bool {
    fn tags(node: &Node, out: &mut Vec<(String, cortexkit_rich_ui_syntax::Span)>) {
        if let Kind::Tag { name, .. } = &node.kind {
            out.push((name.clone(), node.span));
        }
        for child in &node.children {
            tags(child, out);
        }
    }
    fn visit(
        node: &mut T,
        tags: &[(String, cortexkit_rich_ui_syntax::Span)],
        index: &mut usize,
        failed: &mut Vec<cortexkit_rich_ui_syntax::Span>,
        source: &str,
    ) {
        match node {
            T::Component { name, children, .. } => {
                if tags
                    .get(*index)
                    .is_some_and(|(tag, _)| tag == name.as_str())
                {
                    *index += 1;
                }
                for child in children {
                    visit(child, tags, index, failed, source);
                }
            }
            T::ErrorCell { name, .. } => {
                if let Some((_, span)) = tags.get(*index).filter(|(tag, _)| tag == name.as_str()) {
                    *index += 1;
                    failed.push(*span);
                    *node = T::Text(DisplayText::new(
                        source.get(span.start..span.end).unwrap_or(""),
                    ));
                    while tags
                        .get(*index)
                        .is_some_and(|(_, child)| child.start < span.end)
                    {
                        *index += 1;
                    }
                }
            }
            T::Markdown { children, .. } => {
                for child in children {
                    visit(child, tags, index, failed, source);
                }
            }
            _ => (),
        }
    }
    fn replace(node: &mut Node, failed: &[cortexkit_rich_ui_syntax::Span], source: &str) {
        if failed.contains(&node.span) && matches!(node.kind, Kind::Tag { .. }) {
            node.kind = Kind::Literal(
                source
                    .get(node.span.start..node.span.end)
                    .unwrap_or("")
                    .to_owned(),
            );
            node.children.clear();
        } else {
            for child in &mut node.children {
                replace(child, failed, source);
            }
        }
    }
    let mut source_tags = vec![];
    for node in &program.parsed.document.nodes {
        tags(node, &mut source_tags);
    }
    let source = String::from_utf8_lossy(crate::prepare::source_prefix(
        source,
        cortexkit_rich_ui_syntax::REPLY_BYTES,
    ));
    let mut failed = vec![];
    let mut index = 0;
    for node in &mut tree.blocks {
        visit(node, &source_tags, &mut index, &mut failed, &source);
    }
    for node in &mut program.parsed.document.nodes {
        replace(node, &failed, &source);
    }
    !failed.is_empty()
}
