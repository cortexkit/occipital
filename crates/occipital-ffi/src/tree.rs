//! The tree JSON (language spec §14.3 tree ABI) and how a tree is rebuilt
//! from it. The FFI is the reference encoder of this shape:
//!
//! - The root is `{"m":"root","k":[blocks]}`.
//! - A component is `{"c":"Badge","p":{props},"k":[children]}`. `p` lists
//!   the props in the tree's order (source order, then bindings); its keys
//!   are unique.
//! - An error cell is `{"c":"Badge","e":"E_PROP_TYPE"}`, an inline error
//!   marker `{"e":"E_TYPE"}`, text `{"t":"…"}` and a reference chip
//!   `{"r":"7"}`.
//! - A Markdown node is `{"m":kind,…,"k":[children]}`, with kind one of `p`,
//!   `h` (with `level`), `hr`, `codeblock` (with `lang` and `text`), `code`
//!   (with `text`), `quote`, `list` (with `ordered`, and `start` when
//!   ordered), `item`, `task` (with `checked`), `table`, `thead`, `row`,
//!   `cell`, `em`, `strong`, `del`, `link` (with `href`, the destination as
//!   written, `title` and `image`), `softbreak` and `br`.
//! - A prop value is JSON `null`, a boolean, a string or an array (a list)
//!   as itself, and otherwise an object with one member naming its kind:
//!   `{"number":"12.5"}` (a canonical decimal string),
//!   `{"record":{fields in order}}`, `{"node":node}`,
//!   `{"action":[{"op":"set","args":[values]}]}`,
//!   `{"snapshot":{"record_id","state","value"}}` (`value` absent when the
//!   snapshot has no parsed value), `{"handleRecord":id}`,
//!   `{"site":{"name":…}}`, `{"binding":{"state":"$page","type":"number"}}`
//!   and `{"error":"E_TYPE"}`.
//!
//! Decoding is closed: an unknown member, a missing member, a duplicate key
//! or a value of the wrong kind is an error. Every text value (names, text,
//! link and code attributes, prop keys and string values) is rebuilt
//! as [`DisplayText`], which sanitizes it again; markers such as `"root"`,
//! Markdown kinds and tags are matched, not kept. Tree text is already
//! sanitized, so an encoded tree decodes to an equal tree. JSON nesting deeper than
//! serde_json's limit of 128 levels does not decode.

use std::collections::BTreeSet;
use std::fmt;

use occipital_lang::{
    ActionOperation, DiagnosticCode, DisplayText, MarkdownKind, Tree, TreeNode, TreeStep, TreeValue,
};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};

use crate::decimal;

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// A tree as tree JSON.
pub(crate) struct TreeJson<'a>(pub(crate) &'a Tree);

impl Serialize for TreeJson<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("m", "root")?;
        map.serialize_entry("k", &Nodes(&self.0.blocks))?;
        map.end()
    }
}

struct Nodes<'a>(&'a [TreeNode]);

impl Serialize for Nodes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for node in self.0 {
            seq.serialize_element(&Node(node))?;
        }
        seq.end()
    }
}

struct Node<'a>(&'a TreeNode);

impl Serialize for Node<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            TreeNode::Markdown { kind, children } => {
                map.serialize_entry("m", markdown_name(kind))?;
                match kind {
                    MarkdownKind::Heading(level) => map.serialize_entry("level", level)?,
                    MarkdownKind::CodeBlock { language, text } => {
                        map.serialize_entry("lang", language.as_str())?;
                        map.serialize_entry("text", text.as_str())?;
                    }
                    MarkdownKind::Code(text) => map.serialize_entry("text", text.as_str())?,
                    MarkdownKind::List { start } => {
                        map.serialize_entry("ordered", &start.is_some())?;
                        if let Some(start) = start {
                            map.serialize_entry("start", start)?;
                        }
                    }
                    MarkdownKind::Task(checked) => map.serialize_entry("checked", checked)?,
                    MarkdownKind::Link {
                        destination,
                        title,
                        image,
                    } => {
                        map.serialize_entry("href", destination.as_str())?;
                        map.serialize_entry("title", title.as_str())?;
                        map.serialize_entry("image", image)?;
                    }
                    _ => {}
                }
                map.serialize_entry("k", &Nodes(children))?;
            }
            TreeNode::Component {
                name,
                props,
                children,
            } => {
                map.serialize_entry("c", name.as_str())?;
                map.serialize_entry("p", &Fields(props))?;
                map.serialize_entry("k", &Nodes(children))?;
            }
            TreeNode::Text(text) => map.serialize_entry("t", text.as_str())?,
            TreeNode::Reference(n) => map.serialize_entry("r", n.as_str())?,
            TreeNode::ErrorMarker(code) => map.serialize_entry("e", code.as_str())?,
            TreeNode::ErrorCell { name, code } => {
                map.serialize_entry("c", name.as_str())?;
                map.serialize_entry("e", code.as_str())?;
            }
        }
        map.end()
    }
}

/// The name a Markdown kind has in `m`.
fn markdown_name(kind: &MarkdownKind) -> &'static str {
    match kind {
        MarkdownKind::Paragraph => "p",
        MarkdownKind::Heading(_) => "h",
        MarkdownKind::ThematicBreak => "hr",
        MarkdownKind::CodeBlock { .. } => "codeblock",
        MarkdownKind::Code(_) => "code",
        MarkdownKind::Quote => "quote",
        MarkdownKind::List { .. } => "list",
        MarkdownKind::Item => "item",
        MarkdownKind::Task(_) => "task",
        MarkdownKind::Table => "table",
        MarkdownKind::TableHead => "thead",
        MarkdownKind::TableRow => "row",
        MarkdownKind::TableCell => "cell",
        MarkdownKind::Emphasis => "em",
        MarkdownKind::Strong => "strong",
        MarkdownKind::Strikethrough => "del",
        MarkdownKind::Link { .. } => "link",
        MarkdownKind::SoftBreak => "softbreak",
        MarkdownKind::HardBreak => "br",
    }
}

/// Props or record fields as one JSON object, in order.
struct Fields<'a>(&'a [(DisplayText, TreeValue)]);

impl Serialize for Fields<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in self.0 {
            map.serialize_entry(key.as_str(), &Value(value))?;
        }
        map.end()
    }
}

struct Values<'a>(&'a [TreeValue]);

impl Serialize for Values<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&Value(value))?;
        }
        seq.end()
    }
}

struct Value<'a>(&'a TreeValue);

impl Serialize for Value<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            TreeValue::Null => serializer.serialize_unit(),
            TreeValue::Boolean(value) => serializer.serialize_bool(*value),
            TreeValue::String(text) => serializer.serialize_str(text.as_str()),
            TreeValue::List(values) => Values(values).serialize(serializer),
            other => {
                let mut map = serializer.serialize_map(Some(1))?;
                match other {
                    TreeValue::Number(number) => {
                        map.serialize_entry("number", &decimal::encode(*number))?
                    }
                    TreeValue::Record(fields) => map.serialize_entry("record", &Fields(fields))?,
                    TreeValue::Node(node) => map.serialize_entry("node", &Node(node))?,
                    TreeValue::Action(steps) => map.serialize_entry("action", &Steps(steps))?,
                    TreeValue::Snapshot {
                        record_id,
                        state,
                        value,
                    } => map.serialize_entry(
                        "snapshot",
                        &SnapshotJson {
                            record_id: record_id.as_str(),
                            state: state.as_str(),
                            value: value.as_deref().map(Value),
                        },
                    )?,
                    TreeValue::Handle(id) => map.serialize_entry("handleRecord", id.as_str())?,
                    TreeValue::Site(name) => map.serialize_entry(
                        "site",
                        &SiteJson {
                            name: name.as_str(),
                        },
                    )?,
                    TreeValue::Binding { state, kind } => map.serialize_entry(
                        "binding",
                        &BindingJson {
                            state: format!("${}", state.as_str()),
                            kind,
                        },
                    )?,
                    TreeValue::Error(code) => map.serialize_entry("error", code.as_str())?,
                    // Handled above, before the map was opened.
                    TreeValue::Null
                    | TreeValue::Boolean(_)
                    | TreeValue::String(_)
                    | TreeValue::List(_) => {}
                }
                map.end()
            }
        }
    }
}

#[derive(Serialize)]
struct SnapshotJson<'a> {
    record_id: &'a str,
    state: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value<'a>>,
}

#[derive(Serialize)]
struct SiteJson<'a> {
    name: &'a str,
}

#[derive(Serialize)]
struct BindingJson<'a> {
    state: String,
    #[serde(rename = "type")]
    kind: &'a str,
}

struct Steps<'a>(&'a [TreeStep]);

impl Serialize for Steps<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for step in self.0 {
            seq.serialize_element(&StepJson {
                op: operation_name(step.operation),
                args: Values(&step.arguments),
            })?;
        }
        seq.end()
    }
}

#[derive(Serialize)]
struct StepJson<'a> {
    op: &'static str,
    args: Values<'a>,
}

/// The name an action operation has in `op`.
fn operation_name(operation: ActionOperation) -> &'static str {
    match operation {
        ActionOperation::Set => "set",
        ActionOperation::Reset => "reset",
        ActionOperation::Refresh => "refresh",
        ActionOperation::LoadMore => "load_more",
        ActionOperation::Compose => "compose",
        ActionOperation::Open => "open",
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// A tree decoded from tree JSON.
pub(crate) struct TreeDe(pub(crate) Tree);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoot {
    m: String,
    k: Vec<NodeDe>,
}

impl<'de> Deserialize<'de> for TreeDe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let root = RawRoot::deserialize(deserializer)?;
        if root.m != "root" {
            return Err(de::Error::custom(
                "the tree's root must have \"m\": \"root\"",
            ));
        }
        Ok(Self(Tree {
            blocks: root.k.into_iter().map(|node| node.0).collect(),
        }))
    }
}

/// Deserializes a member that is present. With `#[serde(default)]` an
/// absent member is `None`, while an explicit `null` is decoded as `T`
/// (an error for every member type except a prop value).
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// Every member any node may have; which ones a node has decides its kind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNode {
    #[serde(default, deserialize_with = "present")]
    c: Option<String>,
    #[serde(default, deserialize_with = "present")]
    p: Option<FieldsDe>,
    #[serde(default, deserialize_with = "present")]
    k: Option<Vec<NodeDe>>,
    #[serde(default, deserialize_with = "present")]
    t: Option<String>,
    #[serde(default, deserialize_with = "present")]
    r: Option<String>,
    #[serde(default, deserialize_with = "present")]
    e: Option<DiagnosticCode>,
    #[serde(default, deserialize_with = "present")]
    m: Option<String>,
    #[serde(default, deserialize_with = "present")]
    level: Option<u8>,
    #[serde(default, deserialize_with = "present")]
    lang: Option<String>,
    #[serde(default, deserialize_with = "present")]
    text: Option<String>,
    #[serde(default, deserialize_with = "present")]
    ordered: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    start: Option<u64>,
    #[serde(default, deserialize_with = "present")]
    checked: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    href: Option<String>,
    #[serde(default, deserialize_with = "present")]
    title: Option<String>,
    #[serde(default, deserialize_with = "present")]
    image: Option<bool>,
}

impl RawNode {
    /// The names of the members this node has.
    fn members(&self) -> BTreeSet<&'static str> {
        [
            ("c", self.c.is_some()),
            ("p", self.p.is_some()),
            ("k", self.k.is_some()),
            ("t", self.t.is_some()),
            ("r", self.r.is_some()),
            ("e", self.e.is_some()),
            ("m", self.m.is_some()),
            ("level", self.level.is_some()),
            ("lang", self.lang.is_some()),
            ("text", self.text.is_some()),
            ("ordered", self.ordered.is_some()),
            ("start", self.start.is_some()),
            ("checked", self.checked.is_some()),
            ("href", self.href.is_some()),
            ("title", self.title.is_some()),
            ("image", self.image.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, has)| has.then_some(name))
        .collect()
    }

    fn into_node(self) -> Result<TreeNode, String> {
        let members = self.members();
        let exactly = |expected: &[&str]| -> Result<(), String> {
            if members == expected.iter().copied().collect::<BTreeSet<_>>() {
                Ok(())
            } else {
                Err(format!(
                    "a node with members {members:?} must have exactly {expected:?}"
                ))
            }
        };
        let text = |text: Option<String>| DisplayText::new(text.as_deref().unwrap_or_default());
        let children = |k: Option<Vec<NodeDe>>| -> Vec<TreeNode> {
            k.unwrap_or_default()
                .into_iter()
                .map(|node| node.0)
                .collect()
        };
        if let Some(kind) = self.m.as_deref() {
            let kind = match kind {
                "p" => plain(&exactly, MarkdownKind::Paragraph)?,
                "h" => {
                    exactly(&["m", "level", "k"])?;
                    MarkdownKind::Heading(self.level.unwrap_or_default())
                }
                "hr" => plain(&exactly, MarkdownKind::ThematicBreak)?,
                "codeblock" => {
                    exactly(&["m", "lang", "text", "k"])?;
                    MarkdownKind::CodeBlock {
                        language: text(self.lang),
                        text: text(self.text),
                    }
                }
                "code" => {
                    exactly(&["m", "text", "k"])?;
                    MarkdownKind::Code(text(self.text))
                }
                "quote" => plain(&exactly, MarkdownKind::Quote)?,
                "list" => {
                    if self.ordered == Some(true) {
                        exactly(&["m", "ordered", "start", "k"])?;
                    } else {
                        exactly(&["m", "ordered", "k"])?;
                    }
                    MarkdownKind::List { start: self.start }
                }
                "item" => plain(&exactly, MarkdownKind::Item)?,
                "task" => {
                    exactly(&["m", "checked", "k"])?;
                    MarkdownKind::Task(self.checked.unwrap_or_default())
                }
                "table" => plain(&exactly, MarkdownKind::Table)?,
                "thead" => plain(&exactly, MarkdownKind::TableHead)?,
                "row" => plain(&exactly, MarkdownKind::TableRow)?,
                "cell" => plain(&exactly, MarkdownKind::TableCell)?,
                "em" => plain(&exactly, MarkdownKind::Emphasis)?,
                "strong" => plain(&exactly, MarkdownKind::Strong)?,
                "del" => plain(&exactly, MarkdownKind::Strikethrough)?,
                "link" => {
                    exactly(&["m", "href", "title", "image", "k"])?;
                    MarkdownKind::Link {
                        destination: text(self.href),
                        title: text(self.title),
                        image: self.image.unwrap_or_default(),
                    }
                }
                "softbreak" => plain(&exactly, MarkdownKind::SoftBreak)?,
                "br" => plain(&exactly, MarkdownKind::HardBreak)?,
                other => return Err(format!("unknown Markdown kind {other:?}")),
            };
            return Ok(TreeNode::Markdown {
                kind,
                children: children(self.k),
            });
        }
        if self.c.is_some() && self.e.is_some() {
            exactly(&["c", "e"])?;
            return Ok(TreeNode::ErrorCell {
                name: text(self.c),
                code: self.e.unwrap_or(DiagnosticCode::Type),
            });
        }
        if self.c.is_some() {
            exactly(&["c", "p", "k"])?;
            return Ok(TreeNode::Component {
                name: text(self.c),
                props: self.p.map(|fields| fields.0).unwrap_or_default(),
                children: children(self.k),
            });
        }
        if let Some(code) = self.e {
            exactly(&["e"])?;
            return Ok(TreeNode::ErrorMarker(code));
        }
        if self.t.is_some() {
            exactly(&["t"])?;
            return Ok(TreeNode::Text(text(self.t)));
        }
        if self.r.is_some() {
            exactly(&["r"])?;
            return Ok(TreeNode::Reference(text(self.r)));
        }
        Err(format!("a node with members {members:?} has no kind"))
    }
}

/// A Markdown kind with no attributes: only `m` and `k`.
fn plain(
    exactly: &impl Fn(&[&str]) -> Result<(), String>,
    kind: MarkdownKind,
) -> Result<MarkdownKind, String> {
    exactly(&["m", "k"]).map(|()| kind)
}

/// One node decoded from tree JSON.
pub(crate) struct NodeDe(pub(crate) TreeNode);

impl<'de> Deserialize<'de> for NodeDe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        RawNode::deserialize(deserializer)?
            .into_node()
            .map(Self)
            .map_err(de::Error::custom)
    }
}

/// An ordered JSON object of prop values with unique keys.
struct FieldsDe(Vec<(DisplayText, TreeValue)>);

impl<'de> Deserialize<'de> for FieldsDe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FieldsVisitor;
        impl<'de> Visitor<'de> for FieldsVisitor {
            type Value = FieldsDe;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object of prop values")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<FieldsDe, A::Error> {
                let mut seen = BTreeSet::new();
                let mut fields = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom(format!("duplicate key {key:?}")));
                    }
                    let value = map.next_value::<ValueDe>()?;
                    fields.push((DisplayText::new(&key), value.0));
                }
                Ok(FieldsDe(fields))
            }
        }
        deserializer.deserialize_map(FieldsVisitor)
    }
}

/// A prop value decoded from tree JSON.
struct ValueDe(TreeValue);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshot {
    record_id: String,
    state: String,
    #[serde(default, deserialize_with = "present")]
    value: Option<ValueDe>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSite {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBinding {
    state: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    op: OperationName,
    args: Vec<ValueDe>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationName {
    Set,
    Reset,
    Refresh,
    LoadMore,
    Compose,
    Open,
}

/// The state types a binding can name: exactly the names the evaluator
/// writes into `TreeValue::Binding::kind`.
const BINDING_TYPES: [&str; 6] = ["number", "string", "boolean", "list", "record", "scalar"];

impl<'de> Deserialize<'de> for ValueDe {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor).map(Self)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = TreeValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a tree prop value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<TreeValue, E> {
        Ok(TreeValue::Null)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<TreeValue, E> {
        Ok(TreeValue::Boolean(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<TreeValue, E> {
        Ok(TreeValue::String(DisplayText::new(value)))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<TreeValue, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element::<ValueDe>()? {
            values.push(value.0);
        }
        Ok(TreeValue::List(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<TreeValue, A::Error> {
        let Some(tag) = map.next_key::<String>()? else {
            return Err(de::Error::custom("a tagged prop value needs one member"));
        };
        let value = match tag.as_str() {
            "number" => {
                let text = map.next_value::<String>()?;
                TreeValue::Number(
                    decimal::parse(&text)
                        .ok_or_else(|| de::Error::custom("not a canonical decimal"))?,
                )
            }
            "record" => TreeValue::Record(map.next_value::<FieldsDe>()?.0),
            "node" => TreeValue::Node(Box::new(map.next_value::<NodeDe>()?.0)),
            "action" => TreeValue::Action(
                map.next_value::<Vec<RawStep>>()?
                    .into_iter()
                    .map(|step| TreeStep {
                        operation: match step.op {
                            OperationName::Set => ActionOperation::Set,
                            OperationName::Reset => ActionOperation::Reset,
                            OperationName::Refresh => ActionOperation::Refresh,
                            OperationName::LoadMore => ActionOperation::LoadMore,
                            OperationName::Compose => ActionOperation::Compose,
                            OperationName::Open => ActionOperation::Open,
                        },
                        arguments: step.args.into_iter().map(|value| value.0).collect(),
                    })
                    .collect(),
            ),
            "snapshot" => {
                let snapshot = map.next_value::<RawSnapshot>()?;
                TreeValue::Snapshot {
                    record_id: DisplayText::new(&snapshot.record_id),
                    state: DisplayText::new(&snapshot.state),
                    value: snapshot.value.map(|value| Box::new(value.0)),
                }
            }
            "handleRecord" => TreeValue::Handle(DisplayText::new(&map.next_value::<String>()?)),
            "site" => TreeValue::Site(DisplayText::new(&map.next_value::<RawSite>()?.name)),
            "binding" => {
                let binding = map.next_value::<RawBinding>()?;
                let state = binding
                    .state
                    .strip_prefix('$')
                    .ok_or_else(|| de::Error::custom("a binding's state starts with $"))?;
                let kind = BINDING_TYPES
                    .into_iter()
                    .find(|kind| *kind == binding.kind)
                    .ok_or_else(|| de::Error::custom("unknown binding type"))?;
                TreeValue::Binding {
                    state: DisplayText::new(state),
                    kind,
                }
            }
            "error" => TreeValue::Error(map.next_value::<DiagnosticCode>()?),
            other => {
                return Err(de::Error::custom(format!(
                    "unknown prop value kind {other:?}"
                )))
            }
        };
        if map.next_key::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(
                "a tagged prop value has exactly one member",
            ));
        }
        Ok(value)
    }
}

/// The node `path` names in `tree`: the first index picks one of the root's
/// blocks and each later index one of that node's children.
pub(crate) fn node_at<'a>(tree: &'a Tree, path: &[usize]) -> Option<&'a TreeNode> {
    let (first, rest) = path.split_first()?;
    let mut node = tree.blocks.get(*first)?;
    for index in rest {
        let children = match node {
            TreeNode::Markdown { children, .. } | TreeNode::Component { children, .. } => children,
            _ => return None,
        };
        node = children.get(*index)?;
    }
    Some(node)
}

#[cfg(test)]
mod tests;
