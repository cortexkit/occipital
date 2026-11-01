//! The reply teaching text: what a model is told about the display tags it
//! may write in a reply, generated from the embedded catalog's
//! `teaching.reply` section.
//!
//! The catalog states the recipe: the rules whose needs the runner meets,
//! then, for each component in `order` that passes the `filter`, a signature
//! line, its one-line summary and the examples whose needs the runner meets.
//! This module follows that recipe for a runner that records no tool-output
//! references, no Artifact or File handles and no map records, because no
//! runner stores those records for replies yet. A tag that needs one of them
//! would always draw as unavailable, so it is not taught.
//!
//! The text is a pure function of the embedded catalog: the same catalog
//! always gives the same bytes, so the text's digest changes only when the
//! catalog does. Every session pays for this text in its prompt, so it holds
//! nothing beyond what the catalog's recipe asks for.

use std::sync::OnceLock;

use serde_json::Value;

use crate::catalog::{Catalog, Component, Prop};

/// The profile the reply teaching describes.
const REPLY: &str = "reply";

/// The reply teaching text and the parts a reader may want to check
/// separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyTeaching {
    /// The text to put in a model's prompt.
    pub text: String,
    /// Every example the text contains, each exactly as it appears there.
    /// Each one is a complete reply that checks clean under the reply
    /// profile.
    pub examples: Vec<String>,
    /// The tags the text teaches, in the order it teaches them.
    pub tags: Vec<String>,
}

/// The reply teaching text of the embedded catalog, built once per process.
///
/// It teaches the tags a reply can use when the runner records no tool-output
/// references, handles or map records: in catalog v1, Badge, Styled, Callout,
/// Table with written rows, and LinkCard. LinkCard stays in because the
/// store-time network is not known when the prompt is written; the catalog
/// drops it only when the network is known to be off.
pub fn reply_teaching() -> &'static ReplyTeaching {
    static TEACHING: OnceLock<ReplyTeaching> = OnceLock::new();
    TEACHING.get_or_init(|| build(Catalog::shared(), Runner::default()))
}

/// What the runner records at store time, as the catalog's `capabilities`
/// section names it. Each capability a reply tag or rule needs and the runner
/// lacks removes that tag or rule from the text.
#[derive(Clone, Copy, Debug, Default)]
struct Runner {
    /// `$$tc(N)` references to earlier tool output.
    refs: bool,
    /// Artifact and File handles.
    handles: bool,
    /// The record a Map draws from.
    maps: bool,
}

impl Runner {
    /// Whether the runner meets one catalog need. A need may name
    /// alternatives, as in `refs|handles`; meeting any one of them is enough.
    fn meets(self, need: &str) -> bool {
        need.split('|').any(|one| match one {
            "refs" => self.refs,
            "handles" => self.handles,
            "maps" => self.maps,
            _ => false,
        })
    }

    /// Whether the runner meets every need in a catalog `needs` list. A
    /// missing or malformed list needs nothing.
    fn meets_all(self, needs: Option<&Value>) -> bool {
        strings(needs).iter().all(|need| self.meets(need))
    }
}

/// The strings of a JSON array, skipping anything that is not a string.
fn strings(value: Option<&Value>) -> Vec<&str> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn build(catalog: &Catalog, runner: Runner) -> ReplyTeaching {
    let section = catalog.teaching(REPLY);
    let rules = section
        .and_then(|section| section.get("rules"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut text = String::new();
    for rule in rules {
        let Some(line) = rule.get("text").and_then(Value::as_str) else {
            continue;
        };
        if runner.meets_all(rule.get("needs")) {
            text.push_str(line);
            text.push('\n');
        }
    }

    // Decide every taught tag first: a signature lists the tags a component
    // may hold, and must not name one the text never teaches.
    let order = strings(section.and_then(|section| section.get("order")));
    let taught: Vec<(&str, &Component)> = order
        .into_iter()
        .filter_map(|name| {
            let component = catalog.components.get(name)?;
            Shape::of(catalog, component, runner).map(|_| (name, component))
        })
        .collect();
    let tags: Vec<String> = taught.iter().map(|(name, _)| (*name).to_owned()).collect();

    let mut examples = Vec::new();
    for (name, component) in &taught {
        let Some(shape) = Shape::of(catalog, component, runner) else {
            continue;
        };
        text.push('\n');
        text.push_str(&shape.signature(name, component, &tags));
        text.push('\n');
        text.push_str(&component.sum);
        text.push('\n');
        let written = component
            .rules
            .get("teach")
            .and_then(|teach| teach.get("ex"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for example in written {
            let Some(source) = example.get("src").and_then(Value::as_str) else {
                continue;
            };
            if runner.meets_all(example.get("needs")) {
                text.push_str(source);
                text.push('\n');
                examples.push(source.to_owned());
            }
        }
    }
    ReplyTeaching {
        text,
        examples,
        tags,
    }
}

/// The props of a component a reply can write under a runner, each with the
/// type it may take and whether it must be written.
struct Shape {
    /// `(name, type, required, default)` in signature order: required props
    /// first, then optional ones, each group by name.
    props: Vec<(String, String, bool, Option<String>)>,
}

impl Shape {
    /// The writable shape of `component`, or `None` when a reply cannot use
    /// it under `runner`: it is not in the reply profile, the runner lacks a
    /// capability the component itself needs, or none of its forms can be
    /// written.
    fn of(catalog: &Catalog, component: &Component, runner: Runner) -> Option<Self> {
        if !component.profiles.iter().any(|profile| profile == REPLY)
            || !runner.meets_all(component.rules.get("needs"))
        {
            return None;
        }
        // A component with forms is written in one of them; the form decides
        // which props it needs and may narrow a prop's type. Only a form
        // whose needed props all have a writable type can be written.
        let forms = component
            .rules
            .get("forms")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let writable: Vec<&Value> = forms
            .iter()
            .filter(|form| {
                strings(form.get("need")).iter().all(|need| {
                    component.props.get(*need).is_some_and(|prop| {
                        reply_type(catalog, prop, Some(form), need, runner).is_some()
                    })
                })
            })
            .collect();
        if !forms.is_empty() && writable.is_empty() {
            return None;
        }
        // With exactly one writable form its needed props are required and
        // its type overrides apply; with several, no form is assumed.
        let only = match writable.as_slice() {
            [form] => Some(*form),
            _ => None,
        };
        let needed = strings(only.and_then(|form| form.get("need")));

        let mut props = Vec::new();
        for (name, prop) in &component.props {
            let Some(shown) = reply_type(catalog, prop, only, name, runner) else {
                // A required prop no reply can write makes the tag unusable.
                if prop.rules.get("req") == Some(&Value::Bool(true)) {
                    return None;
                }
                continue;
            };
            let required = prop.rules.get("req") == Some(&Value::Bool(true))
                || needed.contains(&name.as_str());
            let default = prop.rules.get("def").map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            });
            props.push((name.clone(), shown, required, default));
        }
        props.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        Some(Self { props })
    }

    /// The signature line: `<Name req: type, opt?: type = default>children</Name>`,
    /// or a self-closing tag when the component holds no children. The
    /// children part names what the tag may hold, leaving out tags the text
    /// does not teach.
    fn signature(&self, name: &str, component: &Component, taught: &[String]) -> String {
        let props: Vec<String> = self
            .props
            .iter()
            .map(|(prop, shown, required, default)| {
                let mark = if *required { "" } else { "?" };
                match default {
                    Some(default) => format!("{prop}{mark}: {shown} = {default}"),
                    None => format!("{prop}{mark}: {shown}"),
                }
            })
            .collect();
        let opening = if props.is_empty() {
            format!("<{name}")
        } else {
            format!("<{name} {}", props.join(", "))
        };
        if component.kids.kind == "none" {
            return format!("{opening} />");
        }
        let held: Vec<&str> = component
            .kids
            .only
            .iter()
            .map(String::as_str)
            .filter(|kid| {
                // Lowercase entries (text, markdown, code) are content kinds,
                // not tags, and are always writable.
                kid.starts_with(|c: char| c.is_ascii_lowercase())
                    || taught.iter().any(|tag| tag == kid)
            })
            .collect();
        let children = if held.is_empty() {
            component.kids.kind.clone()
        } else {
            held.join("|")
        };
        format!("{opening}>{children}</{name}>")
    }
}

/// The type a reply may give `prop`, keeping only the alternatives the
/// runner can write, or `None` when it can write none. A prop limited to
/// other profiles has no reply type. `form` may narrow the prop's type.
fn reply_type(
    catalog: &Catalog,
    prop: &Prop,
    form: Option<&Value>,
    name: &str,
    runner: Runner,
) -> Option<String> {
    if let Some(profiles) = prop.rules.get("in") {
        if !strings(Some(profiles)).contains(&REPLY) {
            return None;
        }
    }
    let declared = form
        .and_then(|form| form.get("types"))
        .and_then(|types| types.get(name))
        .or_else(|| prop.rules.get("tIn").and_then(|types| types.get(REPLY)))
        .and_then(Value::as_str)
        .unwrap_or(&prop.t);
    if declared == "enum" {
        let values: Vec<String> = strings(prop.rules.get("enum"))
            .iter()
            .map(|value| format!("\"{value}\""))
            .collect();
        return (!values.is_empty()).then(|| values.join("|"));
    }
    writable(catalog, declared, runner)
}

/// The alternatives of a union type the runner can write, joined again, or
/// `None` when it can write none. A tag (`node<…>`) is never a reply value,
/// and a type the catalog says needs a capability is writable only when the
/// runner has it. A list is writable when its element type is.
fn writable(catalog: &Catalog, union: &str, runner: Runner) -> Option<String> {
    let kept: Vec<String> = alternatives(union)
        .into_iter()
        .filter_map(|member| {
            if member.starts_with("node<") {
                return None;
            }
            if let Some(element) = member
                .strip_prefix("list<")
                .and_then(|rest| rest.strip_suffix('>'))
            {
                return writable(catalog, element, runner).map(|kept| format!("list<{kept}>"));
            }
            // The types table describes every handle type under one entry.
            let entry = if member.starts_with("handle<") {
                "handle<K>"
            } else {
                member
            };
            catalog
                .type_needs(entry)
                .iter()
                .all(|need| runner.meets(need))
                .then(|| member.to_owned())
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join("|"))
}

/// The top-level alternatives of a union type: `list<a|b>|c` is `list<a|b>`
/// and `c`, because a `|` inside angle brackets belongs to the inner type.
fn alternatives(union: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, c) in union.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => {
                parts.push(&union[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&union[start..]);
    parts
}

#[cfg(test)]
mod tests;
