mod functions;

pub(crate) use functions::{function_table, RESERVED_WRITES, SITE_CONSTRUCTORS};

use std::collections::BTreeMap;

use cortexkit_rich_ui_syntax::{Expression as E, Kind, Node};

use super::resolve::Resolved;
use super::{positioned, Positions};
use crate::{Diagnostic, DiagnosticCode as Code, DiagnosticPhase};

const NUMBER: u16 = 1;
const STRING: u16 = 2;
const BOOLEAN: u16 = 4;
const ABSENT: u16 = 8;
const LIST: u16 = 16;
const RECORD: u16 = 32;
const COMPONENT: u16 = 64;
const ACTION: u16 = 128;
const SNAPSHOT: u16 = 256;
const HANDLE: u16 = 512;
const ENVELOPE: u16 = 1024;
const ANY: u16 = u16::MAX;
const SCALAR: u16 = NUMBER | STRING | BOOLEAN | ABSENT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SiteKind {
    Query,
    Watch,
    PagedList,
}

/// Unknown means data-dependent, not a guessed type. Record fields and unions
/// retain schema/literal information without pretending to know future answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Type {
    Unknown,
    Number,
    String,
    Boolean,
    Null,
    Missing,
    List(Box<Type>),
    Record(Vec<(String, Type)>),
    OpenRecord(Vec<(String, Type)>),
    Enum(Box<Type>, Vec<serde_json::Value>),
    Object,
    Tagged(String),
    Action,
    Snapshot,
    Handle,
    Envelope(SiteKind, Vec<(String, Type)>),
    Union(Vec<Type>),
}

impl Type {
    fn mask(&self) -> u16 {
        match self {
            Self::Unknown => ANY,
            Self::Number => NUMBER,
            Self::String => STRING,
            Self::Boolean => BOOLEAN,
            Self::Null | Self::Missing => ABSENT,
            Self::List(_) => LIST,
            Self::Record(_) | Self::OpenRecord(_) | Self::Object => RECORD,
            Self::Tagged(_) => COMPONENT,
            Self::Action => ACTION,
            Self::Snapshot => SNAPSHOT,
            Self::Handle => HANDLE,
            Self::Envelope(_, _) => ENVELOPE,
            Self::Union(types) => types.iter().fold(0, |mask, ty| mask | ty.mask()),
            Self::Enum(base, _) => base.mask(),
        }
    }

    pub(crate) fn union(types: impl IntoIterator<Item = Type>) -> Self {
        let mut unique = vec![];
        for ty in types {
            let members = if let Self::Union(members) = ty {
                members
            } else {
                vec![ty]
            };
            for member in members {
                if member == Self::Unknown {
                    return Self::Unknown;
                }
                if !unique.contains(&member) {
                    unique.push(member);
                }
            }
        }
        if unique.len() > 1 {
            return Self::Union(unique);
        }
        unique.pop().unwrap_or(Self::Unknown)
    }

    pub(crate) fn member(&self, name: &str) -> Self {
        match self {
            Self::Enum(base, _) => base.member(name),
            Self::OpenRecord(fields) => fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, ty)| ty.clone())
                .unwrap_or(Self::Unknown),
            Self::Record(fields) | Self::Envelope(_, fields) => fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, ty)| ty.clone())
                .unwrap_or(Self::Missing),
            Self::List(element) => Self::List(Box::new(element.member(name))),
            Self::Union(types) => Self::union(types.iter().map(|ty| ty.member(name))),
            Self::Snapshot => match name {
                "text" | "reason" | "tool" => Self::union([Self::String, Self::Missing]),
                "state" => Self::String,
                "bytes" => Self::union([Self::Number, Self::Missing]),
                "value" => Self::Unknown,
                "rows" => Self::union([Self::List(Box::new(Self::Unknown)), Self::Missing]),
                _ => Self::Missing,
            },
            Self::Unknown | Self::Object => Self::Unknown,
            _ => Self::Missing,
        }
    }

    fn element(&self) -> Self {
        match self {
            Self::Enum(base, _) => base.element(),
            Self::List(element) => Self::union([*element.clone(), Self::Missing]),
            Self::Union(types) => Self::union(types.iter().map(Self::element)),
            Self::Unknown => Self::Unknown,
            _ => Self::Missing,
        }
    }

    fn index(&self, key: Option<&str>, index: &Self) -> Self {
        match self {
            Self::Enum(base, _) => base.index(key, index),
            Self::Record(fields) => match key {
                Some(key) => self.member(key),
                None if index.mask() & STRING != 0 => Self::union(
                    fields
                        .iter()
                        .map(|(_, ty)| ty.clone())
                        .chain([Self::Missing]),
                ),
                None => Self::Missing,
            },
            Self::Object if index.mask() & STRING != 0 => Self::Unknown,
            Self::OpenRecord(_) => key.map(|key| self.member(key)).unwrap_or(Self::Unknown),
            Self::List(_) if index.mask() & NUMBER != 0 => self.element(),
            Self::Snapshot | Self::Envelope(_, _) => match key {
                Some(key) => self.member(key),
                None => Self::Missing,
            },
            Self::Union(types) => Self::union(types.iter().map(|ty| ty.index(key, index))),
            Self::Unknown => Self::Unknown,
            _ => Self::Missing,
        }
    }
}

/// Later profile/schema checks supply known host shapes and site envelopes.
/// Availability/authorization is deliberately not decided by type inference.
pub(crate) struct TypeContext {
    pub hosts: BTreeMap<String, Type>,
    pub sites: BTreeMap<String, Type>,
}

impl Default for TypeContext {
    fn default() -> Self {
        Self {
            hosts: [
                ("data".into(), Type::Unknown),
                ("input".into(), Type::Unknown),
                ("view".into(), Type::Unknown),
                ("now".into(), Type::Number),
                ("zone".into(), Type::String),
            ]
            .into(),
            sites: BTreeMap::new(),
        }
    }
}

pub(super) fn check(
    result: &mut Resolved,
    context: &TypeContext,
    names: &BTreeMap<String, usize>,
    order: &[usize],
) {
    let mut checker = Checker {
        parsed: &result.parsed,
        positions: &result.parsed.positions,
        values: context.hosts.clone(),
        states: BTreeMap::new(),
        diagnostics: vec![],
        node_types: BTreeMap::new(),
    };
    // State has a fixed outer kind, not the shape of its initial data (language
    // spec §4.10), since later control input can change the data's shape.
    for declaration in &mut result.declarations {
        if declaration.state {
            let initial = checker.infer(&declaration.expression, &mut vec![]);
            declaration.ty = match initial {
                Type::Number | Type::String | Type::Boolean => Type::union([initial, Type::Null]),
                Type::Null => Type::union([Type::Number, Type::String, Type::Boolean, Type::Null]),
                Type::List(_) => Type::List(Box::new(Type::Unknown)),
                Type::Record(_) => Type::Object,
                _ => Type::Unknown,
            };
            checker
                .states
                .insert(declaration.name.clone(), declaration.ty.clone());
        }
    }
    for index in order {
        let declaration = &mut result.declarations[*index];
        let inferred = checker.infer(&declaration.expression, &mut vec![]);
        declaration.ty = if matches!(&declaration.expression.kind, Kind::Expression(E::Call(name)) if matches!(name.as_str(), "Query" | "Watch" | "PagedList"))
        {
            context
                .sites
                .get(&declaration.name)
                .cloned()
                .unwrap_or(inferred)
        } else {
            inferred
        };
        checker
            .values
            .insert(declaration.name.clone(), declaration.ty.clone());
    }
    for node in &result.parsed.document.nodes {
        if !matches!(node.kind, Kind::Declaration { .. }) {
            checker.infer(node, &mut vec![]);
        }
    }
    debug_assert_eq!(names.len(), result.declarations.len());
    result.diagnostics.extend(checker.diagnostics);
    result.node_types = checker.node_types;
}

struct Checker<'a> {
    parsed: &'a super::Parsed,
    positions: &'a Positions,
    values: BTreeMap<String, Type>,
    states: BTreeMap<String, Type>,
    diagnostics: Vec<Diagnostic>,
    node_types: BTreeMap<(usize, usize), Type>,
}

impl Checker<'_> {
    fn unsupported(&mut self, node: &Node) -> Type {
        match self.parsed.unsupported(node) {
            Ok(_) => Type::String,
            Err(code) => {
                self.report(code, node, "syntax", None);
                Type::Unknown
            }
        }
    }
    fn report(&mut self, code: Code, node: &Node, subject: &str, signature: Option<&str>) {
        let mut diagnostic = positioned(code, DiagnosticPhase::Check, node.span, self.positions)
            .with_subject(subject);
        if let Some(signature) = signature {
            diagnostic = diagnostic.with_signature(signature);
        }
        self.diagnostics.push(diagnostic);
    }

    fn require(
        &mut self,
        ty: &Type,
        mask: u16,
        node: &Node,
        subject: &str,
        signature: Option<&str>,
    ) {
        if ty.mask() & mask == 0 {
            self.report(Code::ArgType, node, subject, signature);
        }
    }

    fn infer(&mut self, node: &Node, binders: &mut Vec<(String, Type)>) -> Type {
        let ty = self.infer_inner(node, binders);
        self.node_types
            .insert((node.span.start, node.span.end), ty.clone());
        ty
    }

    fn infer_inner(&mut self, node: &Node, binders: &mut Vec<(String, Type)>) -> Type {
        if let Kind::Expression(E::Call(name)) = &node.kind {
            return self.call(name, node, binders);
        }
        if let Kind::Tag { props, .. } = &node.kind {
            for prop in props {
                self.infer(&prop.value, binders);
            }
        }
        let args: Vec<_> = node
            .children
            .iter()
            .map(|child| self.infer(child, binders))
            .collect();
        match &node.kind {
            Kind::Markdown(markdown) if crate::eval::tree::kind_of(markdown).is_none() => {
                self.unsupported(node)
            }
            Kind::Reference { .. } => Type::Snapshot,
            Kind::Handle { .. } => Type::Handle,
            Kind::Tag { name, .. } => Type::Tagged(name.clone()),
            Kind::Expression(expression) => match expression {
                E::Number(_) => Type::Number,
                E::String(_) => Type::String,
                E::Boolean(_) => Type::Boolean,
                E::Null => Type::Null,
                E::Name(name) => binders
                    .iter()
                    .rev()
                    .find(|(key, _)| key == name)
                    .map(|(_, ty)| ty)
                    .or_else(|| self.values.get(name))
                    .cloned()
                    .unwrap_or(Type::Unknown),
                E::State(name) => self.states.get(name).cloned().unwrap_or(Type::Unknown),
                E::Group => args[0].clone(),
                E::List => Type::List(Box::new(Type::union(args))),
                E::Record(keys) => Type::Record(keys.iter().cloned().zip(args).collect()),
                E::Member(name) => {
                    if let Type::Envelope(_, fields) = &args[0] {
                        if !fields.iter().any(|(key, _)| key == name) {
                            self.report(Code::EnvelopeMember, node, name, None);
                        }
                    }
                    args[0].member(name)
                }
                E::Index => {
                    let key = if let Kind::Expression(E::String(key)) = &node.children[1].kind {
                        Some(key.as_str())
                    } else {
                        None
                    };
                    if let (Type::Envelope(_, fields), Some(key)) = (&args[0], key) {
                        if !fields.iter().any(|(name, _)| name == key) {
                            self.report(Code::EnvelopeMember, node, key, None);
                        }
                    }
                    args[0].index(key, &args[1])
                }
                E::Unary(op) => {
                    self.require(
                        &args[0],
                        if op == "!" { BOOLEAN | ABSENT } else { NUMBER },
                        node,
                        op,
                        None,
                    );
                    if op == "!" {
                        Type::Boolean
                    } else {
                        Type::Number
                    }
                }
                E::Binary(op) => self.binary(op, node, &args),
                E::Conditional => {
                    self.require(&args[0], BOOLEAN | ABSENT, &node.children[0], "?:", None);
                    Type::union([args[1].clone(), args[2].clone()])
                }
                // `infer_inner` sends calls to `call` before inferring the
                // children, since calls need binder scope. Delegating here
                // too keeps this match total without a panic.
                E::Call(name) => self.call(name, node, binders),
                _ => self.unsupported(node),
            },
            _ => Type::Unknown,
        }
    }

    fn binary(&mut self, op: &str, node: &Node, args: &[Type]) -> Type {
        match op {
            "??" => {
                let left = match &args[0] {
                    Type::Null | Type::Missing => return args[1].clone(),
                    Type::Union(types) => {
                        Type::union(types.iter().filter(|ty| ty.mask() != ABSENT).cloned())
                    }
                    ty => ty.clone(),
                };
                if args[0].mask() & ABSENT == 0 {
                    left
                } else {
                    Type::union([left, args[1].clone()])
                }
            }
            "==" | "!=" => {
                for (child, ty) in node.children.iter().zip(args) {
                    self.require(ty, SCALAR | LIST | RECORD | HANDLE, child, op, None);
                }
                Type::Boolean
            }
            "&&" | "||" => {
                for (child, ty) in node.children.iter().zip(args) {
                    self.require(ty, BOOLEAN | ABSENT, child, op, None);
                }
                Type::Boolean
            }
            "+" => {
                let a = args[0].mask();
                let b = args[1].mask();
                let numeric = a & NUMBER != 0 && b & NUMBER != 0;
                let textual =
                    (a & STRING != 0 && b & SCALAR != 0) || (b & STRING != 0 && a & SCALAR != 0);
                if !numeric && !textual {
                    self.report(Code::ArgType, node, op, None);
                }
                match (numeric, textual) {
                    (true, true) => Type::union([Type::Number, Type::String]),
                    (_, true) => Type::String,
                    _ => Type::Number,
                }
            }
            "<" | "<=" | ">" | ">=" => {
                // An absent operand gives false; otherwise both must be numbers
                // or both strings (language spec §4.12), never an implicit conversion.
                let a = args[0].mask();
                let b = args[1].mask();
                if a & ABSENT == 0 && b & ABSENT == 0 && a & b & (NUMBER | STRING) == 0 {
                    self.report(Code::ArgType, node, op, None);
                }
                Type::Boolean
            }
            _ => {
                for (child, ty) in node.children.iter().zip(args) {
                    self.require(ty, NUMBER, child, op, None);
                }
                Type::Number
            }
        }
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    use cortexkit_rich_ui_syntax::Mode;

    #[test]
    fn unsupported_syntax_type_is_literal_in_reply_and_refused_in_program() {
        for mode in [Mode::Reply, Mode::Program] {
            let mut parsed = crate::prepare::parse(b"{1 + 2}", Mode::Program).unwrap();
            parsed.document.mode = mode;
            let mut checker = Checker {
                parsed: &parsed,
                positions: &parsed.positions,
                values: BTreeMap::new(),
                states: BTreeMap::new(),
                diagnostics: vec![],
                node_types: BTreeMap::new(),
            };
            let ty = checker.unsupported(&parsed.document.nodes[0]);
            if mode == Mode::Reply {
                assert_eq!(ty, Type::String);
                assert!(checker.diagnostics.is_empty());
            } else {
                assert_eq!(ty, Type::Unknown);
                assert_eq!(checker.diagnostics.len(), 1);
                assert_eq!(checker.diagnostics[0].code(), Code::UnsupportedVersion);
                assert_eq!(checker.diagnostics[0].phase(), DiagnosticPhase::Check);
            }
        }
    }
}
