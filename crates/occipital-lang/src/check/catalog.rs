use cortexkit_rich_ui_syntax::Markdown;

use super::*;
use crate::catalog::{Component, Prop as CatalogProp};
use crate::prepare::types::SiteKind;

impl Checker<'_> {
    pub(super) fn tag(
        &mut self,
        node: &Node,
        name: &str,
        block: bool,
        props: &[Prop],
        place: Place<'_>,
    ) {
        let Some(component) = self.catalog.components.get(name) else {
            self.report(Code::UnknownComponent, node, name, None);
            return;
        };
        // Every place below the tag starts afresh, except where it sits: inside
        // a Slot (this one included) and inside a declaration.
        let inherited = Place {
            slot: place.slot || name == "Slot",
            declared: place.declared,
            ..Place::default()
        };
        let signature = signature(name, component);
        if !component
            .profiles
            .iter()
            .any(|p| p == self.context.profile.name())
        {
            self.report(
                if self.context.profile == Profile::Reply {
                    Code::ReplyForbidden
                } else {
                    Code::Placement
                },
                node,
                name,
                Some(&signature),
            );
            return;
        }
        if place.render && component.flow == "block" && !block && place.inline {
            self.report(Code::Placement, node, name, Some(&signature));
        }
        if place.render {
            self.parent(node, &Type::Tagged(name.into()), place.parent, &signature);
        }
        let has_children =
            node.children.iter().any(meaningful) || prop(props, "children").is_some();
        for (key, schema) in &component.props {
            if schema.rules.get("req") == Some(&Json::Bool(true)) && prop(props, key).is_none() {
                self.report(
                    if schema.t == "action" {
                        Code::ActionRequired
                    } else {
                        Code::MissingProp
                    },
                    node,
                    key,
                    Some(&signature),
                );
            }
        }
        self.forms(node, component, props, has_children, &signature);
        for p in props {
            if p.name == "children" {
                if component.kids.kind == "none" {
                    self.report(Code::Placement, &p.value, name, Some(&signature));
                }
                self.children_value(&p.value, component, &signature);
                if let (Some([min, max]), Some(values)) = (
                    component.kids.items,
                    self.known(&p.value).and_then(|v| v.as_array().cloned()),
                ) {
                    if values.len() < min as usize || values.len() > max as usize {
                        self.report(Code::PropType, &p.value, "children", Some(&signature));
                    }
                }
                self.visit(
                    &p.value,
                    Place {
                        parent: Some(name),
                        inline: component.kids.kind == "inline",
                        ..inherited
                    },
                );
                continue;
            }
            let Some(schema) = component.props.get(&p.name) else {
                self.report(Code::UnknownProp, &p.value, &p.name, Some(&signature));
                self.visit(&p.value, inherited);
                continue;
            };
            if schema
                .rules
                .get("in")
                .and_then(Json::as_array)
                .is_some_and(|profiles| !profiles.iter().any(|p| p == self.context.profile.name()))
            {
                self.report(
                    if self.context.profile == Profile::Reply {
                        Code::ReplyForbidden
                    } else {
                        Code::Placement
                    },
                    &p.value,
                    &p.name,
                    Some(&signature),
                );
            }
            let expected = schema
                .rules
                .get("tIn")
                .and_then(|v| v.get(self.context.profile.name()))
                .and_then(Json::as_str)
                .unwrap_or(&schema.t);
            if expected.starts_with("binding") {
                self.binding(&p.value, schema, component, props, &signature);
            } else {
                if schema.rules.contains_key("record") && !column_records(&self.ty(&p.value)) {
                    self.report(Code::PropType, &p.value, &p.name, Some(&signature));
                }
                if !matches_type(&self.ty(&p.value), expected)
                    || !self.literal_elements(&p.value, expected)
                {
                    self.report(Code::PropType, &p.value, &p.name, Some(&signature));
                }
                if let Some(value) = self.known(&p.value) {
                    if enum_values(self.catalog, schema)
                        .is_some_and(|values| !values.contains(&value))
                    {
                        self.report(Code::Enum, &p.value, &p.name, Some(&signature));
                    }
                    if let Some(code) = bounds(
                        name,
                        &p.name,
                        schema,
                        self.context.profile,
                        &value,
                        DiagnosticPhase::Check,
                    ) {
                        self.report(code, &p.value, &p.name, Some(&signature));
                    }
                    if schema.rules.contains_key("record")
                        && value.as_array().is_some_and(|items| {
                            items.iter().any(|item| {
                                item.is_object()
                                    && (!item.get("key").is_some_and(Json::is_string)
                                        || !item.get("label").is_some_and(Json::is_string))
                            })
                        })
                    {
                        self.report(Code::PropType, &p.value, &p.name, Some(&signature));
                    }
                }
                if let Some((min, max)) = schema.rules.get("items").and_then(item_range) {
                    let definition = self.definition(&p.value);
                    let count = definition.children.len() as u64;
                    if matches!(definition.kind, Kind::Expression(E::List))
                        && (count < min || count > max)
                    {
                        self.report(Code::PropType, &p.value, &p.name, Some(&signature));
                    }
                }
                if schema.rules.get("lit") == Some(&Json::Bool(true)) && !catalog_literal(&p.value)
                {
                    self.report(Code::LiteralRequired, &p.value, &p.name, Some(&signature));
                }
                if let Some(conditions) = schema.rules.get("with").and_then(Json::as_object) {
                    for (other, expected) in conditions {
                        let actual = prop(props, other)
                            .and_then(|p| self.known(p))
                            .or_else(|| component.props.get(other)?.rules.get("def").cloned());
                        if actual.as_ref().is_some_and(|actual| actual != expected) {
                            self.report(Code::PropType, &p.value, &p.name, Some(&signature));
                        }
                    }
                }
            }
            let parent = format!("{name}.{}", p.name);
            self.parent(&p.value, &self.ty(&p.value), Some(&parent), &signature);
            if schema.t == "action" && self.steps(&p.value, &mut 256).is_some_and(|n| n > 16) {
                self.report(Code::LimitSteps, &p.value, &p.name, Some(&signature));
            }
            self.visit(
                &p.value,
                Place {
                    parent: Some(&parent),
                    action: schema.t == "action",
                    ..inherited
                },
            );
        }
        if has_children && component.kids.kind == "none" {
            self.report(Code::Placement, node, name, Some(&signature));
        }
        let count = self.children_nodes(&node.children, component, &signature);
        if component.kids.items.is_some_and(|[min, max]| {
            count.is_some_and(|count| count < min as usize || count > max as usize)
        }) {
            self.report(Code::PropType, node, "children", Some(&signature));
        }
        for child in &node.children {
            self.visit(
                child,
                Place {
                    parent: Some(name),
                    inline: component.kids.kind == "inline",
                    ..inherited
                },
            );
        }
    }

    fn parent(&mut self, node: &Node, ty: &Type, parent: Option<&str>, signature: &str) {
        match ty {
            Type::Tagged(name) => {
                if self
                    .catalog
                    .components
                    .get(name)
                    .and_then(|c| c.rules.get("parent"))
                    .and_then(Json::as_array)
                    .is_some_and(|parents| !parents.iter().any(|p| p.as_str() == parent))
                {
                    self.report(Code::Placement, node, name, Some(signature));
                }
            }
            Type::List(ty) => self.parent(node, ty, parent, signature),
            Type::Union(types) => {
                for ty in types {
                    self.parent(node, ty, parent, signature);
                }
            }
            _ => {}
        }
    }

    pub(super) fn render_value(&mut self, node: &Node, ty: &Type, place: Place<'_>) {
        match ty {
            Type::Tagged(name) => {
                if let Some(component) = self.catalog.components.get(name) {
                    let signature = signature(name, component);
                    self.parent(node, ty, place.parent, &signature);
                    if place.inline && component.flow == "block" {
                        self.report(Code::Placement, node, name, Some(&signature));
                    }
                }
            }
            Type::List(item) => self.render_value(node, item, place),
            Type::Union(types) => {
                for ty in types {
                    self.render_value(node, ty, place);
                }
            }
            _ => {}
        }
    }

    fn literal_elements(&self, node: &Node, expected: &str) -> bool {
        let node = self.definition(node);
        if !matches!(node.kind, Kind::Expression(E::List)) {
            return true;
        }
        let options = split(expected);
        options.into_iter().any(|option| {
            option
                .strip_prefix("list<")
                .and_then(|s| s.strip_suffix('>'))
                .is_none_or(|inner| {
                    node.children
                        .iter()
                        .all(|n| matches_type(&self.ty(n), inner))
                })
        })
    }

    fn binding(
        &mut self,
        node: &Node,
        schema: &CatalogProp,
        component: &Component,
        props: &[Prop],
        signature: &str,
    ) {
        let required = if let Some(t) = schema
            .t
            .strip_prefix("binding<")
            .and_then(|t| t.strip_suffix('>'))
        {
            Some(t.to_owned())
        } else {
            schema.rules.get("bind").and_then(|binding| {
                let by = binding.get("by")?.as_str()?;
                let selector = prop(props, by)
                    .and_then(|p| self.known(p))
                    .or_else(|| component.props.get(by)?.rules.get("def").cloned())?;
                let key = selector
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| selector.to_string());
                Some(binding.get(&key)?.as_str()?.to_owned())
            })
        };
        let valid = if let Kind::Expression(E::State(name)) = &node.kind {
            self.resolved
                .declarations
                .iter()
                .find(|d| d.state && d.name == *name)
                .is_some_and(|d| {
                    required.as_ref().is_some_and(|required| {
                        matches_type(&schema::syntax_type(&d.expression), required)
                    })
                })
        } else {
            false
        };
        if !valid {
            self.report(Code::Binding, node, "value", Some(signature));
        }
    }

    fn forms(
        &mut self,
        node: &Node,
        component: &Component,
        props: &[Prop],
        children: bool,
        signature: &str,
    ) {
        let has = |key: &str| {
            if key == "children" {
                children
            } else {
                prop(props, key).is_some()
            }
        };
        for (rule, min, max) in [("one", 1, 1), ("atMostOne", 0, 1), ("anyOf", 1, usize::MAX)] {
            if let Some(groups) = component.rules.get(rule).and_then(Json::as_array) {
                // Every group in the embedded catalog is a list of prop names.
                for group in groups.iter().filter_map(Json::as_array) {
                    let count = group
                        .iter()
                        .filter_map(Json::as_str)
                        .filter(|key| has(key))
                        .count();
                    if count < min {
                        self.report(Code::MissingProp, node, "content", Some(signature));
                    }
                    if count > max {
                        self.report(Code::PropType, node, "content", Some(signature));
                    }
                }
            }
        }
        if let Some(forms) = component.rules.get("forms").and_then(Json::as_array) {
            let valid = forms.iter().any(|form| {
                form.get("need")
                    .and_then(Json::as_array)
                    .is_none_or(|need| need.iter().filter_map(Json::as_str).all(has))
                    && form
                        .get("deny")
                        .and_then(Json::as_array)
                        .is_none_or(|deny| {
                            deny.iter().filter_map(Json::as_str).all(|key| !has(key))
                        })
                    && form
                        .get("types")
                        .and_then(Json::as_object)
                        .is_none_or(|types| {
                            types.iter().all(|(key, t)| {
                                prop(props, key).is_none_or(|node| {
                                    // Catalog form types are type names; a
                                    // malformed one would match nothing.
                                    t.as_str().is_some_and(|t| {
                                        matches_type(&self.ty(node), t)
                                            && self.literal_elements(node, t)
                                    })
                                })
                            })
                        })
            });
            if !valid {
                let has_needs = forms.iter().any(|form| {
                    // Every catalog form lists its needs.
                    form.get("need")
                        .and_then(Json::as_array)
                        .is_none_or(|need| need.iter().filter_map(Json::as_str).all(has))
                });
                self.report(
                    if has_needs {
                        Code::PropType
                    } else {
                        Code::MissingProp
                    },
                    node,
                    "form",
                    Some(signature),
                );
            }
        }
    }

    fn children_nodes(
        &mut self,
        nodes: &[Node],
        component: &Component,
        signature: &str,
    ) -> Option<usize> {
        let mut count = Some(0);
        for node in nodes.iter().filter(|n| meaningful(n)) {
            match &node.kind {
                Kind::Markdown(Markdown::Paragraph) => {
                    let n = self.children_nodes(&node.children, component, signature);
                    count = count.zip(n).map(|(a, b)| a + b);
                }
                Kind::Tag { name, .. } => {
                    if !component.kids.only.is_empty() && !component.kids.only.contains(name) {
                        self.report(Code::Placement, node, name, Some(signature));
                    }
                    count = count.map(|n| n + 1);
                }
                Kind::Interpolation { .. } => {
                    self.children_value(&node.children[0], component, signature);
                    count = None;
                }
                Kind::Text(_) | Kind::Literal(_) => {
                    if !component.kids.only.is_empty()
                        && !component
                            .kids
                            .only
                            .iter()
                            .any(|k| matches!(k.as_str(), "text" | "markdown"))
                    {
                        self.report(Code::Placement, node, "children", Some(signature));
                    }
                    count = count.map(|n| n + 1);
                }
                Kind::Markdown(markdown) => {
                    let kind = match markdown {
                        Markdown::Emphasis | Markdown::Strong | Markdown::Strikethrough => {
                            "emphasis"
                        }
                        Markdown::Code(_) => "code",
                        Markdown::SoftBreak => continue,
                        Markdown::HardBreak => "text",
                        _ => "markdown",
                    };
                    if component.kids.kind == "inline" && kind == "markdown"
                        || !component.kids.only.is_empty()
                            && !component
                                .kids
                                .only
                                .iter()
                                .any(|k| k == kind || k == "markdown")
                    {
                        self.report(Code::Placement, node, "children", Some(signature));
                    }
                    // Markdown wrappers do not hide forbidden descendant tags.
                    self.children_nodes(&node.children, component, signature);
                    count = count.map(|n| n + 1);
                }
                Kind::Declaration { .. } => {}
                _ => {
                    if !component.kids.only.is_empty()
                        && !component.kids.only.iter().any(|k| k == "markdown")
                    {
                        self.report(Code::Placement, node, "children", Some(signature));
                    }
                    count = count.map(|n| n + 1);
                }
            }
        }
        count
    }

    fn children_value(&mut self, node: &Node, component: &Component, signature: &str) {
        let ty = self.ty(node);
        let expected = if component.kids.only.is_empty()
            || component.kids.only.iter().any(|k| k == "markdown")
        {
            "nodes".to_owned()
        } else {
            component
                .kids
                .only
                .iter()
                .map(|name| {
                    if name == "text" {
                        "string|number|boolean".to_owned()
                    } else {
                        format!("node<{name}>")
                    }
                })
                .collect::<Vec<_>>()
                .join("|")
        };
        let ty = if let Type::List(item) = &ty {
            &**item
        } else {
            &ty
        };
        if !matches_type(ty, &expected) {
            self.report(Code::Placement, node, "children", Some(signature));
        }
    }

    fn steps(&self, node: &Node, remaining: &mut usize) -> Option<usize> {
        *remaining = remaining.checked_sub(1)?;
        let node = self.definition(node);
        match &node.kind {
            Kind::Expression(E::Call(name)) if name == "Action" => {
                node.children.first().and_then(|n| self.steps(n, remaining))
            }
            Kind::Expression(E::List) => node
                .children
                .iter()
                .try_fold(0, |n, child| Some(n + self.steps(child, remaining)?)),
            Kind::Expression(E::Call(name))
                if matches!(
                    name.as_str(),
                    "Set" | "Reset" | "Refresh" | "LoadMore" | "Compose" | "Open"
                ) =>
            {
                Some(1)
            }
            _ => None,
        }
    }
}

fn meaningful(node: &Node) -> bool {
    !matches!(&node.kind, Kind::Text(text) if text.trim_matches([' ', '\t', '\n', '\r']).is_empty())
        && !matches!(node.kind, Kind::Declaration { .. })
}

fn catalog_literal(node: &Node) -> bool {
    match &node.kind {
        Kind::Expression(E::Number(_) | E::String(_) | E::Boolean(_) | E::Null)
        | Kind::Handle { .. }
        | Kind::Reference { .. } => true,
        Kind::Expression(E::List | E::Record(_)) => node.children.iter().all(catalog_literal),
        Kind::Expression(E::Unary(op)) if op == "-" => {
            matches!(node.children[0].kind, Kind::Expression(E::Number(_)))
        }
        _ => false,
    }
}

fn split(t: &str) -> Vec<&str> {
    let (mut depth, mut start, mut parts) = (0, 0, vec![]);
    for (i, byte) in t.bytes().enumerate() {
        match byte {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            b'|' if depth == 0 => {
                parts.push(&t[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&t[start..]);
    parts
}

fn matches_type(ty: &Type, t: &str) -> bool {
    if let Type::Enum(base, _) = ty {
        return matches_type(base, t);
    }
    if matches!(ty, Type::Unknown) {
        return true;
    }
    if let Type::Union(types) = ty {
        return types.iter().any(|ty| matches_type(ty, t));
    }
    split(t).into_iter().any(|t| {
        if let Some(inner) = t.strip_prefix("list<").and_then(|t| t.strip_suffix('>')) {
            return matches!(ty, Type::List(item) if matches_type(item, inner));
        }
        if let Some(inner) = t.strip_prefix("node<").and_then(|t| t.strip_suffix('>')) {
            return matches!(ty, Type::Tagged(name) if inner.split('|').any(|n| n == name));
        }
        if t.starts_with("handle<") {
            return matches!(ty, Type::Handle);
        }
        match t {
            "number" => matches!(ty, Type::Number),
            "string" | "markdown" | "address" | "enum" | "tone" | "toneText" | "aspect" | "gap" => {
                matches!(ty, Type::String)
            }
            "boolean" => matches!(ty, Type::Boolean),
            "record" => matches!(ty, Type::Record(_) | Type::OpenRecord(_) | Type::Object),
            "list" => matches!(ty, Type::List(_)),
            "snapshot" => matches!(ty, Type::Snapshot),
            "rows" => matches!(ty, Type::List(item) if matches_type(item, "record")),
            "action" => matches!(ty, Type::Action | Type::Null | Type::Missing),
            "paged" => matches!(ty, Type::Envelope(SiteKind::PagedList, _)),
            "watch" => matches!(ty, Type::Envelope(SiteKind::Watch, _)),
            "node" | "nodes" => {
                matches!(
                    ty,
                    Type::Tagged(_)
                        | Type::String
                        | Type::Number
                        | Type::Boolean
                        | Type::Null
                        | Type::Missing
                ) || matches!(ty, Type::List(item) if t == "nodes" && matches_type(item, "node"))
            }
            _ => false,
        }
    })
}

fn column_records(ty: &Type) -> bool {
    match ty {
        Type::List(item) | Type::Enum(item, _) => column_records(item),
        Type::Union(types) => types.iter().all(column_records),
        Type::Record(_) | Type::OpenRecord(_) => {
            matches_type(&ty.member("key"), "string") && matches_type(&ty.member("label"), "string")
        }
        _ => true,
    }
}

fn enum_values(catalog: &Catalog, schema: &CatalogProp) -> Option<Vec<Json>> {
    let values: Vec<_> = match schema.t.as_str() {
        "tone" => catalog.vocabulary.tone.clone(),
        "toneText" => catalog.vocabulary.tone_text.clone(),
        "aspect" => catalog.vocabulary.aspect.keys().cloned().collect(),
        "gap" => catalog.vocabulary.gap.keys().cloned().collect(),
        _ => return schema.rules.get("enum").and_then(Json::as_array).cloned(),
    };
    Some(values.into_iter().map(Json::String).collect())
}

fn signature(name: &str, component: &Component) -> String {
    let props = component
        .props
        .iter()
        .map(|(key, schema)| {
            format!(
                "{}{}: {}",
                key,
                if schema.rules.get("req") == Some(&Json::Bool(true)) {
                    ""
                } else {
                    "?"
                },
                schema.t
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("<{name} {props}> {} </{name}>", component.kids.kind)
}

/// A catalog `items` rule: the least and greatest item counts. Every such
/// rule in the embedded catalog is a pair of counts; anything else reads as
/// no rule.
fn item_range(items: &Json) -> Option<(u64, u64)> {
    match items.as_array()?.as_slice() {
        [min, max] => Some((min.as_u64()?, max.as_u64()?)),
        _ => None,
    }
}

/// Scalar range violations are prop-type errors even during evaluation. An
/// evaluated list above its item-count maximum instead exceeds a render size
/// limit, so it reports E_RENDER_LIMIT rather than E_PROP_TYPE.
pub(crate) fn bounds(
    component: &str,
    key: &str,
    schema: &CatalogProp,
    profile: Profile,
    value: &Json,
    phase: DiagnosticPhase,
) -> Option<Code> {
    bound_shape(
        component,
        key,
        schema,
        profile,
        decimal(value),
        value.as_array().map(Vec::len),
        phase,
    )
}

fn bound_shape(
    component: &str,
    key: &str,
    schema: &CatalogProp,
    profile: Profile,
    number: Option<Decimal>,
    count: Option<usize>,
    phase: DiagnosticPhase,
) -> Option<Code> {
    if let Some(number) = number {
        if schema.rules.get("int") == Some(&Json::Bool(true)) && !number.is_integer() {
            return Some(Code::PropType);
        }
        let range = schema
            .rules
            .get("rangeIn")
            .and_then(|v| v.get(profile.name()))
            .or_else(|| schema.rules.get("range"));
        if !schema.rules.contains_key("rangeRule")
            && range
                .and_then(Json::as_array)
                .and_then(|r| match r.as_slice() {
                    // Every catalog range is a pair of numbers; anything
                    // else reads as no range.
                    [min, max] => Some((decimal(min)?, decimal(max)?)),
                    _ => None,
                })
                .is_some_and(|(min, max)| number < min || number > max)
        {
            return Some(Code::PropType);
        }
        if component == "Slider" && key == "step" && number <= Decimal::ZERO {
            return Some(Code::PropType);
        }
    }
    if let (Some(count), Some((min, max))) = (count, schema.rules.get("items").and_then(item_range))
    {
        let count = count as u64;
        if count < min || count > max {
            return Some(if phase == DiagnosticPhase::Evaluate && count > max {
                Code::RenderLimit
            } else {
                Code::PropType
            });
        }
    }
    None
}

pub(crate) fn evaluated_bounds(
    component: &str,
    key: &str,
    profile: Profile,
    value: &crate::Value,
) -> Option<Diagnostic> {
    let catalog = Catalog::tool_templates();
    let schema = catalog.components.get(component)?.props.get(key)?;
    let (number, count) = match value.kind() {
        crate::ValueKind::Number(number) => (Some(*number), None),
        crate::ValueKind::List(items) => (None, Some(items.len())),
        _ => (None, None),
    };
    bound_shape(
        component,
        key,
        schema,
        profile,
        number,
        count,
        DiagnosticPhase::Evaluate,
    )
    .map(|code| Diagnostic::new(code, DiagnosticPhase::Evaluate).with_subject(key))
}
