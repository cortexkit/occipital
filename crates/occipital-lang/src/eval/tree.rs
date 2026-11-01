//! The output tree (§5.6, §14.3 tree ABI) and how values are placed in it
//! (§3.12). Every string in the tree is sanitized display text (§5.6).

use cortexkit_rich_ui_syntax::{Markdown, Node};

use super::{Abort, Evaluator, TREE_DEPTH, TREE_NODES, TREE_TEXT};
use crate::catalog::{Catalog, Prop as PropSchema};
use crate::check::catalog::evaluated_bounds;
use crate::{
    ActionOperation, Address, Component, Decimal, Diagnostic, DiagnosticCode, DiagnosticPhase,
    DisplayText, SourceSpan, Value, ValueError, ValueKind,
};

/// The root of one evaluated tree: the body's blocks in order (§5.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    /// The body's blocks, in source order.
    pub blocks: Vec<TreeNode>,
}

/// One output node. Each counts once against the node limit (§9.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeNode {
    /// A Markdown node.
    Markdown {
        /// Which Markdown node.
        kind: MarkdownKind,
        /// Its children, in order.
        children: Vec<TreeNode>,
    },
    /// A catalog component with its given props in source order, followed by
    /// its bindings. A prop that was absent is not listed: the catalog
    /// default applies (§5.6).
    Component {
        /// The component's catalog name.
        name: DisplayText,
        /// Props in source order, then bindings.
        props: Vec<(DisplayText, TreeValue)>,
        /// Its children, in order.
        children: Vec<TreeNode>,
    },
    /// Sanitized text.
    Text(DisplayText),
    /// A reference written in Markdown text, drawn as a chip (§3.8).
    Reference(DisplayText),
    /// A fixed-size inline marker showing an error code (§3.12, §4.6).
    ErrorMarker(DiagnosticCode),
    /// A component with an error in a prop, or a required prop not given,
    /// drawn as an error cell without its children (§5.6).
    ErrorCell {
        /// The component's catalog name.
        name: DisplayText,
        /// The error drawn in the cell.
        code: DiagnosticCode,
    },
}

/// Markdown node kinds of the tree ABI (§14.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownKind {
    /// A paragraph.
    Paragraph,
    /// A heading of this level, 1 to 6.
    Heading(u8),
    /// A thematic break.
    ThematicBreak,
    /// A fenced or indented code block.
    CodeBlock {
        /// The info string's language, possibly empty.
        language: DisplayText,
        /// The code.
        text: DisplayText,
    },
    /// An inline code span.
    Code(DisplayText),
    /// A block quote.
    Quote,
    /// A list.
    List {
        /// The first number of an ordered list; none for a bullet list.
        start: Option<u64>,
    },
    /// A list item.
    Item,
    /// A task list item, checked or not.
    Task(bool),
    /// A GFM table.
    Table,
    /// A table's header row group.
    TableHead,
    /// A table row.
    TableRow,
    /// A table cell.
    TableCell,
    /// Emphasis.
    Emphasis,
    /// Strong emphasis.
    Strong,
    /// Strikethrough.
    Strikethrough,
    /// The destination as written; opening goes through the address rules
    /// of §3.11 when the link is used.
    Link {
        /// The destination as written.
        destination: DisplayText,
        /// The link title, possibly empty.
        title: DisplayText,
        /// Whether this is an image rather than a link.
        image: bool,
    },
    /// A soft line break.
    SoftBreak,
    /// A hard line break.
    HardBreak,
}

/// A prop value in the tree. Data keeps its shape; components become nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeValue {
    /// `null`.
    Null,
    /// A number.
    Number(Decimal),
    /// Sanitized text.
    String(DisplayText),
    /// A boolean.
    Boolean(bool),
    /// A list, in order.
    List(Vec<TreeValue>),
    /// A record, in insertion order.
    Record(Vec<(DisplayText, TreeValue)>),
    /// A component or Markdown node held in a prop.
    Node(Box<TreeNode>),
    /// An action: its frozen steps in order.
    Action(Vec<TreeStep>),
    /// A snapshot view: its record id, state and parsed value, if any. The raw
    /// text stays in the host's snapshot table.
    Snapshot {
        /// The snapshot's record id.
        record_id: DisplayText,
        /// `resolved`, `unresolved`, `too_large` or `held`.
        state: DisplayText,
        /// The parsed value when a format hint parsed the snapshot.
        value: Option<Box<TreeValue>>,
    },
    /// A handle, by its record id (§4.9).
    Handle(DisplayText),
    /// A site prop, by site name.
    Site(DisplayText),
    /// A `binding<T>` prop: the state a control writes and its type (§14.3).
    Binding {
        /// The state the control writes, without `$`.
        state: DisplayText,
        /// The state's type, such as `number`.
        kind: &'static str,
    },
    /// An error inside data, or an action prop that draws disabled (§7.1).
    Error(DiagnosticCode),
}

/// One frozen action step (§7.1). `Set` and `Reset` name their state first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeStep {
    /// What the step does.
    pub operation: ActionOperation,
    /// The step's frozen arguments.
    pub arguments: Vec<TreeValue>,
}

/// The name of the value standing for a reference chip in tag content.
pub(super) const REFERENCE: &str = "#ref";

/// A Markdown attribute stored on its content value.
pub(super) enum Attribute {
    Text(String),
    Number(Decimal),
    Boolean(bool),
}

/// Markdown in a tag's content becomes a value named after its kind, with
/// its attributes as props. The names start with `#`, which no catalog
/// component can (tag names start with an uppercase letter, §3.5).
pub(super) type MarkdownContent = (&'static str, Vec<(&'static str, Attribute)>);

pub(super) fn pseudo(markdown: &Markdown) -> Option<MarkdownContent> {
    let text = |text: &str| Attribute::Text(text.to_owned());
    Some(match markdown {
        Markdown::Paragraph => ("#p", vec![]),
        Markdown::Heading(level) => (
            "#h",
            vec![("level", Attribute::Number(integer(u64::from(*level))))],
        ),
        Markdown::ThematicBreak => ("#hr", vec![]),
        Markdown::CodeBlock {
            language,
            text: code,
        } => (
            "#codeblock",
            vec![("language", text(language)), ("text", text(code))],
        ),
        Markdown::Code(code) => ("#code", vec![("text", text(code))]),
        Markdown::Quote => ("#quote", vec![]),
        Markdown::List { start } => (
            "#list",
            start
                .map(|start| vec![("start", Attribute::Number(integer(start)))])
                .unwrap_or_default(),
        ),
        Markdown::Item => ("#item", vec![]),
        Markdown::Task(checked) => ("#task", vec![("checked", Attribute::Boolean(*checked))]),
        Markdown::Table => ("#table", vec![]),
        Markdown::TableHead => ("#thead", vec![]),
        Markdown::TableRow => ("#row", vec![]),
        Markdown::TableCell => ("#cell", vec![]),
        Markdown::Emphasis => ("#em", vec![]),
        Markdown::Strong => ("#strong", vec![]),
        Markdown::Strikethrough => ("#del", vec![]),
        Markdown::Link {
            destination,
            title,
            image,
        } => (
            "#link",
            vec![
                ("destination", text(destination)),
                ("title", text(title)),
                ("image", Attribute::Boolean(*image)),
            ],
        ),
        Markdown::SoftBreak => ("#softbreak", vec![]),
        Markdown::HardBreak => ("#hardbreak", vec![]),
        _ => return None,
    })
}

fn integer(value: u64) -> Decimal {
    Decimal::from_integer(i128::from(value)).unwrap_or(Decimal::ZERO)
}

pub(crate) fn kind_of(markdown: &Markdown) -> Option<MarkdownKind> {
    Some(match markdown {
        Markdown::Paragraph => MarkdownKind::Paragraph,
        Markdown::Heading(level) => MarkdownKind::Heading(*level),
        Markdown::ThematicBreak => MarkdownKind::ThematicBreak,
        Markdown::CodeBlock { language, text } => MarkdownKind::CodeBlock {
            language: DisplayText::new(language),
            text: DisplayText::new(text),
        },
        Markdown::Code(text) => MarkdownKind::Code(DisplayText::new(text)),
        Markdown::Quote => MarkdownKind::Quote,
        Markdown::List { start } => MarkdownKind::List { start: *start },
        Markdown::Item => MarkdownKind::Item,
        Markdown::Task(checked) => MarkdownKind::Task(*checked),
        Markdown::Table => MarkdownKind::Table,
        Markdown::TableHead => MarkdownKind::TableHead,
        Markdown::TableRow => MarkdownKind::TableRow,
        Markdown::TableCell => MarkdownKind::TableCell,
        Markdown::Emphasis => MarkdownKind::Emphasis,
        Markdown::Strong => MarkdownKind::Strong,
        Markdown::Strikethrough => MarkdownKind::Strikethrough,
        Markdown::Link {
            destination,
            title,
            image,
        } => MarkdownKind::Link {
            destination: DisplayText::new(destination),
            title: DisplayText::new(title),
            image: *image,
        },
        Markdown::SoftBreak => MarkdownKind::SoftBreak,
        Markdown::HardBreak => MarkdownKind::HardBreak,
        _ => return None,
    })
}

/// The Markdown kind a content value stands for, from its name and props.
fn kind_from(component: &Component) -> Option<MarkdownKind> {
    let text = |key: &str| match component.props.get(key).kind() {
        ValueKind::String(text) => text.as_str().to_owned(),
        _ => String::new(),
    };
    let flag = |key: &str| matches!(component.props.get(key).kind(), ValueKind::Boolean(true));
    let number = |key: &str| match component.props.get(key).kind() {
        ValueKind::Number(number) => number.integer(),
        _ => None,
    };
    let markdown = match &*component.name {
        "#p" => Markdown::Paragraph,
        "#h" => Markdown::Heading(number("level").and_then(|n| u8::try_from(n).ok())?),
        "#hr" => Markdown::ThematicBreak,
        "#codeblock" => Markdown::CodeBlock {
            language: text("language"),
            text: text("text"),
        },
        "#code" => Markdown::Code(text("text")),
        "#quote" => Markdown::Quote,
        "#list" => Markdown::List {
            start: number("start").and_then(|n| u64::try_from(n).ok()),
        },
        "#item" => Markdown::Item,
        "#task" => Markdown::Task(flag("checked")),
        "#table" => Markdown::Table,
        "#thead" => Markdown::TableHead,
        "#row" => Markdown::TableRow,
        "#cell" => Markdown::TableCell,
        "#em" => Markdown::Emphasis,
        "#strong" => Markdown::Strong,
        "#del" => Markdown::Strikethrough,
        "#link" => Markdown::Link {
            destination: text("destination"),
            title: text("title"),
            image: flag("image"),
        },
        "#softbreak" => Markdown::SoftBreak,
        "#hardbreak" => Markdown::HardBreak,
        _ => return None,
    };
    kind_of(&markdown)
}

/// Text a Markdown node carries itself, counted as output text.
fn own_text(kind: &MarkdownKind) -> u64 {
    match kind {
        MarkdownKind::Code(text) | MarkdownKind::CodeBlock { text, .. } => {
            text.as_str().chars().count() as u64
        }
        _ => 0,
    }
}

/// A paragraph holding only line breaks or white space. Removing
/// declarations from the body leaves these between them; they show nothing.
pub(super) fn insignificant(node: &Node) -> bool {
    use cortexkit_rich_ui_syntax::Kind;
    matches!(node.kind, Kind::Markdown(Markdown::Paragraph))
        && node.children.iter().all(|child| match &child.kind {
            Kind::Markdown(Markdown::SoftBreak | Markdown::HardBreak) => true,
            Kind::Text(text) => text.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r')),
            _ => false,
        })
}

/// Whether a value sits inside a paragraph rather than standing as a block.
fn inline(value: &Value) -> bool {
    match value.kind() {
        ValueKind::Component(component) => match &*component.name {
            "#code" | "#em" | "#strong" | "#del" | "#link" | "#softbreak" | "#hardbreak"
            | REFERENCE => true,
            name if name.starts_with('#') => false,
            name => Catalog::tool_templates()
                .components
                .get(name)
                .is_some_and(|schema| schema.flow == "inline"),
        },
        _ => true,
    }
}

/// Children normalization (§5.6): nested lists flattened in order, absent
/// elements dropped. Empty strings insert no text.
fn flatten(value: &Value, items: &mut Vec<Value>) {
    match value.kind() {
        ValueKind::List(values) => {
            for value in values.iter() {
                flatten(value, items);
            }
        }
        ValueKind::Null | ValueKind::Missing => {}
        ValueKind::String(text) if text.is_empty() => {}
        _ => items.push(value.clone()),
    }
}

impl<'a> Evaluator<'a> {
    /// The body, block by block in source order (§5.3 rule 1).
    pub(super) fn body(&mut self) -> Result<Tree, Abort> {
        let mut blocks = Vec::new();
        let program = self.program;
        for node in &program.parsed.document.nodes {
            self.source(node, &mut blocks, 1)?;
        }
        Ok(Tree { blocks })
    }

    /// Admit one output node before creating it (§5.6, §9.4): its depth,
    /// the node count and its text, then the expansion fuel (§9.2).
    fn add(&mut self, depth: u32, text: u64, fuel: u64) -> Result<(), Abort> {
        if depth > TREE_DEPTH {
            return Err(Abort::LimitDepth);
        }
        if self.meter.nodes >= TREE_NODES {
            return Err(Abort::LimitNodes);
        }
        if self.meter.text.saturating_add(text) > TREE_TEXT {
            return Err(Abort::LimitText);
        }
        self.charge(fuel)?;
        self.meter.nodes += 1;
        self.meter.text += text;
        Ok(())
    }

    fn source(&mut self, node: &'a Node, out: &mut Vec<TreeNode>, depth: u32) -> Result<(), Abort> {
        use cortexkit_rich_ui_syntax::Kind;
        match &node.kind {
            Kind::Declaration { .. } => Ok(()),
            Kind::Markdown(markdown) => {
                if insignificant(node) {
                    return Ok(());
                }
                self.markdown_source(node, kind_of(markdown), out, depth)
            }
            Kind::Text(text) | Kind::Literal(text) => self.text(text, out, depth),
            Kind::Reference { n } => {
                self.add(depth, 0, 1)?;
                out.push(TreeNode::Reference(DisplayText::new(n)));
                Ok(())
            }
            Kind::Tag { block, .. } => {
                let value = self.expr(node)?;
                self.place(&value, *block, out, depth, Some(self.span(node)))
            }
            Kind::Interpolation { block } => {
                let value = self.expr(&node.children[0])?;
                self.place(&value, *block, out, depth, Some(self.span(node)))
            }
            _ => {
                let value = self.expr(node)?;
                self.place(&value, false, out, depth, Some(self.span(node)))
            }
        }
    }

    pub(super) fn markdown_source(
        &mut self,
        node: &'a Node,
        kind: Option<MarkdownKind>,
        out: &mut Vec<TreeNode>,
        depth: u32,
    ) -> Result<(), Abort> {
        let Some(kind) = kind else {
            let value = self.unsupported(node)?;
            return self.place(&value, false, out, depth, Some(self.span(node)));
        };
        let text = own_text(&kind);
        // Adding a Markdown node: 1 plus ⌈L/8⌉ for its text (§9.2).
        self.add(depth, text, 1 + text.div_ceil(8))?;
        let mut children = Vec::new();
        for child in &node.children {
            self.source(child, &mut children, depth + 1)?;
        }
        out.push(TreeNode::Markdown { kind, children });
        Ok(())
    }

    /// Insert a value (§3.12). In block position each run of inline items
    /// becomes one paragraph, so text inserted as a block is a paragraph of
    /// its own and an inline-only component placed as a block sits inside a
    /// paragraph (§3.5).
    fn place(
        &mut self,
        value: &Value,
        block: bool,
        out: &mut Vec<TreeNode>,
        depth: u32,
        span: Option<SourceSpan>,
    ) -> Result<(), Abort> {
        let mut items = Vec::new();
        flatten(value, &mut items);
        let mut index = 0;
        while index < items.len() {
            if !block || !inline(&items[index]) {
                self.item(&items[index], out, depth, span)?;
                index += 1;
                continue;
            }
            let end = items[index..]
                .iter()
                .position(|item| !inline(item))
                .map_or(items.len(), |offset| index + offset);
            self.add(depth, 0, 1)?;
            let mut children = Vec::new();
            for item in &items[index..end] {
                self.item(item, &mut children, depth + 1, span)?;
            }
            out.push(TreeNode::Markdown {
                kind: MarkdownKind::Paragraph,
                children,
            });
            index = end;
        }
        Ok(())
    }

    fn text(&mut self, text: &str, out: &mut Vec<TreeNode>, depth: u32) -> Result<(), Abort> {
        let scalars = text.chars().count() as u64;
        self.add(depth, scalars, 1 + scalars.div_ceil(8))?;
        out.push(TreeNode::Text(DisplayText::new(text)));
        Ok(())
    }

    /// One normalized item (§3.12, §5.6).
    fn item(
        &mut self,
        value: &Value,
        out: &mut Vec<TreeNode>,
        depth: u32,
        span: Option<SourceSpan>,
    ) -> Result<(), Abort> {
        match value.kind() {
            ValueKind::String(text) => self.text(text.as_str(), out, depth),
            ValueKind::Number(_) | ValueKind::Boolean(_) => {
                let text = value
                    .to_text(span.unwrap_or(PLACEHOLDER))
                    .unwrap_or_default();
                self.text(&text, out, depth)
            }
            ValueKind::Component(component) => self.node(component, out, depth),
            ValueKind::Error(error) => {
                self.report(error, None);
                self.marker(super::code(&error.code), out, depth)
            }
            ValueKind::List(_) | ValueKind::Null | ValueKind::Missing => Ok(()),
            ValueKind::Action(_) => {
                self.prop_problem(DiagnosticCode::ActionPlacement, "action");
                self.marker(DiagnosticCode::ActionPlacement, out, depth)
            }
            // Records, snapshots, handles and site envelopes have no
            // place in text: an E_TYPE marker (§3.12).
            _ => {
                match span {
                    Some(span) => self.report(&ValueError::new("E_TYPE", span), None),
                    None => self.push(Diagnostic::new(
                        DiagnosticCode::Type,
                        DiagnosticPhase::Evaluate,
                    )),
                }
                self.marker(DiagnosticCode::Type, out, depth)
            }
        }
    }

    fn marker(
        &mut self,
        code: DiagnosticCode,
        out: &mut Vec<TreeNode>,
        depth: u32,
    ) -> Result<(), Abort> {
        self.add(depth, 0, 1)?;
        out.push(TreeNode::ErrorMarker(code));
        Ok(())
    }

    /// A component value as a tree node, with its props checked (§5.6).
    fn node(
        &mut self,
        component: &Component,
        out: &mut Vec<TreeNode>,
        depth: u32,
    ) -> Result<(), Abort> {
        if component.name.starts_with('#') {
            return self.markdown_value(component, out, depth);
        }
        let name = DisplayText::new(&component.name);
        let Some(schema) = Catalog::tool_templates().components.get(&*component.name) else {
            self.prop_problem(DiagnosticCode::UnknownComponent, &component.name);
            self.add(depth, 0, 4)?;
            out.push(TreeNode::ErrorCell {
                name,
                code: DiagnosticCode::UnknownComponent,
            });
            return Ok(());
        };
        let mut given: Vec<(&str, &Value, bool)> = Vec::new();
        let mut failure = None;
        // A Slot's `needs` is never drawn, only whether it is waiting for it:
        // the tree carries `waiting: true` instead, and no children (see the
        // tool catalog's Slot entry).
        let slot = &*component.name == "Slot";
        let waiting = slot
            && component
                .props
                .fields()
                .iter()
                .any(|(key, value)| key.as_str() == "needs" && value.is_absent());
        for (key, value) in component.props.fields() {
            let key = key.as_str();
            if slot && key == "needs" {
                continue;
            }
            let Some(prop) = schema.props.get(key).filter(|_| key != "children") else {
                continue;
            };
            if value.is_absent() {
                continue;
            }
            if prop.t == "action" {
                given.push((key, value, true));
                continue;
            }
            let problem = match value.kind() {
                ValueKind::Error(error) => {
                    self.report(error, Some(key));
                    Some(super::code(&error.code))
                }
                _ => match self.fits(&component.name, key, prop, value) {
                    Some(code) => {
                        self.prop_problem(code, key);
                        Some(code)
                    }
                    None => None,
                },
            };
            if problem.is_some() {
                failure = failure.or(problem);
            }
            given.push((key, value, false));
        }
        for (key, prop) in &schema.props {
            let required = prop.rules.get("req") == Some(&serde_json::Value::Bool(true));
            if required
                && prop.t != "action"
                && !given.iter().any(|(name, _, _)| name == key)
                && !component.bindings.iter().any(|(name, _)| &**name == key)
            {
                self.prop_problem(DiagnosticCode::MissingProp, key);
                failure = failure.or(Some(DiagnosticCode::MissingProp));
            }
        }
        let props = (given.len() + component.bindings.len()) as u64;
        if let Some(code) = failure {
            self.add(depth, 0, 4 + props)?;
            out.push(TreeNode::ErrorCell { name, code });
            return Ok(());
        }
        if !self.inputs.network {
            let mut addresses = Vec::new();
            for (_, value, action) in &given {
                if *action {
                    if let ValueKind::Action(action) = value.kind() {
                        addresses.extend(crate::actions::address_text(action));
                    }
                }
            }
            if !addresses.is_empty() {
                // Offline addresses replace the control with visible text;
                // no action step in the replaced control can run (§7.5).
                return self.text(&addresses.join("\n"), out, depth);
            }
        }
        let mut items = Vec::new();
        let children = component.props.get("children");
        if children.is_absent() {
            for child in component.children.iter() {
                flatten(child, &mut items);
            }
        } else {
            flatten(&children, &mut items);
        }
        // Adding a component node: 4, plus 1 per prop, plus 1 per child (§9.2).
        self.add(depth, 0, 4 + props + items.len() as u64)?;
        let mut tree_props = Vec::with_capacity(props as usize);
        for (key, value, action) in given {
            let value = if action {
                self.action_prop(key, value, depth + 1)?
            } else {
                self.tree_value(value, depth + 1)?
            };
            tree_props.push((DisplayText::new(key), value));
        }
        for (key, state) in component.bindings.iter() {
            tree_props.push((
                DisplayText::new(key),
                TreeValue::Binding {
                    state: DisplayText::new(state),
                    kind: self.state_kind(state),
                },
            ));
        }
        if waiting {
            tree_props.push((DisplayText::new("waiting"), TreeValue::Boolean(true)));
        }
        let mut nodes = Vec::with_capacity(items.len());
        for item in &items {
            self.item(item, &mut nodes, depth + 1, None)?;
        }
        out.push(TreeNode::Component {
            name,
            props: tree_props,
            children: nodes,
        });
        Ok(())
    }

    fn markdown_value(
        &mut self,
        component: &Component,
        out: &mut Vec<TreeNode>,
        depth: u32,
    ) -> Result<(), Abort> {
        if &*component.name == REFERENCE {
            self.add(depth, 0, 1)?;
            let n = component.props.get("n");
            let n = match n.kind() {
                ValueKind::String(n) => n.as_str(),
                _ => "",
            };
            out.push(TreeNode::Reference(DisplayText::new(n)));
            return Ok(());
        }
        let Some(kind) = kind_from(component) else {
            return self.marker(DiagnosticCode::Type, out, depth);
        };
        let text = own_text(&kind);
        self.add(depth, text, 1 + text.div_ceil(8))?;
        let mut items = Vec::new();
        for child in component.children.iter() {
            flatten(child, &mut items);
        }
        let mut children = Vec::with_capacity(items.len());
        for item in &items {
            self.item(item, &mut children, depth + 1, None)?;
        }
        out.push(TreeNode::Markdown { kind, children });
        Ok(())
    }

    /// An action prop that is absent, an error or the wrong kind leaves the
    /// control disabled rather than failing the component (§7.1).
    fn action_prop(&mut self, key: &str, value: &Value, depth: u32) -> Result<TreeValue, Abort> {
        match value.kind() {
            ValueKind::Action(action) => {
                let start = self.actions.len();
                let mut steps = Vec::with_capacity(action.steps.len());
                for step in action.steps.iter() {
                    let mut arguments = Vec::with_capacity(step.arguments.len());
                    for argument in step.arguments.iter() {
                        arguments.push(self.tree_value(argument, depth)?);
                    }
                    steps.push(TreeStep {
                        operation: step.operation,
                        arguments,
                    });
                }
                // Components inside frozen arguments are not drawn controls.
                self.actions.truncate(start);
                self.actions.push((**action).clone());
                Ok(TreeValue::Action(steps))
            }
            ValueKind::Error(error) => {
                self.report(error, Some(key));
                Ok(TreeValue::Error(super::code(&error.code)))
            }
            _ => {
                self.prop_problem(DiagnosticCode::PropType, key);
                Ok(TreeValue::Error(DiagnosticCode::PropType))
            }
        }
    }

    fn prop_problem(&mut self, code: DiagnosticCode, subject: &str) {
        self.push(Diagnostic::new(code, DiagnosticPhase::Evaluate).with_subject(subject));
    }

    /// Whether an evaluated prop value has the catalog prop's type (§5.6),
    /// and is within its catalog bounds. Returns the problem's code.
    fn fits(
        &self,
        component: &str,
        key: &str,
        prop: &PropSchema,
        value: &Value,
    ) -> Option<DiagnosticCode> {
        let expected = prop
            .rules
            .get("tIn")
            .and_then(|types| types.get(self.inputs.profile.name()))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&prop.t);
        if expected == "address" {
            if let ValueKind::String(text) = value.kind() {
                if let Err(code) = Address::parse(text.as_str()) {
                    return Some(code);
                }
            }
        }
        if !self.matches(value, expected, prop) {
            return Some(DiagnosticCode::PropType);
        }
        // A list prop whose catalog entry names a record shape (Table
        // `columns`) needs a string `key` and `label` in each record entry.
        if let (true, ValueKind::List(items)) = (prop.rules.contains_key("record"), value.kind()) {
            let shaped = |item: &Value| match item.kind() {
                ValueKind::Record(record) => {
                    matches!(record.get("key").kind(), ValueKind::String(_))
                        && matches!(record.get("label").kind(), ValueKind::String(_))
                }
                _ => true,
            };
            if !items.iter().all(shaped) {
                return Some(DiagnosticCode::PropType);
            }
        }
        evaluated_bounds(component, key, self.inputs.profile, value).map(|d| d.code())
    }

    fn matches(&self, value: &Value, expected: &str, prop: &PropSchema) -> bool {
        alternatives(expected)
            .into_iter()
            .any(|alternative| self.matches_one(value, alternative, prop))
    }

    fn matches_one(&self, value: &Value, expected: &str, prop: &PropSchema) -> bool {
        let catalog = Catalog::tool_templates();
        if let Some(inner) = expected
            .strip_prefix("list<")
            .and_then(|t| t.strip_suffix('>'))
        {
            return match value.kind() {
                ValueKind::List(items) => items
                    .iter()
                    .all(|item| item.is_absent() || self.matches(item, inner, prop)),
                _ => false,
            };
        }
        if let Some(inner) = expected
            .strip_prefix("node<")
            .and_then(|t| t.strip_suffix('>'))
        {
            return matches!(value.kind(),
                ValueKind::Component(component) if inner.split('|').any(|name| name == &*component.name));
        }
        if expected.starts_with("handle<") {
            return matches!(value.kind(), ValueKind::Handle(_));
        }
        match expected {
            "number" => matches!(value.kind(), ValueKind::Number(_)),
            "string" | "markdown" | "address" => matches!(value.kind(), ValueKind::String(_)),
            "boolean" => matches!(value.kind(), ValueKind::Boolean(_)),
            "enum" => member(
                value,
                prop.rules
                    .get("enum")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str),
            ),
            "tone" => member(value, catalog.vocabulary.tone.iter().map(String::as_str)),
            "toneText" => member(
                value,
                catalog.vocabulary.tone_text.iter().map(String::as_str),
            ),
            "aspect" => member(value, catalog.vocabulary.aspect.keys().map(String::as_str)),
            "gap" => member(value, catalog.vocabulary.gap.keys().map(String::as_str)),
            "record" => matches!(value.kind(), ValueKind::Record(_)),
            "list" => matches!(value.kind(), ValueKind::List(_)),
            "snapshot" => matches!(value.kind(), ValueKind::Snapshot(_)),
            "rows" => match value.kind() {
                ValueKind::List(items) => items
                    .iter()
                    .all(|item| item.is_absent() || matches!(item.kind(), ValueKind::Record(_))),
                _ => false,
            },
            "paged" | "watch" => match value.kind() {
                ValueKind::SiteEnvelope(envelope) => {
                    let constructor = if expected == "paged" {
                        "PagedList"
                    } else {
                        "Watch"
                    };
                    self.program.declarations.iter().any(|declaration| {
                        *declaration.name == *envelope.name
                            && matches!(&declaration.expression.kind,
                                cortexkit_rich_ui_syntax::Kind::Expression(
                                    cortexkit_rich_ui_syntax::Expression::Call(name)
                                ) if name == constructor)
                    })
                }
                _ => false,
            },
            "node" => matches!(
                value.kind(),
                ValueKind::Component(_)
                    | ValueKind::String(_)
                    | ValueKind::Number(_)
                    | ValueKind::Boolean(_)
            ),
            "nodes" => match value.kind() {
                ValueKind::List(items) => items
                    .iter()
                    .all(|item| item.is_absent() || self.matches_one(item, "nodes", prop)),
                _ => self.matches_one(value, "node", prop),
            },
            _ => false,
        }
    }

    /// A data value as a tree prop value, with every string sanitized.
    fn tree_value(&mut self, value: &Value, depth: u32) -> Result<TreeValue, Abort> {
        Ok(match value.kind() {
            ValueKind::Number(number) => TreeValue::Number(*number),
            ValueKind::String(text) => TreeValue::String(DisplayText::new(text.as_str())),
            ValueKind::Boolean(value) => TreeValue::Boolean(*value),
            ValueKind::Null | ValueKind::Missing => TreeValue::Null,
            ValueKind::Error(error) => TreeValue::Error(super::code(&error.code)),
            ValueKind::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items.iter() {
                    values.push(self.tree_value(item, depth)?);
                }
                TreeValue::List(values)
            }
            ValueKind::Record(record) => {
                let mut fields = Vec::with_capacity(record.fields().len());
                for (key, value) in record.fields() {
                    fields.push((
                        DisplayText::new(key.as_str()),
                        self.tree_value(value, depth)?,
                    ));
                }
                TreeValue::Record(fields)
            }
            ValueKind::Component(component) => {
                let mut nodes = Vec::with_capacity(1);
                self.node(component, &mut nodes, depth)?;
                match nodes.pop() {
                    Some(node) => TreeValue::Node(Box::new(node)),
                    None => TreeValue::Null,
                }
            }
            ValueKind::Action(_) => {
                self.prop_problem(DiagnosticCode::ActionPlacement, "action");
                TreeValue::Error(DiagnosticCode::ActionPlacement)
            }
            ValueKind::Snapshot(snapshot) => {
                let state = match snapshot.members.get("state").kind() {
                    ValueKind::String(state) => DisplayText::new(state.as_str()),
                    _ => DisplayText::new(""),
                };
                let parsed = snapshot.members.get("value");
                let value = if parsed.value_type() == crate::ValueType::Missing {
                    None
                } else {
                    Some(Box::new(self.tree_value(&parsed, depth)?))
                };
                TreeValue::Snapshot {
                    record_id: DisplayText::new(&snapshot.record_id),
                    state,
                    value,
                }
            }
            ValueKind::Handle(handle) => TreeValue::Handle(DisplayText::new(&handle.record_id)),
            ValueKind::SiteEnvelope(envelope) => TreeValue::Site(DisplayText::new(&envelope.name)),
        })
    }

    /// The type a binding writes, from the state's declared value (§4.10).
    fn state_kind(&self, state: &str) -> &'static str {
        let declaration = self
            .program
            .declarations
            .iter()
            .find(|declaration| declaration.state && declaration.name == state);
        match declaration.map(|declaration| super::literal(&declaration.expression)) {
            Some(value) => match value.kind() {
                ValueKind::Number(_) => "number",
                ValueKind::String(_) => "string",
                ValueKind::Boolean(_) => "boolean",
                ValueKind::List(_) => "list",
                ValueKind::Record(_) => "record",
                _ => "scalar",
            },
            None => "scalar",
        }
    }
}

/// A source position used only where text conversion of a number or boolean
/// cannot fail, so the span is never reported.
const PLACEHOLDER: SourceSpan = SourceSpan {
    line: 1,
    column: 1,
    end_line: 1,
    end_column: 1,
};

/// Whether a string value is one of a closed list of names.
fn member<'s>(value: &Value, mut names: impl Iterator<Item = &'s str>) -> bool {
    match value.kind() {
        ValueKind::String(text) => names.any(|name| name == text.as_str()),
        _ => false,
    }
}

/// Split a catalog type at its top-level `|` (inside `<…>` it nests).
fn alternatives(expected: &str) -> Vec<&str> {
    let (mut depth, mut start, mut parts) = (0i32, 0, Vec::new());
    for (index, byte) in expected.bytes().enumerate() {
        match byte {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            b'|' if depth == 0 => {
                parts.push(&expected[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&expected[start..]);
    parts
}
