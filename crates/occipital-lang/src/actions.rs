//! Local, pure activation handling (§7). Evaluation only freezes actions;
//! the session installs controls here only when their revision commits.

use std::collections::{BTreeMap, BTreeSet};

use crate::eval::tree::{Tree, TreeNode, TreeValue};
use crate::{
    Action, ActionOperation, Address, CodecMode, Diagnostic, DiagnosticCode, DiagnosticPhase,
    DisplayText, Handle, Value, ValueKind, ValueType,
};

/// A debug path in one revision, including named props and list/child indices.
/// A path alone is never an activation identity (§7.3, §14.3).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ControlId(pub Vec<PathPart>);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// One step of a path from the tree root to a node or value (§7.3).
pub enum PathPart {
    /// The child at this index in `blocks` or `children`.
    Child(usize),
    /// The named prop of a component.
    Prop(String),
    /// The element at this index of a list.
    Index(usize),
    /// The named field of a record.
    Field(String),
}

/// Pair the evaluator's lossless actions with paths in the display tree.
/// Both walks use prop order before children; frozen arguments are not nodes
/// that the person can activate, and are deliberately not traversed here.
pub(crate) fn controls(tree: &Tree, actions: Vec<Action>) -> BTreeMap<ControlId, Action> {
    fn node(
        node: &TreeNode,
        path: &mut Vec<PathPart>,
        actions: &mut impl Iterator<Item = Action>,
        out: &mut BTreeMap<ControlId, Action>,
    ) {
        match node {
            TreeNode::Component {
                props, children, ..
            } => {
                let enabled = !props.iter().any(|(key, value)| {
                    key.as_str() == "disabled" && *value == TreeValue::Boolean(true)
                });
                for (key, value) in props {
                    path.push(PathPart::Prop(key.as_str().to_owned()));
                    prop(value, path, enabled, actions, out);
                    path.pop();
                }
                nodes(children, path, actions, out);
            }
            TreeNode::Markdown { children, .. } => nodes(children, path, actions, out),
            _ => {}
        }
    }
    fn nodes(
        children: &[TreeNode],
        path: &mut Vec<PathPart>,
        actions: &mut impl Iterator<Item = Action>,
        out: &mut BTreeMap<ControlId, Action>,
    ) {
        for (index, child) in children.iter().enumerate() {
            path.push(PathPart::Child(index));
            node(child, path, actions, out);
            path.pop();
        }
    }
    fn prop(
        value: &TreeValue,
        path: &mut Vec<PathPart>,
        enabled: bool,
        actions: &mut impl Iterator<Item = Action>,
        out: &mut BTreeMap<ControlId, Action>,
    ) {
        match value {
            TreeValue::Action(_) => {
                // Evaluation freezes one action per action prop, in this
                // walk's order. Were one missing, the control would simply
                // not be installed.
                if let Some(action) = actions.next() {
                    if enabled {
                        out.insert(ControlId(path.clone()), action);
                    }
                }
            }
            TreeValue::Node(child) => node(child, path, actions, out),
            TreeValue::List(items) => {
                for (index, item) in items.iter().enumerate() {
                    path.push(PathPart::Index(index));
                    prop(item, path, enabled, actions, out);
                    path.pop();
                }
            }
            TreeValue::Record(fields) => {
                for (key, value) in fields {
                    path.push(PathPart::Field(key.as_str().to_owned()));
                    prop(value, path, enabled, actions, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    let mut actions = actions.into_iter();
    nodes(&tree.blocks, &mut Vec::new(), &mut actions, &mut out);
    assert!(
        actions.next().is_none(),
        "all frozen actions have tree paths"
    );
    out
}

pub(crate) fn address_text(action: &Action) -> Vec<String> {
    action
        .steps
        .iter()
        .filter(|step| step.operation == ActionOperation::Open)
        .filter_map(|step| match step.arguments.first()?.kind() {
            ValueKind::String(text) => Some(
                Address::parse(text.as_str())
                    .ok()?
                    .display()
                    .as_str()
                    .to_owned(),
            ),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A person's tap on a control of a committed revision (§7.3).
pub struct Activation {
    /// The instance epoch the revision belongs to.
    pub instance_epoch: u64,
    /// The committed revision the control was drawn in.
    pub revision: u64,
    /// The control, by its path in that revision's tree.
    pub control: ControlId,
    /// A host-minted id for this tap; a repeated id is ignored.
    pub id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Why an input or activation was not applied (§7.3).
pub enum Rejected {
    /// It names a revision or epoch that is no longer current.
    Stale,
    /// The control is drawn disabled.
    Disabled,
    /// The same activation id was already applied.
    Repeated,
    /// An earlier activation's handoffs have not completed yet.
    Busy,
}

/// These are host handoffs, not broker writes or confirmations. The first tap
/// on an address asks the host to show its full escaped URL. Only a second
/// tap in that sheet may open a browser; this enum cannot open one (§7.5).
#[derive(Clone, Debug)]
pub enum Handoff {
    /// Append after the draft on a new line (or into an empty composer),
    /// focus it, and never send. Text is sanitized only for this handoff.
    Compose(DisplayText),
    /// Ask the capability broker to resolve the recorded handle using its
    /// producer's computer/session/scope authority, never chat links or the
    /// person's file reader.
    OpenHandle(Handle),
    /// Show the full escaped address in a sheet; opening is a second tap there.
    AddressSheet(Address),
}

/// Names of sites to refresh or page, not the request identities from when
/// the action was built. The session must resolve each name using the final
/// state after all local steps, before issuing any request (§7.3, §8.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SiteIntent {
    Refresh(String),
    LoadMore(String),
}

#[derive(Debug, Default)]
pub(crate) struct LocalResult {
    pub handoffs: Vec<Handoff>,
    pub intents: Vec<SiteIntent>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Owns only local activation state. Site scheduling and the single final
/// evaluation per event batch belong to the session, not this executor.
pub(crate) struct Runtime {
    epoch: u64,
    revision: Option<u64>,
    controls: BTreeMap<ControlId, Action>,
    initial: BTreeMap<String, Value>,
    pub state: BTreeMap<String, Value>,
    seen: BTreeSet<u64>,
    pending: BTreeMap<u64, ControlId>,
}

impl Runtime {
    pub(crate) fn new(epoch: u64, initial: BTreeMap<String, Value>) -> Self {
        Self {
            epoch,
            revision: None,
            controls: BTreeMap::new(),
            state: initial.clone(),
            initial,
            seen: BTreeSet::new(),
            pending: BTreeMap::new(),
        }
    }

    /// Called only after the session accepts this revision's measured tree
    /// height under the current layout key. Merely evaluating a candidate
    /// must never replace the controls from the visible, committed revision.
    pub(crate) fn commit(&mut self, revision: u64, controls: BTreeMap<ControlId, Action>) {
        // Remeasurement changes geometry, not the activation identity or handoffs.
        if self.revision == Some(revision) {
            return;
        }
        assert!(self.revision.is_none_or(|old| revision > old));
        self.revision = Some(revision);
        self.controls = controls;
        self.pending.clear();
    }

    pub(crate) fn activate(&mut self, event: &Activation) -> Result<LocalResult, Rejected> {
        if event.instance_epoch != self.epoch || Some(event.revision) != self.revision {
            return Err(Rejected::Stale);
        }
        let action = self
            .controls
            .get(&event.control)
            .ok_or(Rejected::Disabled)?
            .clone();
        if self.seen.contains(&event.id) {
            return Err(Rejected::Repeated);
        }
        if self
            .pending
            .values()
            .any(|control| control == &event.control)
        {
            return Err(Rejected::Busy);
        }
        self.seen.insert(event.id);
        let result = self.run(&action);
        if !result.handoffs.is_empty() {
            self.pending.insert(event.id, event.control.clone());
        }
        Ok(result)
    }

    /// All host handoffs for this activation finished. Read answers do not
    /// call this: requests are site intents, not action completion (§7.3).
    pub(crate) fn handoffs_complete(&mut self, event: &Activation) {
        if event.instance_epoch == self.epoch
            && Some(event.revision) == self.revision
            && self.pending.get(&event.id) == Some(&event.control)
        {
            self.pending.remove(&event.id);
        }
    }

    fn run(&mut self, action: &Action) -> LocalResult {
        let mut out = LocalResult::default();
        for step in action.steps.iter() {
            let result = match (step.operation, step.arguments.as_ref()) {
                (ActionOperation::Set, [state, value]) => self.assign(state, Some(value)),
                (ActionOperation::Reset, [state]) => self.assign(state, None),
                (ActionOperation::Refresh | ActionOperation::LoadMore, [target]) => {
                    if let ValueKind::SiteEnvelope(site) = target.kind() {
                        out.intents
                            .push(if step.operation == ActionOperation::Refresh {
                                SiteIntent::Refresh(site.name.to_string())
                            } else {
                                SiteIntent::LoadMore(site.name.to_string())
                            });
                        Ok(())
                    } else {
                        Err(DiagnosticCode::Type)
                    }
                }
                (ActionOperation::Compose, [text]) => match text.kind() {
                    ValueKind::String(text) => {
                        out.handoffs
                            .push(Handoff::Compose(DisplayText::new(text.as_str())));
                        Ok(())
                    }
                    _ => Err(DiagnosticCode::Type),
                },
                (ActionOperation::Open, [target]) => match target.kind() {
                    ValueKind::Handle(handle) => {
                        out.handoffs.push(Handoff::OpenHandle((**handle).clone()));
                        Ok(())
                    }
                    ValueKind::String(text) => Address::parse(text.as_str()).map(|address| {
                        out.handoffs.push(Handoff::AddressSheet(address));
                    }),
                    _ => Err(DiagnosticCode::Type),
                },
                _ => Err(DiagnosticCode::Arity),
            };
            if let Err(code) = result {
                out.diagnostics
                    .push(Diagnostic::new(code, DiagnosticPhase::Evaluate));
                break;
            }
        }
        out
    }

    pub(crate) fn assign(
        &mut self,
        state: &Value,
        value: Option<&Value>,
    ) -> Result<(), DiagnosticCode> {
        let ValueKind::String(state) = state.kind() else {
            return Err(DiagnosticCode::StateType);
        };
        let name = state.as_str();
        let initial = self.initial.get(name).ok_or(DiagnosticCode::StateType)?;
        let value = value.unwrap_or(initial);
        if !accepts(initial.value_type(), value) {
            return Err(DiagnosticCode::StateType);
        }
        let mut next = self.state.clone();
        next.insert(name.to_owned(), value.clone());
        let record = Value::record(
            next.iter()
                .map(|(key, value)| (crate::ScalarString::new(key.as_str()), value.clone()))
                .collect(),
        )
        .map_err(|_| DiagnosticCode::StateType)?;
        if record
            .encode_ordered(CodecMode::State)
            .map_err(|_| DiagnosticCode::StateType)?
            .len()
            > 65_536
        {
            return Err(DiagnosticCode::StateType);
        }
        self.state = next;
        Ok(())
    }
}

fn accepts(initial: ValueType, value: &Value) -> bool {
    fn data(value: &Value) -> bool {
        match value.kind() {
            ValueKind::Number(_)
            | ValueKind::String(_)
            | ValueKind::Boolean(_)
            | ValueKind::Null => true,
            ValueKind::List(items) => items.iter().all(data),
            ValueKind::Record(record) => record.values().all(data),
            _ => false,
        }
    }
    data(value)
        && match initial {
            ValueType::Null => matches!(
                value.value_type(),
                ValueType::Number | ValueType::String | ValueType::Boolean | ValueType::Null
            ),
            ValueType::Number | ValueType::String | ValueType::Boolean => {
                value.value_type() == initial || value.value_type() == ValueType::Null
            }
            ValueType::List | ValueType::Record => value.value_type() == initial,
            _ => false,
        }
}

#[cfg(test)]
#[path = "actions/tests.rs"]
mod tests;
