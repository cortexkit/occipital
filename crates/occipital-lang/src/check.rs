//! Catalog and profile checks, after syntax preparation and gradual typing.

pub(crate) mod catalog;
mod policy;
mod schema;
pub(crate) mod trial;

use std::collections::BTreeMap;

use cortexkit_rich_ui_syntax::{Expression as E, Kind, Mode, Node, Prop};
use serde_json::Value as Json;

use crate::catalog::Catalog;
use crate::prepare::resolve::Resolved;
use crate::prepare::types::{Type, TypeContext, RESERVED_WRITES, SITE_CONSTRUCTORS};
use crate::prepare::{parse, positioned, resolve};
use crate::{CheckContext, Decimal, Diagnostic, DiagnosticCode as Code, DiagnosticPhase, Profile};

/// Returns a resolved AST with catalog/profile diagnostics and emits no effects.
pub(crate) fn check(
    source: &[u8],
    context: &CheckContext,
) -> Result<Resolved, crate::accounts::Refused> {
    let mode = if context.profile == Profile::Reply {
        Mode::Reply
    } else {
        Mode::Program
    };
    Ok(check_parsed(parse(source, mode)?, context))
}

pub(crate) fn check_parsed(mut parsed: crate::prepare::Parsed, context: &CheckContext) -> Resolved {
    if context.profile == Profile::Reply {
        reply_writes(&mut parsed);
    }
    let types = schema::context(&parsed.document.nodes, context);
    let mut resolved = resolve::resolve(parsed, &types);
    if resolved.parsed.document.fallback {
        return resolved;
    }
    // A tool template may name the tool catalog, whose extra components no
    // other profile may use; everything else is checked against catalog 1.0.
    let tool_catalog = matches!(context.profile, Profile::ToolRow | Profile::ToolPage)
        && context.catalog == crate::CatalogVersion::Tool1_1;
    let catalog = if tool_catalog {
        Catalog::tool_templates()
    } else {
        Catalog::shared()
    };
    let mut checker = Checker {
        resolved: &resolved,
        context,
        catalog,
        diagnostics: vec![],
        sites: [0; 3],
        slots: tool_catalog && context.profile == Profile::ToolRow,
        changing: if tool_catalog && context.profile == Profile::ToolRow {
            changing_declarations(&resolved)
        } else {
            Default::default()
        },
    };
    checker.hosts();
    for node in &resolved.parsed.document.nodes {
        checker.visit(node, Place::default());
    }
    checker.unused();
    let diagnostics = checker.diagnostics;
    resolved.diagnostics.extend(diagnostics);
    resolved
}

#[derive(Clone, Copy)]
struct Place<'a> {
    parent: Option<&'a str>,
    inline: bool,
    action: bool,
    site: bool,
    render: bool,
    /// Inside a `Slot`, where a tool row may read what changes after the row
    /// is drawn.
    slot: bool,
    /// Inside a declaration's expression, which draws nothing by itself: a
    /// read there counts where the declared name is used.
    declared: bool,
}

impl Default for Place<'_> {
    fn default() -> Self {
        Self {
            parent: None,
            inline: false,
            action: false,
            site: false,
            render: true,
            slot: false,
            declared: false,
        }
    }
}

struct Checker<'a> {
    resolved: &'a Resolved,
    context: &'a CheckContext,
    catalog: &'a Catalog,
    diagnostics: Vec<Diagnostic>,
    sites: [usize; 3],
    /// Whether this is a tool row checked against the tool catalog, which has
    /// `Slot` and so the rule that changing data is read only inside one.
    slots: bool,
    /// In such a row, the declarations whose values read `view` or `now`
    /// outside a `Slot`, directly or through other declarations.
    changing: std::collections::BTreeSet<String>,
}

impl Checker<'_> {
    fn report(&mut self, code: Code, node: &Node, subject: &str, signature: Option<&str>) {
        // Type preparation already diagnoses reserved writes. Avoid a second
        // copy when a later phase visits the same call (§12.1).
        let span = self.resolved.parsed.positions.span(node.span);
        if self
            .resolved
            .diagnostics
            .iter()
            .chain(&self.diagnostics)
            .any(|d| d.code() == code && d.span() == Some(crate::DiagnosticSpan::Source(span)))
        {
            return;
        }
        let mut diagnostic = positioned(
            code,
            DiagnosticPhase::Check,
            node.span,
            &self.resolved.parsed.positions,
        )
        .with_subject(subject);
        if let Some(signature) = signature {
            diagnostic = diagnostic.with_signature(signature);
        }
        self.diagnostics.push(diagnostic);
    }

    fn ty(&self, node: &Node) -> Type {
        self.resolved
            .node_types
            .get(&(node.span.start, node.span.end))
            .cloned()
            .unwrap_or_else(|| schema::syntax_type(node))
    }

    fn definition<'a>(&'a self, node: &'a Node) -> &'a Node {
        let mut node = node;
        for _ in 0..256 {
            match &node.kind {
                Kind::Expression(E::Group) => node = &node.children[0],
                Kind::Expression(E::Name(name)) => {
                    if let Some(declaration) = self
                        .resolved
                        .declarations
                        .iter()
                        .find(|d| !d.state && d.name == *name)
                    {
                        node = &declaration.expression;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        node
    }

    fn known(&self, node: &Node) -> Option<Json> {
        // Folding is bounded and partial: it skips expressions that depend on
        // future answers, state or general calls.
        self.fold(node, &mut 8192, &mut 65_536)
    }

    fn fold(&self, node: &Node, remaining: &mut usize, bytes: &mut usize) -> Option<Json> {
        *remaining = remaining.checked_sub(1)?;
        let node = self.definition(node);
        let value = match &node.kind {
            Kind::Expression(E::Number(n)) => folded_number(Decimal::from_literal(n).ok()?)?,
            Kind::Expression(E::String(s)) => {
                *bytes = bytes.checked_sub(s.len())?;
                Json::String(s.clone())
            }
            Kind::Expression(E::Boolean(b)) => Json::Bool(*b),
            Kind::Expression(E::Null) => Json::Null,
            Kind::Expression(E::List) => Json::Array(
                node.children
                    .iter()
                    .map(|n| self.fold(n, remaining, bytes))
                    .collect::<Option<_>>()?,
            ),
            Kind::Expression(E::Record(keys)) => Json::Object(
                keys.iter()
                    .zip(&node.children)
                    .map(|(key, n)| {
                        *bytes = bytes.checked_sub(key.len())?;
                        Some((key.clone(), self.fold(n, remaining, bytes)?))
                    })
                    .collect::<Option<_>>()?,
            ),
            Kind::Expression(E::Unary(op)) if op == "-" => {
                let value = decimal(&self.fold(&node.children[0], remaining, bytes)?)?;
                folded_number(-value)?
            }
            Kind::Expression(E::Binary(op)) => {
                let a = decimal(&self.fold(&node.children[0], remaining, bytes)?)?;
                let b = decimal(&self.fold(&node.children[1], remaining, bytes)?)?;
                let value = match op.as_str() {
                    "+" => a.checked_add(b),
                    "-" => a.checked_sub(b),
                    "*" => a.checked_mul(b),
                    "/" => a.checked_div(b),
                    "%" => a.checked_rem(b),
                    _ => return None,
                }
                .ok()?;
                folded_number(value)?
            }
            _ => return None,
        };
        // Stop alias expansion before it can allocate an exponentially large
        // value. Larger static data is handled by normal evaluation limits.
        if serde_json::to_vec(&value).ok()?.len() > 65_536 {
            return None;
        }
        Some(value)
    }

    fn visit(&mut self, node: &Node, place: Place<'_>) {
        match &node.kind {
            Kind::Declaration { state, .. } => {
                if let Some(expression) = node.children.first() {
                    self.visit(
                        expression,
                        Place {
                            site: !state,
                            render: false,
                            declared: true,
                            ..Place::default()
                        },
                    );
                }
                return;
            }
            Kind::Tag { name, block, props } => {
                self.tag(node, name, *block, props, place);
                return;
            }
            Kind::Markdown(
                cortexkit_rich_ui_syntax::Markdown::Paragraph
                | cortexkit_rich_ui_syntax::Markdown::Heading(_)
                | cortexkit_rich_ui_syntax::Markdown::TableCell,
            ) => {
                for child in &node.children {
                    self.visit(
                        child,
                        Place {
                            inline: true,
                            site: false,
                            ..place
                        },
                    );
                }
                return;
            }
            Kind::Interpolation { block } => {
                for child in &node.children {
                    self.visit(
                        child,
                        Place {
                            inline: !block,
                            site: false,
                            ..place
                        },
                    );
                }
                return;
            }
            Kind::Expression(E::Name(name))
                if matches!(name.as_str(), "data" | "input" | "view" | "now" | "zone") =>
            {
                if !self.context.profile.supplies(name) {
                    self.report(Code::HostValueUnavailable, node, name, None);
                }
            }
            Kind::Reference { .. } if !self.context.profile.references() => {
                self.report(Code::ReferenceUnavailable, node, "reference", None);
            }
            Kind::Expression(E::Call(name)) if RESERVED_WRITES.contains(&name.as_str()) => {
                // Name or cycle errors stop type inference before its write
                // check, so this walk must still refuse reserved calls.
                self.report(Code::WriteNotAllowed, node, name, None);
            }
            Kind::Expression(E::Call(name)) if SITE_CONSTRUCTORS.contains(&name.as_str()) => {
                self.site(node, name, place.site);
            }
            _ => {}
        }
        if let Kind::Expression(E::Name(name)) = &node.kind {
            // A tool row is measured once, before it is drawn, and never
            // resized, so only a Slot's fixed box may show the call's view or
            // the time, which change while the row is on screen.
            if self.slots
                && !place.slot
                && !place.declared
                && (matches!(name.as_str(), "view" | "now") || self.changing.contains(name))
            {
                self.report(Code::UnslottedRead, node, name, None);
            }
        }
        let ty = self.ty(node);
        if place.render {
            self.render_value(node, &ty, place);
        }
        if contains_action(&ty) && !place.action && !matches!(node.kind, Kind::Declaration { .. }) {
            self.report(Code::ActionPlacement, node, "action", None);
            // One diagnostic for the misplaced action, not every step it owns.
            for child in &node.children {
                self.visit(
                    child,
                    Place {
                        action: true,
                        site: false,
                        ..place
                    },
                );
            }
            return;
        }
        for (index, child) in node.children.iter().enumerate() {
            let render = match &node.kind {
                Kind::Expression(E::Call(name)) => name == "Each" && index == 2 && place.render,
                Kind::Expression(E::Conditional) => index != 0 && place.render,
                Kind::Expression(
                    E::Binary(_) | E::Unary(_) | E::Member(_) | E::Index | E::Record(_),
                ) => false,
                _ => place.render,
            };
            self.visit(
                child,
                Place {
                    site: false,
                    render,
                    ..place
                },
            );
        }
    }
}

/// The declarations whose values read `view` or `now` outside a `Slot`,
/// directly or through other such declarations. A Slot inside a declaration
/// keeps what it reads in its own box, so it doesn't count.
fn changing_declarations(resolved: &Resolved) -> std::collections::BTreeSet<String> {
    fn reads(node: &Node, changing: &std::collections::BTreeSet<String>) -> bool {
        match &node.kind {
            Kind::Tag { name, .. } if name == "Slot" => false,
            Kind::Tag { props, .. } => {
                props.iter().any(|p| reads(&p.value, changing))
                    || node.children.iter().any(|c| reads(c, changing))
            }
            Kind::Expression(E::Name(name)) => {
                matches!(name.as_str(), "view" | "now") || changing.contains(name)
            }
            _ => node.children.iter().any(|c| reads(c, changing)),
        }
    }
    let mut changing = std::collections::BTreeSet::new();
    // Each pass adds at least one name or stops, so there are at most as
    // many passes as declarations.
    loop {
        let before = changing.len();
        for declaration in &resolved.declarations {
            if !declaration.state
                && !changing.contains(&declaration.name)
                && reads(&declaration.expression, &changing)
            {
                changing.insert(declaration.name.clone());
            }
        }
        if changing.len() == before {
            return changing;
        }
    }
}

fn prop<'a>(props: &'a [Prop], name: &str) -> Option<&'a Node> {
    props.iter().find(|p| p.name == name).map(|p| &p.value)
}

/// `value` as a folded JSON number, only when the JSON number reads back as
/// the same decimal: always for integers in 64 bits and for decimals a binary
/// float holds exactly, never for longer ones. serde_json keeps a number's
/// exact text only under its global `arbitrary_precision` feature, which this
/// library doesn't turn on, so a longer number is left unknown here, the same
/// whatever an app's build turns on, and is checked when evaluated instead.
fn folded_number(value: Decimal) -> Option<Json> {
    let text = value.to_string();
    let number = match (text.parse::<i64>(), text.parse::<u64>()) {
        (Ok(n), _) => serde_json::Number::from(n),
        (_, Ok(n)) => serde_json::Number::from(n),
        _ => serde_json::Number::from_f64(text.parse().ok()?)?,
    };
    (Decimal::parse_text(&number.to_string()).ok()? == value).then_some(Json::Number(number))
}

fn decimal(value: &Json) -> Option<Decimal> {
    Decimal::from_literal(&value.as_number()?.to_string()).ok()
}

fn contains_action(ty: &Type) -> bool {
    match ty {
        Type::Action => true,
        Type::List(item) => contains_action(item),
        Type::Record(fields) | Type::Envelope(_, fields) => {
            fields.iter().any(|(_, ty)| contains_action(ty))
        }
        Type::Union(types) => types.iter().any(contains_action),
        _ => false,
    }
}

fn reply_writes(parsed: &mut crate::prepare::Parsed) {
    fn calls(node: &Node, offset: usize, out: &mut Vec<(cortexkit_rich_ui_syntax::Span, String)>) {
        if let Kind::Expression(E::Call(name)) = &node.kind {
            if RESERVED_WRITES.contains(&name.as_str()) {
                out.push((
                    cortexkit_rich_ui_syntax::Span {
                        start: offset + node.span.start,
                        end: offset + node.span.end,
                    },
                    name.clone(),
                ));
            }
        }
        if let Kind::Tag { props, .. } = &node.kind {
            for p in props {
                calls(&p.value, offset, out);
            }
        }
        for child in &node.children {
            calls(child, offset, out);
        }
    }
    fn literals(
        node: &Node,
        parsed: &crate::prepare::Parsed,
        out: &mut Vec<(cortexkit_rich_ui_syntax::Span, String)>,
    ) {
        if let Kind::Literal(text) = &node.kind {
            if parsed
                .document
                .diagnostics
                .iter()
                .any(|d| d.code.as_str() == "E_REPLY_FORBIDDEN" && d.span == node.span)
            {
                // Reply parsing intentionally makes a forbidden tag opaque.
                // Reparse only that already-identified span with the same
                // grammar to recognize reserved write calls, never prose or
                // data strings. Reference scanning still uses the reply AST.
                for tag in
                    cortexkit_rich_ui_syntax::parse_bytes(text.as_bytes(), Mode::Program).nodes
                {
                    calls(&tag, node.span.start, out);
                }
            }
        }
        for child in &node.children {
            literals(child, parsed, out);
        }
    }
    let mut writes = vec![];
    for node in &parsed.document.nodes {
        literals(node, parsed, &mut writes);
    }
    for (span, name) in writes {
        parsed.diagnostics.push(
            positioned(
                Code::WriteNotAllowed,
                DiagnosticPhase::Check,
                span,
                &parsed.positions,
            )
            .with_subject(&name),
        );
    }
}

#[cfg(test)]
mod tests;
