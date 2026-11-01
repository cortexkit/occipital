//! Immutable values with creation-time logical sizes (§4.1, §9.3).
//!
//! `Arc`, rather than thread-local reference counting, lets a host measure a
//! revision on another thread without copying its shared value graph.

mod codec;
mod compare;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::number::Decimal;

pub use codec::{CodecError, CodecMode};
pub use compare::{Relation, ValueFailure};

/// A scalar sequence; neither construction nor comparison normalizes it (§4.3).
#[derive(Clone, Debug)]
pub struct ScalarString {
    text: Arc<str>,
    scalars: u64,
}

impl ScalarString {
    /// A string from text, counting its scalars once.
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        let text = text.into();
        let scalars = text.chars().count() as u64;
        Self { text, scalars }
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Constant-time scalar length, not UTF-8 bytes or grapheme clusters.
    pub fn len(&self) -> u64 {
        self.scalars
    }

    /// Whether the string has no scalars.
    pub fn is_empty(&self) -> bool {
        self.scalars == 0
    }
}

/// Source positions are one-based scalar coordinates, with an exclusive end (§12.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSpan {
    /// First line, from 1.
    pub line: u32,
    /// First column, from 1, in scalars.
    pub column: u32,
    /// Last line, from 1.
    pub end_line: u32,
    /// Column just after the end, from 1, in scalars.
    pub end_column: u32,
}

impl SourceSpan {
    pub(crate) fn is_valid(self) -> bool {
        self.line > 0
            && self.column > 0
            && self.end_column > 0
            && (self.end_line, self.end_column) >= (self.line, self.column)
    }
}

/// A local value error, distinct from a fuel or size abort (§4.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueError {
    /// The §12.4 code, such as `E_TYPE`.
    pub code: Arc<str>,
    /// Where in the source the error arose.
    pub span: SourceSpan,
}

impl ValueError {
    /// An error value with this code and position.
    pub fn new(code: impl Into<Arc<str>>, span: SourceSpan) -> Self {
        Self {
            code: code.into(),
            span,
        }
    }

    pub(crate) fn type_error(span: SourceSpan) -> Self {
        Self::new("E_TYPE", span)
    }
}

/// Construction cannot wrap a logical size or silently replace a record field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueBuildError {
    /// A logical size would overflow.
    SizeOverflow,
    /// A record names this key twice.
    DuplicateKey(String),
}

/// Host values contribute no evaluation allocation (§9.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Created by evaluation, charged to the evaluation account.
    Evaluation,
    /// Supplied by the host, such as answers and restored state.
    Host,
}

/// The thirteen distinct kinds in §4.1. Null and missing remain distinguishable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    /// A decimal number.
    Number,
    /// A string.
    String,
    /// A boolean.
    Boolean,
    /// `null`.
    Null,
    /// A missing value.
    Missing,
    /// A local error.
    Error,
    /// A list.
    List,
    /// A record.
    Record,
    /// A component.
    Component,
    /// An action.
    Action,
    /// A snapshot.
    Snapshot,
    /// A handle.
    Handle,
    /// A site envelope.
    SiteEnvelope,
}

/// Insertion-ordered fields; construction omits missing, but preserves null (§4.5).
#[derive(Clone, Debug)]
pub struct Record {
    fields: Arc<[(ScalarString, Value)]>,
    longest_key: u64,
}

impl Record {
    /// The fields in insertion order.
    pub fn fields(&self) -> &[(ScalarString, Value)] {
        &self.fields
    }

    /// The field named `key`, or missing.
    pub fn get(&self, key: &str) -> Value {
        self.fields
            .iter()
            .find(|(name, _)| name.as_str() == key)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(Value::missing)
    }

    /// The keys in insertion order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.fields.iter().map(|(key, _)| key.as_str())
    }

    /// The values in insertion order.
    pub fn values(&self) -> impl ExactSizeIterator<Item = &Value> {
        self.fields.iter().map(|(_, value)| value)
    }

    /// The record `Value::record` would hold, with its logical size,
    /// shallow size and depth.
    fn build(
        fields: Vec<(ScalarString, Value)>,
    ) -> Result<(Record, u64, u64, u32), ValueBuildError> {
        let mut names = std::collections::BTreeSet::new();
        let mut logical = 16;
        let mut longest_key = 0;
        for (key, value) in &fields {
            if !names.insert(key.as_str()) {
                return Err(ValueBuildError::DuplicateKey(key.as_str().into()));
            }
            if value.value_type() != ValueType::Missing {
                logical = add(
                    logical,
                    add(slots(16, key.len(), 4)?, value.logical_size())?,
                )?;
                longest_key = longest_key.max(key.len());
            }
        }
        let fields: Vec<_> = fields
            .into_iter()
            .filter(|(_, value)| value.value_type() != ValueType::Missing)
            .collect();
        let shallow = slots(16, fields.len() as u64, 16)?;
        let depth = nesting(fields.iter().map(|(_, value)| value));
        let record = Record {
            fields: fields.into(),
            longest_key,
        };
        Ok((record, logical, shallow, depth))
    }

    /// The record `Value::record` would build from these fields, failing the
    /// same way, for callers that need the record itself (component props).
    pub(crate) fn new(fields: Vec<(ScalarString, Value)>) -> Result<Record, ValueBuildError> {
        Record::build(fields).map(|(record, ..)| record)
    }

    /// The members of a host-owned wrapper, a snapshot or a site envelope,
    /// from member names the crate fixes itself. It cannot fail: a wrapper's
    /// size does not include its members, so no size is computed, and the
    /// crate never repeats a member name. Were one repeated, the first value
    /// would be kept, the one `get` returns. Missing values are omitted, as
    /// in every record.
    pub(crate) fn members(fields: Vec<(ScalarString, Value)>) -> Record {
        let mut names = std::collections::BTreeSet::new();
        let mut longest_key = 0;
        let mut kept = Vec::with_capacity(fields.len());
        for (key, value) in fields {
            if value.value_type() == ValueType::Missing || names.contains(key.as_str()) {
                continue;
            }
            names.insert(key.as_str().to_owned());
            longest_key = longest_key.max(key.len());
            kept.push((key, value));
        }
        Record {
            fields: kept.into(),
            longest_key,
        }
    }
}

/// A component waiting to be placed; its immutable children may share structure.
#[derive(Clone, Debug)]
pub struct Component {
    /// The catalog component name.
    pub name: Arc<str>,
    /// The props given.
    pub props: Record,
    /// Props written as a bare `$state` for a `binding<T>` prop (§4.10), as
    /// (prop name, state name) pairs in source order. A binding names the
    /// state a control writes; it is not the state's current value, so it is
    /// kept apart from ordinary prop values that data could imitate.
    pub bindings: Arc<[(Arc<str>, Arc<str>)]>,
    /// The children, in order.
    pub children: Arc<[Value]>,
}

/// Only read-only v1 action operations are representable (§7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionOperation {
    /// `Set($state, value)`.
    Set,
    /// `Reset($state)`.
    Reset,
    /// `Refresh(site)`.
    Refresh,
    /// `LoadMore(site)`.
    LoadMore,
    /// `Compose(text)`.
    Compose,
    /// `Open(target)`.
    Open,
}

/// Arguments are frozen values, not executable expressions (§7.1).
#[derive(Clone, Debug)]
pub struct ActionStep {
    /// What the step does.
    pub operation: ActionOperation,
    /// The frozen arguments.
    pub arguments: Arc<[Value]>,
}

#[derive(Clone, Debug)]
/// An action value: steps run in order when the control is activated (§7.1).
pub struct Action {
    /// The steps.
    pub steps: Arc<[ActionStep]>,
}

/// A reference to an immutable host-owned snapshot and its readable members (§4.8).
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The snapshot's record id.
    pub record_id: Arc<str>,
    /// `state`, `text`, `value`, `rows` and the other readable members.
    pub members: Record,
}

/// A scoped resource ID and version, or a scoped record ID; never a general
/// filesystem path that a renderer could open without authorization (§4.9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandleIdentity {
    /// A resource the broker resolved at store time.
    Resolved {
        /// `artifact` or `file`.
        kind: Arc<str>,
        /// The scoped resource id.
        scoped_resource_id: Arc<str>,
        /// The resource version.
        version: Arc<str>,
    },
    /// A record the broker did not resolve.
    Unresolved {
        /// The scoped record id.
        scoped_record_id: Arc<str>,
    },
}

#[derive(Clone, Debug)]
/// An `Artifact(…)` or `File(…)` handle value (§4.9).
pub struct Handle {
    /// The handle record's id.
    pub record_id: Arc<str>,
    /// What the record names.
    pub identity: HandleIdentity,
}

/// A host-owned site read view; its contents do not expand the wrapper's size.
#[derive(Clone, Debug)]
pub struct SiteEnvelope {
    /// The site's name.
    pub name: Arc<str>,
    /// `status`, `value`, `stale` and the other envelope members.
    pub members: Record,
}

/// Read-only inspection of a value. Size metadata cannot be invalidated by mutation.
#[derive(Clone, Debug)]
pub enum ValueKind {
    /// A decimal number.
    Number(Decimal),
    /// A string.
    String(ScalarString),
    /// A boolean.
    Boolean(bool),
    /// `null`.
    Null,
    /// A missing value.
    Missing,
    /// A local error.
    Error(ValueError),
    /// A list.
    List(Arc<[Value]>),
    /// A record.
    Record(Record),
    /// A component.
    Component(Arc<Component>),
    /// An action.
    Action(Arc<Action>),
    /// A snapshot.
    Snapshot(Arc<Snapshot>),
    /// A handle.
    Handle(Arc<Handle>),
    /// A site envelope.
    SiteEnvelope(Arc<SiteEnvelope>),
}

#[derive(Clone, Debug)]
/// A language value (§4.1). Values are immutable, share structure through
/// `Arc`, and record their logical size and depth when created (§9.3).
pub struct Value {
    kind: ValueKind,
    logical_size: u64,
    shallow_size: u64,
    depth: u32,
    /// Prepared descendants share ownership of their parse's retained charge.
    retention: Option<Arc<crate::accounts::Retention>>,
}

/// An envelope containing revision metadata and an immutable root value.
#[derive(Clone, Debug)]
pub struct Revision {
    /// The instance epoch.
    pub instance_epoch: u64,
    /// The revision number within the epoch.
    pub revision: u64,
    /// The root value.
    pub root: Value,
}

fn add(left: u64, right: u64) -> Result<u64, ValueBuildError> {
    left.checked_add(right).ok_or(ValueBuildError::SizeOverflow)
}

fn slots(base: u64, count: u64, width: u64) -> Result<u64, ValueBuildError> {
    add(
        base,
        count
            .checked_mul(width)
            .ok_or(ValueBuildError::SizeOverflow)?,
    )
}

fn sum_sizes<'a>(
    base: u64,
    values: impl IntoIterator<Item = &'a Value>,
) -> Result<u64, ValueBuildError> {
    values
        .into_iter()
        .try_fold(base, |size, value| add(size, value.logical_size()))
}

/// One more than the deepest part, so a container of atoms is one level deep.
fn nesting<'a>(values: impl IntoIterator<Item = &'a Value>) -> u32 {
    values
        .into_iter()
        .map(Value::depth)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

impl Value {
    pub(crate) fn attach_retention(&mut self, retention: &Arc<crate::accounts::Retention>) {
        self.retention = Some(retention.clone());
        match &mut self.kind {
            ValueKind::List(values) => {
                for value in Arc::make_mut(values) {
                    value.attach_retention(retention);
                }
            }
            ValueKind::Record(record) => {
                for (_, value) in Arc::make_mut(&mut record.fields) {
                    value.attach_retention(retention);
                }
            }
            _ => {}
        }
    }

    fn sized(kind: ValueKind, logical_size: u64, shallow_size: u64) -> Self {
        Self {
            kind,
            logical_size,
            shallow_size,
            depth: 0,
            retention: None,
        }
    }

    fn nested(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    fn atom(kind: ValueKind) -> Self {
        // Inline atoms have a 16-byte shallow tariff, matching their §9.3 header.
        Self::sized(kind, 16, 16)
    }

    /// A number.
    pub fn number(value: Decimal) -> Self {
        Self::atom(ValueKind::Number(value))
    }
    /// A boolean.
    pub fn boolean(value: bool) -> Self {
        Self::atom(ValueKind::Boolean(value))
    }
    /// `null`.
    pub fn null() -> Self {
        Self::atom(ValueKind::Null)
    }
    /// A missing value.
    pub fn missing() -> Self {
        Self::atom(ValueKind::Missing)
    }
    /// A local error value.
    pub fn error(error: ValueError) -> Self {
        Self::atom(ValueKind::Error(error))
    }

    /// A string; fails only when its size overflows.
    pub fn string(text: impl Into<Arc<str>>) -> Result<Self, ValueBuildError> {
        let text = ScalarString::new(text);
        let size = slots(16, text.len(), 4)?;
        Ok(Self::sized(ValueKind::String(text), size, size))
    }

    /// A string the crate builds itself, such as a fixed status word or a
    /// host string already inside the data caps. It cannot fail: the size
    /// saturates instead of overflowing. No string that fits in memory comes
    /// near that, and a saturated size would fail every value-size cap.
    pub(crate) fn text(text: impl Into<Arc<str>>) -> Self {
        let text = ScalarString::new(text);
        let size = 16_u64.saturating_add(text.len().saturating_mul(4));
        Self::sized(ValueKind::String(text), size, size)
    }

    /// A list of these values.
    pub fn list(values: Vec<Value>) -> Result<Self, ValueBuildError> {
        let logical = sum_sizes(16, &values)?;
        let shallow = slots(16, values.len() as u64, 8)?;
        let depth = nesting(&values);
        Ok(Self::sized(ValueKind::List(values.into()), logical, shallow).nested(depth))
    }

    /// A list the crate builds itself, such as an empty site value or a
    /// buffer whose size it checks afterwards. It cannot fail: sizes
    /// saturate instead of overflowing, and a saturated size would fail
    /// every value-size cap.
    pub(crate) fn items(values: Vec<Value>) -> Self {
        let logical = values.iter().fold(16_u64, |size, value| {
            size.saturating_add(value.logical_size())
        });
        let shallow = 16_u64.saturating_add((values.len() as u64).saturating_mul(8));
        let depth = nesting(&values);
        Self::sized(ValueKind::List(values.into()), logical, shallow).nested(depth)
    }

    /// A record with these fields in order; a repeated key fails.
    pub fn record(fields: Vec<(ScalarString, Value)>) -> Result<Self, ValueBuildError> {
        let (record, logical, shallow, depth) = Record::build(fields)?;
        Ok(Self::sized(ValueKind::Record(record), logical, shallow).nested(depth))
    }

    /// A component with props and children and no bindings.
    pub fn component(
        name: impl Into<Arc<str>>,
        props: Record,
        children: Vec<Value>,
    ) -> Result<Self, ValueBuildError> {
        Self::bound_component(name, props, Vec::new(), children)
    }

    /// A component with `binding<T>` props. Each binding occupies a prop slot
    /// and counts 16 logical bytes, like the other host-owned references.
    pub fn bound_component(
        name: impl Into<Arc<str>>,
        props: Record,
        bindings: Vec<(Arc<str>, Arc<str>)>,
        children: Vec<Value>,
    ) -> Result<Self, ValueBuildError> {
        // §9.3 counts each prop value and each child at every occurrence. Text
        // children use the ordinary string size (16 + 4L), with no extra wrapper.
        let logical = add(
            sum_sizes(sum_sizes(32, props.values())?, &children)?,
            slots(0, bindings.len() as u64, 16)?,
        )?;
        let shallow = slots(
            32,
            add(
                add(props.fields.len() as u64, bindings.len() as u64)?,
                children.len() as u64,
            )?,
            8,
        )?;
        let depth = nesting(props.values().chain(&children));
        Ok(Self::sized(
            ValueKind::Component(Arc::new(Component {
                name: name.into(),
                props,
                bindings: bindings.into(),
                children: children.into(),
            })),
            logical,
            shallow,
        )
        .nested(depth))
    }

    /// An action with these steps.
    pub fn action(steps: Vec<ActionStep>) -> Result<Self, ValueBuildError> {
        let arguments = steps.iter().flat_map(|step| step.arguments.iter());
        let logical = sum_sizes(16, arguments)?;
        let depth = nesting(steps.iter().flat_map(|step| step.arguments.iter()));
        let count = steps
            .iter()
            .try_fold(0, |n, step| add(n, step.arguments.len() as u64))?;
        // Frozen arguments use 8-byte shallow reference slots. Their own creation
        // allocations are charged separately, never recursively charged here.
        let shallow = slots(16, count, 8)?;
        Ok(Self::sized(
            ValueKind::Action(Arc::new(Action {
                steps: steps.into(),
            })),
            logical,
            shallow,
        )
        .nested(depth))
    }

    /// A snapshot value.
    pub fn snapshot(value: Snapshot) -> Self {
        Self::sized(ValueKind::Snapshot(Arc::new(value)), 16, 0)
    }
    /// A handle value.
    pub fn handle(value: Handle) -> Self {
        Self::sized(ValueKind::Handle(Arc::new(value)), 16, 0)
    }
    /// A site envelope value.
    pub fn site_envelope(value: SiteEnvelope) -> Self {
        Self::sized(ValueKind::SiteEnvelope(Arc::new(value)), 16, 0)
    }

    /// The value's kind and contents.
    pub fn kind(&self) -> &ValueKind {
        &self.kind
    }
    /// The logical size recorded at creation (§9.3).
    pub fn logical_size(&self) -> u64 {
        self.logical_size
    }

    /// Value nesting (§4.7, §9.4): 0 for atoms and host-owned wrappers, one
    /// more than the deepest element, field, prop, child or step argument for
    /// a container. It is recorded at creation, like the logical size.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The shallow contribution of this creation, not retained allocator bytes.
    /// Cloning a shared value is not a new value creation. Host-supplied values,
    /// including answers and restored state, contribute zero (§9.3).
    pub fn allocation_size(&self, origin: Origin) -> u64 {
        match origin {
            Origin::Evaluation => self.shallow_size,
            Origin::Host => 0,
        }
    }

    /// The value's kind, without contents.
    pub fn value_type(&self) -> ValueType {
        match &self.kind {
            ValueKind::Number(_) => ValueType::Number,
            ValueKind::String(_) => ValueType::String,
            ValueKind::Boolean(_) => ValueType::Boolean,
            ValueKind::Null => ValueType::Null,
            ValueKind::Missing => ValueType::Missing,
            ValueKind::Error(_) => ValueType::Error,
            ValueKind::List(_) => ValueType::List,
            ValueKind::Record(_) => ValueType::Record,
            ValueKind::Component(_) => ValueType::Component,
            ValueKind::Action(_) => ValueType::Action,
            ValueKind::Snapshot(_) => ValueType::Snapshot,
            ValueKind::Handle(_) => ValueType::Handle,
            ValueKind::SiteEnvelope(_) => ValueType::SiteEnvelope,
        }
    }

    /// Whether the value is `null` or missing.
    pub fn is_absent(&self) -> bool {
        matches!(self.kind, ValueKind::Null | ValueKind::Missing)
    }

    /// Looks up record fields only. This is not full expression member access:
    /// list-wide field extraction and host-view members need separate dispatch.
    pub fn record_field(&self, key: &str) -> Self {
        match &self.kind {
            ValueKind::Record(record) => record.get(key),
            _ => Self::missing(),
        }
    }

    /// The list lookup primitive; an out-of-range index is missing (§4.5).
    pub fn list_element(&self, index: usize) -> Self {
        match &self.kind {
            ValueKind::List(values) => values.get(index).cloned().unwrap_or_else(Self::missing),
            _ => Self::missing(),
        }
    }

    /// Typed conditions do not treat zero, empty strings or lists as false (§4.4).
    pub fn condition(&self, span: SourceSpan) -> Result<bool, ValueError> {
        match &self.kind {
            ValueKind::Boolean(value) => Ok(*value),
            ValueKind::Null | ValueKind::Missing => Ok(false),
            ValueKind::Error(error) => Err(error.clone()),
            _ => Err(ValueError::type_error(span)),
        }
    }

    /// Text conversion (§4.11); production fuel and output reservation belong to
    /// the caller, which can preflight the result before allocating a value.
    pub fn to_text(&self, span: SourceSpan) -> Result<String, ValueError> {
        match &self.kind {
            ValueKind::String(text) => Ok(text.as_str().into()),
            ValueKind::Number(number) => Ok(number.to_string()),
            ValueKind::Boolean(value) => Ok(value.to_string()),
            ValueKind::Null | ValueKind::Missing => Ok(String::new()),
            ValueKind::Error(error) => Err(error.clone()),
            _ => Err(ValueError::type_error(span)),
        }
    }
}

#[cfg(test)]
mod tests;
