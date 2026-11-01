use super::*;
use crate::prepare::types::SiteKind;
use crate::{Operation, Schema, SchemaType};

pub(super) fn from_schema(schema: &Schema) -> Type {
    let mut ty = match schema.kind {
        None => schema
            .enumeration
            .as_ref()
            .map(|values| Type::union(values.iter().map(json_type)))
            .unwrap_or(Type::Unknown),
        Some(SchemaType::Null) => Type::Null,
        Some(SchemaType::Boolean) => Type::Boolean,
        Some(SchemaType::Number) => Type::Number,
        Some(SchemaType::String) => Type::String,
        Some(SchemaType::Array) => Type::List(Box::new(
            schema
                .items
                .as_deref()
                .map(from_schema)
                .unwrap_or(Type::Unknown),
        )),
        Some(SchemaType::Object) => Type::OpenRecord(
            schema
                .properties
                .iter()
                .map(|(key, value)| {
                    let ty = from_schema(value);
                    (
                        key.clone(),
                        if schema.required.contains(key) {
                            ty
                        } else {
                            Type::union([ty, Type::Missing])
                        },
                    )
                })
                .collect(),
        ),
    };
    if schema.nullable {
        ty = Type::union([ty, Type::Null]);
    }
    if let Some(values) = &schema.enumeration {
        ty = Type::Enum(Box::new(ty), values.clone());
    }
    ty
}

pub(super) fn context(nodes: &[Node], context: &CheckContext) -> TypeContext {
    let mut types = TypeContext::default();
    types
        .hosts
        .insert("data".into(), Type::union([Type::Snapshot, Type::Null]));
    if let Some(tool) = &context.tool {
        types
            .hosts
            .insert("input".into(), from_schema(&tool.input.schema));
        types
            .hosts
            .insert("view".into(), from_schema(&tool.view.schema));
    }
    for node in nodes {
        let Kind::Declaration { name, state: false } = &node.kind else {
            continue;
        };
        let Some(expression) = node.children.first() else {
            continue;
        };
        let Kind::Expression(E::Call(constructor)) = &expression.kind else {
            continue;
        };
        let Some(Node {
            kind: Kind::Expression(E::String(operation)),
            ..
        }) = expression.children.first()
        else {
            continue;
        };
        let Some(operation) = context.registry.0.get(operation) else {
            continue;
        };
        let (kind, value) = match (constructor.as_str(), operation) {
            ("Query", Operation::Query { result, .. }) => (SiteKind::Query, from_schema(result)),
            ("Watch", Operation::Watch { item, .. }) => {
                (SiteKind::Watch, Type::List(Box::new(from_schema(item))))
            }
            ("PagedList", Operation::Paged { page, .. }) => {
                (SiteKind::PagedList, from_schema(page).member("items"))
            }
            _ => continue,
        };
        let mut fields = vec![
            ("status".into(), Type::String),
            ("error".into(), Type::union([Type::String, Type::Null])),
            ("stale".into(), Type::Boolean),
        ];
        match kind {
            SiteKind::Query => fields.push(("value".into(), value)),
            SiteKind::Watch => {
                fields.extend([("value".into(), value), ("dropped".into(), Type::Number)])
            }
            SiteKind::PagedList => fields.extend([
                ("items".into(), value),
                ("done".into(), Type::Boolean),
                ("total".into(), Type::union([Type::Number, Type::Null])),
                ("loadingMore".into(), Type::Boolean),
                ("capped".into(), Type::Boolean),
            ]),
        }
        types
            .sites
            .insert(name.clone(), Type::Envelope(kind, fields));
    }
    types
}

pub(super) fn syntax_type(node: &Node) -> Type {
    match &node.kind {
        Kind::Tag { name, .. } => Type::Tagged(name.clone()),
        Kind::Reference { .. } => Type::Snapshot,
        Kind::Handle { .. } => Type::Handle,
        Kind::Expression(E::String(_)) => Type::String,
        Kind::Expression(E::Number(_)) => Type::Number,
        Kind::Expression(E::Boolean(_)) => Type::Boolean,
        Kind::Expression(E::Null) => Type::Null,
        Kind::Expression(E::List) => {
            Type::List(Box::new(Type::union(node.children.iter().map(syntax_type))))
        }
        Kind::Expression(E::Record(keys)) => Type::Record(
            keys.iter()
                .cloned()
                .zip(node.children.iter().map(syntax_type))
                .collect(),
        ),
        Kind::Expression(E::Unary(op)) if op == "-" => Type::Number,
        Kind::Expression(E::Group) => syntax_type(&node.children[0]),
        _ => Type::Unknown,
    }
}

pub(super) fn data_type(ty: &Type) -> bool {
    match ty {
        Type::Enum(base, _) => data_type(base),
        Type::Unknown
        | Type::Number
        | Type::String
        | Type::Boolean
        | Type::Null
        | Type::Missing
        | Type::Object => true,
        Type::List(item) => data_type(item),
        Type::Record(fields) | Type::OpenRecord(fields) => {
            fields.iter().all(|(_, value)| data_type(value))
        }
        Type::Union(types) => types.iter().all(data_type),
        _ => false,
    }
}

/// Reject only a definite mismatch. A future answer or optional field cannot
/// be proven invalid until evaluation has a concrete value (§5.7, §8.2).
pub(super) fn compatible(ty: &Type, schema: &Schema) -> bool {
    match ty {
        Type::Enum(_, values) => values.iter().any(|value| schema.accepts(value)),
        Type::Unknown | Type::Missing => true,
        Type::Union(types) => types.iter().any(|ty| compatible(ty, schema)),
        Type::Null if schema.nullable => true,
        _ => {
            let kind = match schema.kind {
                None => true,
                Some(SchemaType::Null) => matches!(ty, Type::Null),
                Some(SchemaType::Boolean) => matches!(ty, Type::Boolean),
                Some(SchemaType::Number) => matches!(ty, Type::Number),
                Some(SchemaType::String) => matches!(ty, Type::String),
                Some(SchemaType::Array) => matches!(ty, Type::List(_)),
                Some(SchemaType::Object) => {
                    matches!(ty, Type::Record(_) | Type::OpenRecord(_) | Type::Object)
                }
            };
            kind && match ty {
                Type::List(item) => schema
                    .items
                    .as_deref()
                    .is_none_or(|schema| compatible(item, schema)),
                Type::Record(fields) | Type::OpenRecord(fields) => {
                    fields.iter().all(|(key, ty)| {
                        schema
                            .properties
                            .get(key)
                            .is_none_or(|schema| compatible(ty, schema))
                    })
                }
                _ => true,
            }
        }
    }
}

fn json_type(value: &Json) -> Type {
    match value {
        Json::Null => Type::Null,
        Json::Bool(_) => Type::Boolean,
        Json::Number(_) => Type::Number,
        Json::String(_) => Type::String,
        Json::Array(values) => Type::List(Box::new(Type::union(values.iter().map(json_type)))),
        Json::Object(fields) => Type::Record(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), json_type(value)))
                .collect(),
        ),
    }
}
