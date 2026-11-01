use super::*;
use crate::{Operation, Schema, Severity};

impl Checker<'_> {
    pub(super) fn hosts(&mut self) {
        if self.context.execution_dialect != crate::host::EXECUTION_DIALECT {
            self.diagnostics.push(
                Diagnostic::new(Code::UnsupportedVersion, DiagnosticPhase::Check)
                    .with_subject("executionDialect"),
            );
        }
        if matches!(self.context.profile, Profile::ToolRow | Profile::ToolPage) {
            if let Some(tool) = &self.context.tool {
                for (name, projection) in [("input", &tool.input), ("view", &tool.view)] {
                    if !projection.schema.accepts(&projection.value) {
                        self.diagnostics.push(
                            Diagnostic::new(Code::ArgType, DiagnosticPhase::Check)
                                .with_subject(name),
                        );
                    }
                }
            }
        }
    }

    pub(super) fn site(&mut self, node: &Node, name: &str, whole_declaration: bool) {
        if !whole_declaration || !self.context.profile.sites() {
            self.report(Code::SitePlacement, node, name, None);
        }
        if whole_declaration {
            self.sites[0] += 1;
            if name == "Watch" {
                self.sites[1] += 1;
            }
            if name == "PagedList" {
                self.sites[2] += 1;
            }
            if self.sites[0] > 16 || self.sites[1] > 2 || self.sites[2] > 2 {
                self.report(Code::SiteLimit, node, name, None);
            }
        }
        let Some(operation) = node.children.first() else {
            return;
        };
        let Kind::Expression(E::String(operation_name)) = &operation.kind else {
            self.report(Code::LiteralRequired, operation, name, None);
            return;
        };
        // Tool-row and reply profiles reject sites before registry lookup: the
        // refusal does not mean the host's registry entry is unknown.
        if !self.context.profile.sites() {
            return;
        }
        let Some(operation) = self.context.registry.0.get(operation_name) else {
            self.report(Code::UnknownOperation, node, operation_name, None);
            return;
        };
        let (kind, arguments) = match operation {
            Operation::Query { arguments, .. } => ("Query", arguments),
            Operation::Watch { arguments, .. } => ("Watch", arguments),
            Operation::Paged { arguments, .. } => ("PagedList", arguments),
        };
        if kind != name {
            self.report(Code::OperationKind, node, operation_name, None);
        }
        if let Some(args) = node.children.get(1) {
            let ty = self.ty(args);
            let record = matches!(
                ty,
                Type::Unknown | Type::Record(_) | Type::OpenRecord(_) | Type::Object
            ) || matches!(&ty, Type::Union(types) if types.iter().any(|t| matches!(t, Type::Record(_) | Type::OpenRecord(_) | Type::Object)));
            if !record
                || !schema::data_type(&ty)
                || self.static_size(args, &mut 8192).0 > 8192
                || !self.schema_args(args, arguments)
            {
                self.report(Code::SiteArgs, args, name, None);
            }
        }
        if name == "Query" {
            if let Some(default) = node.children.get(2) {
                if !schema::data_type(&self.ty(default)) {
                    self.report(Code::SiteArgs, default, name, None);
                }
            }
            if let Some(refresh) = node.children.get(3) {
                // Only literal intervals are refused here. Computed intervals
                // are not rejected during checking (§8.3).
                if let Kind::Expression(E::Number(_)) | Kind::Expression(E::Unary(_)) = refresh.kind
                {
                    if self
                        .known(refresh)
                        .as_ref()
                        .and_then(decimal)
                        .is_some_and(|n| n < Decimal::from_i64(2) || n > Decimal::from_i64(3600))
                    {
                        self.report(Code::RefreshRange, refresh, name, None);
                    }
                }
            }
        }
        if name == "PagedList" {
            if let Some(size) = node.children.get(2) {
                if self
                    .known(size)
                    .as_ref()
                    .and_then(decimal)
                    .is_some_and(|n| {
                        !n.is_integer() || n < Decimal::ONE || n > Decimal::from_i64(200)
                    })
                {
                    self.report(Code::SiteArgs, size, name, None);
                }
            }
        }
    }

    // A lower bound on canonical bytes avoids expanding shared aliases just to
    // refuse oversized arguments. Unknown fields may be omitted, so they add
    // no bytes; known lists/records still contribute their definite contents.
    fn static_size(&self, node: &Node, visits: &mut usize) -> (usize, bool) {
        let Some(left) = visits.checked_sub(1) else {
            return (0, false);
        };
        *visits = left;
        let node = self.definition(node);
        match &node.kind {
            Kind::Expression(E::List) => {
                let mut size = 2 + node.children.len().saturating_sub(1);
                for child in &node.children {
                    size = (size + self.static_size(child, visits).0).min(8193);
                    if size > 8192 {
                        break;
                    }
                }
                (size, true)
            }
            Kind::Expression(E::Record(keys)) => {
                let (mut size, mut count) = (2, 0);
                for (key, child) in keys.iter().zip(&node.children) {
                    let (bytes, present) = self.static_size(child, visits);
                    if present {
                        size = (size
                            // A JSON value's display is its serialized text,
                            // so this is the quoted, escaped key's length.
                            + serde_json::Value::from(key.as_str()).to_string().len()
                            + 1
                            + bytes
                            + usize::from(count > 0))
                        .min(8193);
                        count += 1;
                    }
                    if size > 8192 {
                        break;
                    }
                }
                (size, true)
            }
            _ => self
                .known(node)
                .and_then(|value| crate::canonical::json(&value).ok())
                .map(|json| (json.len().min(8193), true))
                .unwrap_or((0, false)),
        }
    }

    fn schema_args(&self, node: &Node, schema: &Schema) -> bool {
        if let Some(value) = self.known(node) {
            return schema.accepts(&value);
        }
        if !schema::compatible(&self.ty(node), schema) {
            return false;
        }
        let node = self.definition(node);
        match &node.kind {
            Kind::Expression(E::Record(keys)) => {
                schema.required.iter().all(|key| keys.contains(key))
                    && keys.iter().zip(&node.children).all(|(key, node)| {
                        schema
                            .properties
                            .get(key)
                            .is_none_or(|schema| self.schema_args(node, schema))
                    })
            }
            Kind::Expression(E::List) => schema.items.as_deref().is_none_or(|schema| {
                node.children
                    .iter()
                    .all(|node| self.schema_args(node, schema))
            }),
            _ => true,
        }
    }

    pub(super) fn unused(&mut self) {
        if self
            .resolved
            .diagnostics
            .iter()
            .chain(&self.diagnostics)
            .any(|d| d.severity() == Severity::Error)
        {
            return;
        }
        let mut used = BTreeMap::new();
        let mut pending = vec![];
        for node in &self.resolved.parsed.document.nodes {
            if !matches!(node.kind, Kind::Declaration { .. }) {
                names(node, &mut pending);
            }
        }
        while let Some(name) = pending.pop() {
            if used.insert(name.clone(), ()).is_some() {
                continue;
            }
            if let Some(declaration) = self.resolved.declarations.iter().find(|d| d.name == name) {
                names(&declaration.expression, &mut pending);
            }
        }
        for declaration in &self.resolved.declarations {
            if !used.contains_key(&declaration.name) {
                self.report(
                    Code::Unused,
                    &declaration.expression,
                    &declaration.name,
                    None,
                );
            }
        }
    }
}

fn names(node: &Node, names_out: &mut Vec<String>) {
    match &node.kind {
        Kind::Expression(E::Name(name) | E::State(name)) => names_out.push(name.clone()),
        Kind::Tag { props, .. } => {
            for prop in props {
                names(&prop.value, names_out);
            }
        }
        _ => {}
    }
    for child in &node.children {
        names(child, names_out);
    }
}
