use std::collections::BTreeMap;

use cortexkit_rich_ui_syntax::{Expression, Kind, Node};

use super::types::{self, Type, TypeContext};
use super::{positioned, Parsed};
use crate::{Diagnostic, DiagnosticCode as Code, DiagnosticPhase, Severity};

pub(crate) struct Declaration {
    pub name: String,
    pub state: bool,
    pub expression: Node,
    pub dependencies: Vec<usize>,
    pub ty: Type,
}

pub(crate) struct Resolved {
    pub parsed: Parsed,
    pub declarations: Vec<Declaration>,
    pub diagnostics: Vec<Diagnostic>,
    pub node_types: BTreeMap<(usize, usize), Type>,
}

pub(crate) fn reserved(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        || matches!(
            name,
            "data"
                | "input"
                | "view"
                | "now"
                | "zone"
                | "true"
                | "false"
                | "null"
                | "export"
                | "const"
                | "let"
        )
}

pub(crate) fn identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub(crate) fn resolve(parsed: Parsed, context: &TypeContext) -> Resolved {
    let mut result = Resolved {
        diagnostics: parsed.diagnostics.clone(),
        parsed,
        declarations: vec![],
        node_types: BTreeMap::new(),
    };
    if result.parsed.document.fallback
        || result
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Error)
    {
        return result;
    }
    let mut names = BTreeMap::new();
    for node in &result.parsed.document.nodes {
        if let Kind::Declaration { name, state } = &node.kind {
            // The syntax crate's duplicate and reserved declaration checks
            // refuse these cases before the error-diagnostic return above.
            names.insert(name.clone(), result.declarations.len());
            result.declarations.push(Declaration {
                name: name.clone(),
                state: *state,
                expression: node.children[0].clone(),
                dependencies: vec![],
                ty: Type::Unknown,
            });
        }
    }
    let mut resolver = Names {
        parsed: &result.parsed,
        declarations: &result.declarations,
        names: &names,
        diagnostics: vec![],
        edges: vec![vec![]; result.declarations.len()],
    };
    for (index, declaration) in result.declarations.iter().enumerate() {
        resolver.visit(&declaration.expression, Some(index), &mut vec![]);
    }
    for node in &result.parsed.document.nodes {
        if !matches!(node.kind, Kind::Declaration { .. }) {
            resolver.visit(node, None, &mut vec![]);
        }
    }
    let Names {
        diagnostics, edges, ..
    } = resolver;
    result.diagnostics.extend(diagnostics);
    for (declaration, edges) in result.declarations.iter_mut().zip(edges) {
        declaration.dependencies = edges;
    }
    if !result.diagnostics.is_empty()
        && result
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Error)
    {
        return result;
    }
    let mut marks = vec![0; result.declarations.len()];
    let mut order = vec![];
    for index in 0..result.declarations.len() {
        cycles(index, &mut marks, &mut vec![], &mut order, &mut result);
    }
    if result.diagnostics.iter().any(|d| d.code() == Code::Cycle) {
        return result;
    }
    types::check(&mut result, context, &names, &order);
    result
}

struct Names<'a> {
    parsed: &'a Parsed,
    declarations: &'a [Declaration],
    names: &'a BTreeMap<String, usize>,
    diagnostics: Vec<Diagnostic>,
    edges: Vec<Vec<usize>>,
}

impl Names<'_> {
    fn report(&mut self, code: Code, node: &Node, subject: &str) {
        self.diagnostics.push(
            positioned(
                code,
                DiagnosticPhase::Check,
                node.span,
                &self.parsed.positions,
            )
            .with_subject(subject),
        );
    }

    fn visit(&mut self, node: &Node, owner: Option<usize>, binders: &mut Vec<String>) {
        match &node.kind {
            Kind::Expression(Expression::Name(name)) => {
                if !binders.contains(name)
                    && !matches!(name.as_str(), "data" | "input" | "view" | "now" | "zone")
                {
                    match self.names.get(name).copied() {
                        Some(index) if !self.declarations[index].state => {
                            // The syntax crate's LiteralRequired check permits
                            // only literal state initializers, so state cannot
                            // own a name-dependency edge here.
                            if let Some(owner) = owner {
                                if !self.edges[owner].contains(&index) {
                                    self.edges[owner].push(index);
                                }
                            }
                        }
                        _ => self.report(Code::UnknownName, node, name),
                    }
                }
            }
            Kind::Expression(Expression::State(name)) => {
                if !self
                    .names
                    .get(name)
                    .is_some_and(|i| self.declarations[*i].state)
                {
                    self.report(Code::UnknownState, node, name);
                }
            }
            Kind::Expression(Expression::Call(name))
                if name == "Each" && node.children.len() == 3 =>
            {
                self.visit(&node.children[0], owner, binders);
                let binder = match &node.children[1].kind {
                    Kind::Expression(Expression::String(name))
                        if identifier(name)
                            && !reserved(name)
                            && !self.names.contains_key(name)
                            && !binders.contains(name) =>
                    {
                        Some(name.clone())
                    }
                    _ => None,
                };
                if binder.is_none() {
                    self.report(Code::EachBinder, &node.children[1], "Each");
                }
                if binders.len() >= 4 {
                    self.report(Code::EachNesting, node, "Each");
                }
                if let Some(name) = binder {
                    binders.push(name);
                    self.visit(&node.children[2], owner, binders);
                    binders.pop();
                } else {
                    self.visit(&node.children[2], owner, binders);
                }
                return;
            }
            Kind::Tag { props, .. } => {
                for prop in props {
                    self.visit(&prop.value, owner, binders);
                }
            }
            _ => {}
        }
        for child in &node.children {
            self.visit(child, owner, binders);
        }
    }
}

fn cycles(
    index: usize,
    marks: &mut [u8],
    stack: &mut Vec<usize>,
    order: &mut Vec<usize>,
    result: &mut Resolved,
) {
    if marks[index] == 2 || result.declarations[index].state {
        return;
    }
    if marks[index] == 1 {
        // A node marked active is on the DFS stack, so the search finds it.
        let Some(start) = stack.iter().position(|entry| *entry == index) else {
            return;
        };
        let mut cycle: Vec<_> = stack[start..]
            .iter()
            .map(|i| result.declarations[*i].name.as_str())
            .collect();
        cycle.push(&result.declarations[index].name);
        result.diagnostics.push(
            positioned(
                Code::Cycle,
                DiagnosticPhase::Check,
                result.declarations[index].expression.span,
                &result.parsed.positions,
            )
            .refine(|diagnostic| diagnostic.with_cycle(&cycle)),
        );
        return;
    }
    marks[index] = 1;
    stack.push(index);
    for dependency in result.declarations[index].dependencies.clone() {
        cycles(dependency, marks, stack, order, result);
    }
    stack.pop();
    marks[index] = 2;
    order.push(index);
}
