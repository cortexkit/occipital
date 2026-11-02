//! Calls and constructed values: built-ins (§6), `Each` (§5.5), `Parse`
//! (§10.2), sites (§8), actions (§7), handles (§3.9), references (§3.8) and
//! components written as tags (§3.5).

use std::sync::Arc;

use cortexkit_rich_ui_syntax::{Expression as E, Kind, Node, Prop};

use super::{snapshot, Abort, Demand, Evaluator};
use crate::builtins::{Builtin, Failure};
use crate::catalog::Catalog;
use crate::prepare::resolve::{identifier, reserved};
use crate::{
    ActionOperation, ActionStep, Address, Handle, HandleIdentity, HandleKind, ScalarString,
    SiteEnvelope, Value, ValueKind,
};

/// `Compose` text is at most 8,192 scalars (§7.5).
const COMPOSE_SCALARS: u64 = 8_192;
/// An action has at most 16 steps (§7.1).
const ACTION_STEPS: usize = 16;
/// `Each` nests at most four deep (§5.5).
const EACH_DEPTH: usize = 4;

impl<'a> Evaluator<'a> {
    pub(super) fn call(&mut self, name: &'a str, node: &'a Node) -> Result<Value, Abort> {
        match name {
            "Each" => self.each(node),
            "Parse" => self.parse(node),
            // Sites are evaluated only as the whole expression of a named
            // value (§6.5), where `named` dispatches them.
            "Query" | "Watch" | "PagedList" => self.error("E_SITE_PLACEMENT", node),
            "Action" | "Set" | "Reset" | "Refresh" | "LoadMore" | "Compose" | "Open" => {
                self.step(name, node)
            }
            "Mutation" | "Run" => self.error("E_WRITE_NOT_ALLOWED", node),
            _ => match builtin(name) {
                Some(builtin) => self.builtin(builtin, node),
                None => self.error("E_UNKNOWN_FUNCTION", node),
            },
        }
    }

    /// Arguments left to right; the first error argument is the result, and
    /// later arguments are not evaluated (§4.6). `IsError` alone takes an
    /// error as an ordinary argument.
    fn builtin(&mut self, builtin: Builtin, node: &'a Node) -> Result<Value, Abort> {
        let mut arguments = Vec::with_capacity(node.children.len());
        for child in &node.children {
            let value = self.expr(child)?;
            if builtin != Builtin::IsError && matches!(value.kind(), ValueKind::Error(_)) {
                return Ok(value);
            }
            arguments.push(value);
        }
        let span = self.span(node);
        let meter = &mut self.meter;
        // The built-in dispatch charges the call base and its §6 amount itself.
        let result = builtin.call(&arguments, span, &mut |fuel| meter.charge(fuel));
        match result {
            Ok(value) => {
                // A built-in that selects an existing element creates nothing.
                if !matches!(
                    builtin,
                    Builtin::First | Builtin::Last | Builtin::Min | Builtin::Max
                ) {
                    self.meter.admit(
                        value.allocation_size(crate::Origin::Evaluation),
                        value.logical_size(),
                        value.depth(),
                    )?;
                    if let ValueKind::String(text) = value.kind() {
                        self.meter.longest_string = self.meter.longest_string.max(text.len());
                    }
                }
                Ok(value)
            }
            Err(Failure::Value(error)) => self.atom(Value::error(error)),
            Err(Failure::Fuel(abort)) => Err(abort),
            Err(Failure::Limit(code)) => Err(Abort::from_code(code)),
        }
    }

    /// `Each(list, "name", template)` (§5.5): one unit for the call, one per
    /// element, then the template once per element in list order (§9.2).
    fn each(&mut self, node: &'a Node) -> Result<Value, Abort> {
        self.charge(1)?;
        if node.children.len() != 3 {
            return self.error("E_ARITY", node);
        }
        let input = self.expr(&node.children[0])?;
        if matches!(input.kind(), ValueKind::Error(_)) {
            return Ok(input);
        }
        let binder = match &node.children[1].kind {
            Kind::Expression(E::String(name))
                if identifier(name)
                    && !reserved(name)
                    && !self
                        .program
                        .declarations
                        .iter()
                        .any(|declaration| declaration.name == *name)
                    && !self.is_binder(name) =>
            {
                name.as_str()
            }
            _ => return self.error("E_EACH_BINDER", &node.children[1]),
        };
        if self.binders.len() >= EACH_DEPTH {
            return self.error("E_EACH_NESTING", node);
        }
        let items = match input.kind() {
            ValueKind::List(items) => items.clone(),
            ValueKind::Null | ValueKind::Missing => Arc::from(Vec::new()),
            _ => return self.error("E_TYPE", &node.children[0]),
        };
        self.charge(items.len() as u64)?;
        let mut results = Vec::with_capacity(items.len());
        let mut logical = 16u64;
        for item in items.iter() {
            self.binders.push((binder, item.clone()));
            let result = self.expr(&node.children[2]);
            self.binders.pop();
            let result = result?;
            // Refuse the result list as soon as it is known to be too large,
            // before more template work is done.
            logical = logical.saturating_add(result.logical_size());
            if logical > super::VALUE_BYTES {
                return Err(Abort::LimitValue);
            }
            if result.depth() >= super::VALUE_DEPTH {
                return Err(Abort::LimitDepth);
            }
            results.push(result);
        }
        self.list(results)
    }

    /// `Parse(snapshot, format, options?)` reads the result the host prepared
    /// for this (snapshot, format, options) at the cost of one lookup (§10.2).
    fn parse(&mut self, node: &'a Node) -> Result<Value, Abort> {
        self.charge(1)?;
        if !matches!(node.children.len(), 2 | 3) {
            return self.error("E_ARITY", node);
        }
        let target = self.expr(&node.children[0])?;
        let snapshot = match target.kind() {
            ValueKind::Snapshot(snapshot) => snapshot.clone(),
            ValueKind::Error(_) | ValueKind::Null | ValueKind::Missing => return Ok(target),
            _ => return self.error("E_TYPE", &node.children[0]),
        };
        let key = match snapshot::parse_key(node) {
            Ok(key) => key,
            Err(code) => return self.error(code, node),
        };
        Ok(match self.inputs.prepared.get(&key) {
            Some(prepared) => snapshot::with_prepared(&snapshot, prepared),
            None => target,
        })
    }

    /// A site constructor (§8): its arguments are evaluated in order, the site
    /// joins the demand set, and the program reads its current envelope.
    pub(super) fn site(
        &mut self,
        index: usize,
        constructor: &'a str,
        node: &'a Node,
    ) -> Result<Value, Abort> {
        self.charge(1)?;
        let outer = std::mem::take(&mut self.provenance);
        let mut arguments = Vec::with_capacity(node.children.len());
        for child in &node.children {
            arguments.push(self.expr(child)?);
        }
        let name: Arc<str> = self.program.declarations[index].name.as_str().into();
        let demand = Demand {
            site: name.clone(),
            constructor: constructor.into(),
            arguments,
            dependencies: self.provenance.values().cloned().collect(),
        };
        self.provenance = outer;
        let envelope = match self.resolver.as_mut() {
            Some(resolver) => resolver.resolve(&demand),
            None => self
                .inputs
                .sites
                .get(name.as_ref())
                .cloned()
                .unwrap_or_else(|| loading(&name, constructor, &demand.arguments)),
        };
        self.provenance.insert(name.to_string(), envelope.clone());
        self.demands.push(demand);
        Ok(envelope)
    }

    /// Action steps and `Action([…])` (§7.1, §7.2). Arguments are frozen when
    /// the action is built, at one unit per step (§9.2).
    fn step(&mut self, name: &'a str, node: &'a Node) -> Result<Value, Abort> {
        let expected = if matches!(name, "Set") { 2 } else { 1 };
        if node.children.len() != expected {
            self.charge(1)?;
            return self.error("E_ARITY", node);
        }
        let first = &node.children[0];
        let (operation, arguments) = match name {
            "Action" => {
                let list = self.expr(first)?;
                let items = match list.kind() {
                    ValueKind::List(items) => items.clone(),
                    ValueKind::Error(_) => return Ok(list),
                    _ => return self.error("E_TYPE", first),
                };
                let mut steps = Vec::new();
                for item in items.iter() {
                    match item.kind() {
                        ValueKind::Action(action) => steps.extend(action.steps.iter().cloned()),
                        ValueKind::Error(_) => return Ok(item.clone()),
                        ValueKind::Null | ValueKind::Missing => {}
                        _ => return self.error("E_TYPE", first),
                    }
                }
                self.charge(steps.len() as u64)?;
                // Single-step calls below build exactly one step. This check
                // bounds the flattened Action before Runtime::run receives it;
                // activation does not repeat the step-count limit (§7.1).
                if steps.len() > ACTION_STEPS {
                    return self.error("E_LIMIT_STEPS", node);
                }
                return self.action(steps);
            }
            "Set" | "Reset" => {
                let Kind::Expression(E::State(state)) = &first.kind else {
                    self.charge(1)?;
                    return self.error("E_LITERAL_REQUIRED", first);
                };
                let mut arguments = vec![];
                if name == "Set" {
                    let value = self.expr(&node.children[1])?;
                    if matches!(value.kind(), ValueKind::Error(_)) {
                        return Ok(value);
                    }
                    arguments.push(value);
                }
                let operation = if name == "Set" {
                    ActionOperation::Set
                } else {
                    ActionOperation::Reset
                };
                // The step names the state it writes; it never reads it.
                self.charge(1)?;
                let state = self.string(state.clone(), state.chars().count() as u64)?;
                arguments.insert(0, state);
                (operation, arguments)
            }
            "Refresh" | "LoadMore" => {
                let target = self.expr(first)?;
                let program = self.program;
                let paged = |envelope: &SiteEnvelope| {
                    program.declarations.iter().any(|declaration| {
                        *declaration.name == *envelope.name
                            && matches!(&declaration.expression.kind,
                                Kind::Expression(E::Call(c)) if c == "PagedList")
                    })
                };
                match target.kind() {
                    ValueKind::Error(_) => return Ok(target),
                    ValueKind::SiteEnvelope(envelope) if name == "Refresh" || paged(envelope) => {}
                    _ => return self.error("E_TYPE", first),
                }
                self.charge(1)?;
                let operation = if name == "Refresh" {
                    ActionOperation::Refresh
                } else {
                    ActionOperation::LoadMore
                };
                (operation, vec![target])
            }
            "Compose" => {
                let text = self.expr(first)?;
                // This is the Compose size check. Runtime::run hands off the
                // frozen, already checked text without repeating it (§7.5).
                match text.kind() {
                    ValueKind::Error(_) => return Ok(text),
                    ValueKind::String(value) if value.len() <= COMPOSE_SCALARS => {}
                    ValueKind::String(_) => return self.error("E_ARG", first),
                    _ => return self.error("E_TYPE", first),
                }
                self.charge(1)?;
                (ActionOperation::Compose, vec![text])
            }
            _ => {
                let target = self.expr(first)?;
                let target = match target.kind() {
                    ValueKind::Error(_) => return Ok(target),
                    ValueKind::Handle(_) => target,
                    // An address must parse under §3.11; the step carries the
                    // canonical serialization, the exact URL that would open.
                    ValueKind::String(text) => match Address::parse(text.as_str()) {
                        Ok(address) => {
                            let canonical = address.as_str().to_owned();
                            let scalars = canonical.chars().count() as u64;
                            self.string(canonical, scalars)?
                        }
                        Err(code) => return self.error(code.as_str(), first),
                    },
                    _ => return self.error("E_TYPE", first),
                };
                self.charge(1)?;
                (ActionOperation::Open, vec![target])
            }
        };
        self.action(vec![ActionStep {
            operation,
            arguments: arguments.into(),
        }])
    }

    /// A tag in an expression or the body builds a component value: its props
    /// in source order, then its content (§5.3 rule 1). Building the value is
    /// one visit; adding it to the tree is charged when it is inserted.
    pub(super) fn tag(
        &mut self,
        node: &'a Node,
        name: &'a str,
        props: &'a [Prop],
    ) -> Result<Value, Abort> {
        self.charge(1)?;
        let schema = Catalog::tool_templates().components.get(name);
        let mut fields = Vec::with_capacity(props.len());
        let mut bindings = Vec::new();
        for prop in props {
            let binding = schema
                .and_then(|component| component.props.get(&prop.name))
                .is_some_and(|schema| schema.t.starts_with("binding"));
            if let (true, Kind::Expression(E::State(state))) = (binding, &prop.value.kind) {
                // A binding names its state; it does not read the value.
                self.charge(1)?;
                bindings.push((prop.name.as_str().into(), state.as_str().into()));
                continue;
            }
            let value = self.expr(&prop.value)?;
            fields.push((ScalarString::new(prop.name.as_str()), value));
        }
        // A Slot whose `needs` is given but absent waits for data: the app
        // draws the empty box as loading, or as muted once the data won't come,
        // so the content is never evaluated against the missing data.
        let waiting = name == "Slot"
            && fields
                .iter()
                .any(|(key, value)| key.as_str() == "needs" && value.is_absent());
        let children = if waiting {
            // A missing `needs`, such as a field read on a record that hasn't
            // arrived, would drop out of the component's props, and the tree
            // would then see a Slot with no `needs` and no content: neither
            // waiting nor drawn. Kept as null, it still reads as waiting there.
            for (key, value) in &mut fields {
                if key.as_str() == "needs" {
                    *value = Value::null();
                }
            }
            Vec::new()
        } else {
            self.content(&node.children)?
        };
        self.component(name, fields, bindings, children)
    }

    /// A tag's content as child values. Text and Markdown structure become
    /// values too, so the content can be stored in a named value and placed
    /// later exactly as written; each costs one literal visit.
    fn content(&mut self, nodes: &'a [Node]) -> Result<Vec<Value>, Abort> {
        let mut values = Vec::with_capacity(nodes.len());
        for node in nodes {
            match &node.kind {
                Kind::Declaration { .. } => {}
                Kind::Text(text) | Kind::Literal(text) => {
                    self.charge(1)?;
                    values.push(self.string(text.clone(), text.chars().count() as u64)?);
                }
                Kind::Markdown(markdown) => {
                    if super::tree::insignificant(node) {
                        continue;
                    }
                    values.push(self.markdown_content(node, super::tree::pseudo(markdown))?);
                }
                Kind::Interpolation { .. } => values.push(self.expr(&node.children[0])?),
                Kind::Reference { n } => {
                    self.charge(1)?;
                    let number = self.string(n.clone(), n.chars().count() as u64)?;
                    let field = vec![(ScalarString::new("n"), number)];
                    values.push(self.component(
                        super::tree::REFERENCE,
                        field,
                        Vec::new(),
                        vec![],
                    )?);
                }
                _ => values.push(self.expr(node)?),
            }
        }
        Ok(values)
    }

    pub(super) fn markdown_content(
        &mut self,
        node: &'a Node,
        descriptor: Option<super::tree::MarkdownContent>,
    ) -> Result<Value, Abort> {
        let Some((name, attributes)) = descriptor else {
            return self.unsupported(node);
        };
        self.charge(1)?;
        let children = self.content(&node.children)?;
        let mut fields = Vec::with_capacity(attributes.len());
        for (key, value) in attributes {
            fields.push((ScalarString::new(key), self.attribute(value)?));
        }
        self.component(name, fields, Vec::new(), children)
    }

    fn attribute(&mut self, value: super::tree::Attribute) -> Result<Value, Abort> {
        match value {
            super::tree::Attribute::Text(text) => {
                let scalars = text.chars().count() as u64;
                self.string(text, scalars)
            }
            super::tree::Attribute::Number(number) => self.atom(Value::number(number)),
            super::tree::Attribute::Boolean(value) => self.atom(Value::boolean(value)),
        }
    }

    /// The snapshot for `$$tc(N)` (§3.8, §4.8). Without a stored record the
    /// reference is unresolved; the crate never resolves N itself.
    pub(super) fn reference(&self, n: &str) -> Value {
        self.inputs
            .references
            .get(n)
            .cloned()
            .unwrap_or_else(|| snapshot::unresolved(n))
    }

    /// A handle value from the record stored for this source span (§3.9,
    /// §4.9). It carries the record id, never the written path.
    pub(super) fn handle(&self, node: &Node, kind: &str) -> Value {
        let span = self.span(node);
        let record = self.inputs.handles.iter().find(|record| {
            record.span == span
                && matches!(
                    (record.kind, kind),
                    (HandleKind::Artifact, "Artifact") | (HandleKind::File, "File")
                )
        });
        let Some(record) = record else {
            return Value::handle(Handle {
                record_id: "".into(),
                identity: HandleIdentity::Unresolved {
                    scoped_record_id: "".into(),
                },
            });
        };
        let scoped = |id: &str| {
            [
                record.computer.as_str(),
                record.session.as_str(),
                record.scope.as_str(),
                id,
            ]
            .join("\u{0}")
        };
        let identity = match (&record.resource_id, &record.version, record.available) {
            (Some(resource), Some(version), true) => HandleIdentity::Resolved {
                kind: kind.into(),
                scoped_resource_id: scoped(resource).into(),
                version: version.as_str().into(),
            },
            _ => HandleIdentity::Unresolved {
                scoped_record_id: scoped(&record.record_id).into(),
            },
        };
        Value::handle(Handle {
            record_id: record.record_id.as_str().into(),
            identity,
        })
    }
}

/// The first-demand envelope of a site that has no host state yet: loading,
/// not stale, with the `Query` default as its value (§8.3, §8.5, §8.6).
fn loading(name: &str, constructor: &str, arguments: &[Value]) -> Value {
    let empty = || Value::items(Vec::new());
    let mut fields: Vec<(&str, Value)> = match constructor {
        "Query" => vec![(
            "value",
            arguments.get(2).cloned().unwrap_or_else(Value::null),
        )],
        "Watch" => vec![("value", empty())],
        _ => vec![("items", empty())],
    };
    fields.extend([("status", Value::text("loading")), ("error", Value::null())]);
    match constructor {
        "Query" => {}
        "Watch" => fields.push(("dropped", Value::number(crate::Decimal::ZERO))),
        _ => fields.extend([
            ("done", Value::boolean(false)),
            ("total", Value::null()),
            ("loadingMore", Value::boolean(false)),
            ("capped", Value::boolean(false)),
        ]),
    }
    fields.push(("stale", Value::boolean(false)));
    let members = crate::Record::members(
        fields
            .into_iter()
            .map(|(key, value)| (ScalarString::new(key), value))
            .collect(),
    );
    Value::site_envelope(SiteEnvelope {
        name: name.into(),
        members,
    })
}

/// The built-in for a call name (§6.1–§6.4).
fn builtin(name: &str) -> Option<Builtin> {
    use Builtin::*;
    Some(match name {
        "Count" => Count,
        "Sum" => Sum,
        "Avg" => Avg,
        "Min" => Min,
        "Max" => Max,
        "Filter" => Filter,
        "Sort" => Sort,
        "First" => First,
        "Last" => Last,
        "Slice" => Slice,
        "Range" => Range,
        "Contains" => Contains,
        "Keys" => Keys,
        "Values" => Values,
        "Join" => Join,
        "IsError" => IsError,
        "Round" => Round,
        "Floor" => Floor,
        "Ceil" => Ceil,
        "Abs" => Abs,
        "Sqrt" => Sqrt,
        "Clamp" => Clamp,
        "Number" => Number,
        "Div" => Div,
        "Text" => Text,
        "Len" => Len,
        "Upper" => Upper,
        "Lower" => Lower,
        "Trim" => Trim,
        "StartsWith" => StartsWith,
        "EndsWith" => EndsWith,
        "Replace" => Replace,
        "Split" => Split,
        "Lines" => Lines,
        "Truncate" => Truncate,
        "FormatNumber" => FormatNumber,
        "FormatBytes" => FormatBytes,
        "FormatDuration" => FormatDuration,
        "FormatDate" => FormatDate,
        "FormatRelative" => FormatRelative,
        "ParseDate" => ParseDate,
        _ => return None,
    })
}
