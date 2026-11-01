//! The event/phase boundary (§8.2). The evaluator resolves each demanded site
//! before using its envelope; only a successful final pass dispatches reads.

mod identity;
mod paged;
mod protocol;
mod query;
mod watch;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::actions::{Activation, ControlId, Handoff, PathPart, Rejected, Runtime, SiteIntent};
use crate::eval::tree::{Tree, TreeNode, TreeValue};
use crate::eval::{self, Inputs};
use crate::geometry::{
    self, Height, LayoutContext, LayoutKey, MeasureRevision, Measurement, Placement,
    ReservationOutcome, RowFrame,
};
use crate::prepare::resolve::Resolved;
use crate::{Action, Decimal, Diagnostic, DisplayText, Value};
pub(crate) use identity::Reads;
pub use identity::{Authority, Identity, ReadVersion};
use query::Queries;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// A request epoch within an instance epoch; a late answer for an old epoch
/// is ignored (§8.2).
pub struct Epoch {
    /// The instance epoch.
    pub instance: u64,
    /// The request number within it.
    pub request: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A timer the host arms and fires back as an event.
pub enum Timer {
    /// The five-second first-answer wait of the instance epoch (§8.2).
    FirstAnswer(u64),
    /// A request's answer timeout.
    Timeout(Epoch),
    /// A query's refresh interval (§8.3).
    Refresh(Box<Identity>, Epoch),
    /// The pacing timer for applying buffered watch items (§8.5).
    WatchApply(Epoch, u64),
    /// The grace period after the instance is hidden (§8.7).
    Hidden(Epoch),
    /// The grace period after the connection drops (§8.7).
    Offline(Epoch),
}

#[derive(Clone, Debug)]
/// Something the host must do, in the order listed (§14.2 effects table).
pub enum Effect {
    /// `query.issue`: make a read.
    Issue {
        /// The site.
        site: String,
        /// The full read identity.
        identity: Identity,
        /// The request epoch the answer must carry.
        epoch: Epoch,
    },
    /// `watch.subscribe`.
    Subscribe {
        /// The site.
        site: String,
        /// The full read identity.
        identity: Identity,
        /// The subscription epoch.
        epoch: Epoch,
        /// Resume after this item sequence, if any.
        after: Option<u64>,
    },
    /// `watch.credit`: allow more notices.
    Credit {
        /// The subscription epoch.
        epoch: Epoch,
        /// Notices allowed.
        notices: u32,
        /// Bytes allowed.
        bytes: u64,
    },
    /// `page.request`.
    Page {
        /// The site.
        site: String,
        /// The full read identity.
        identity: Identity,
        /// The request epoch.
        epoch: Epoch,
        /// The cursor, or none for the first page.
        cursor: Option<String>,
        /// The page size.
        limit: u16,
    },
    /// `query.cancel`, `watch.unsubscribe` or `page.cancel`.
    Cancel {
        /// The site.
        site: String,
        /// The read identity.
        identity: Identity,
        /// The epoch being cancelled.
        epoch: Epoch,
    },
    /// `timer.start`: arm the timer for this many seconds.
    Arm(Timer, Decimal),
    /// `timer.stop`.
    Stop(Timer),
    /// `compose` or `open`: a host handoff for an activation.
    Handoff(Activation, Handoff),
}

/// An event the host feeds to a program session (§8.2).
pub enum Event {
    /// The host mints a fresh instance epoch, even when restoring saved state.
    Load {
        /// A fresh instance epoch.
        epoch: u64,
        /// Host values prepared by `ProgramSession::load_event`.
        inputs: Box<Inputs>,
        /// Saved state by name, without `$`.
        restored: BTreeMap<String, Value>,
    },
    /// A control wrote a state through its binding (§7.6).
    Input {
        /// The instance epoch.
        instance: u64,
        /// The committed revision the control was drawn in.
        revision: u64,
        /// The control.
        control: ControlId,
        /// The state it writes, without `$`.
        binding: String,
        /// The new value.
        value: Value,
    },
    /// A control was tapped.
    Activate(Activation),
    /// The host finished an activation's handoffs.
    HandoffsComplete(Activation),
    /// The answer to a `query.issue`.
    Answer {
        /// The site.
        site: String,
        /// The identity from the effect.
        identity: Box<Identity>,
        /// The epoch from the effect.
        epoch: Epoch,
        /// The answer bytes, or an error message.
        answer: Result<Vec<u8>, String>,
    },
    /// An armed timer fired.
    Timer(Timer),
    /// The computer acknowledged a subscription.
    WatchAck {
        /// The identity from the effect.
        identity: Box<Identity>,
        /// The subscription epoch.
        epoch: Epoch,
        /// The last item sequence already delivered.
        after: u64,
    },
    /// The computer reserves count and byte credit before delivery (§8.5).
    /// Its monotonic token identifies that delivery reservation, not an item's
    /// sequence number, so redelivery cannot release the reservation twice.
    WatchNotice {
        /// The identity from the effect.
        identity: Box<Identity>,
        /// The subscription epoch.
        epoch: Epoch,
        /// The delivery reservation token.
        token: u64,
        /// The sequence number of the first item.
        first: u64,
        /// The notice bytes.
        bytes: Vec<u8>,
    },
    /// The subscription ended.
    WatchEnd {
        /// The identity from the effect.
        identity: Box<Identity>,
        /// The subscription epoch.
        epoch: Epoch,
        /// The error, or none for a clean end.
        error: Option<String>,
    },
    /// The answer to a `page.request`.
    PageAnswer {
        /// The identity from the effect.
        identity: Box<Identity>,
        /// The request epoch.
        epoch: Epoch,
        /// The cursor from the effect.
        cursor: Option<String>,
        /// The page bytes, or an error message.
        answer: Result<Vec<u8>, String>,
    },
    /// A paged list's load-more sentinel scrolled into view.
    AutoLoadMore {
        /// The instance epoch.
        instance: u64,
        /// The committed revision.
        revision: u64,
        /// The sentinel control.
        control: ControlId,
        /// A host-minted token; a repeated token is ignored.
        token: u64,
    },
    /// The instance became visible or hidden (§8.7).
    Visibility(bool),
    /// The app came to the foreground or left it.
    Foreground(bool),
    /// The connection came up or went down.
    Connection(bool),
    /// A new `now` and `zone`.
    Clock {
        /// Milliseconds since the Unix epoch.
        now: Decimal,
        /// The time zone name, as a string value.
        zone: Value,
    },
    /// The visibility/connection policy decides when reads may run (§8.7).
    Availability(bool),
    /// The layout context changed.
    Layout(LayoutContext),
    /// A measurement of the current candidate.
    MeasureComplete(Measurement),
}

#[derive(Clone, Debug)]
/// An evaluated revision waiting to be measured and committed.
pub struct Candidate {
    /// The instance epoch.
    pub instance_epoch: u64,
    /// The revision number.
    pub revision: u64,
    /// The layout context it was evaluated for.
    pub layout: LayoutContext,
    /// A digest of the state it was evaluated with.
    pub state_hash: String,
    /// The tree.
    pub tree: Arc<Tree>,
    retention: Option<Arc<crate::accounts::Retention>>,
    /// A separate, effect-free load evaluation, never a previously framed tree.
    pub first: Option<Arc<Candidate>>,
    /// Whether this is the first-answer loading root.
    pub loading: bool,
    controls: BTreeMap<ControlId, Action>,
    bindings: BTreeMap<ControlId, String>,
    sentinels: BTreeMap<ControlId, String>,
}

impl Candidate {
    /// The revision as input to [`measure`](crate::measure).
    pub fn measure_revision(&self) -> MeasureRevision<'_> {
        MeasureRevision {
            instance_epoch: self.instance_epoch,
            revision: self.revision,
            state_hash: &self.state_hash,
            tree: &self.tree,
            first: self.first.as_ref().map(|first| first.tree.as_ref()),
        }
    }

    /// The layout key this revision measures under in `context`.
    pub fn layout_key(&self, context: &LayoutContext) -> LayoutKey {
        geometry::layout_key(&self.measure_revision(), context)
    }

    /// The controls the person can activate in this revision, each named by
    /// its path in the tree (§7.3).
    pub fn controls(&self) -> impl Iterator<Item = &ControlId> {
        self.controls.keys()
    }

    /// Each input control and the state its binding writes (§7.6).
    pub fn bindings(&self) -> impl Iterator<Item = (&ControlId, &str)> {
        self.bindings
            .iter()
            .map(|(control, state)| (control, state.as_str()))
    }
}

#[derive(Clone, Debug)]
/// The revision on screen.
pub struct Committed {
    /// The committed candidate.
    pub candidate: Candidate,
    /// An aborted measurement does not supply geometry for the fallback.
    pub height: Option<Decimal>,
    /// The measured content height.
    pub content_height: Height,
    /// The row frame, for a `show_ui` row.
    pub row_frame: Option<RowFrame>,
    /// Whether this is the fallback after an abort (§9.6).
    pub fallback: bool,
}

/// Indices refer to `Output.effects`; commit precedes abort cancellations (§9.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputStep {
    /// The effect at this index of `effects`.
    Effect(usize),
    /// The commit of this revision.
    Commit(u64),
}

#[derive(Default, Debug)]
/// What one batch asks of the host (§8.2).
pub struct Output {
    /// Effects to perform, in order.
    pub effects: Vec<Effect>,
    /// A new revision to measure.
    pub candidate: Option<Candidate>,
    /// The defaults tree to measure for a `show_ui` row frame.
    pub defaults: Option<Arc<Candidate>>,
    /// A revision committed by this batch.
    pub committed: Option<Committed>,
    /// Runtime diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Inputs and activations that were not applied.
    pub rejected: Vec<Rejected>,
    /// How many evaluations ran.
    pub evaluations: u32,
    /// Fuel those evaluations used.
    pub fuel: u64,
    /// The abort, when the program stopped (§9.6).
    pub abort: Option<eval::Abort>,
    /// Effects and commits in the order they happened.
    pub order: Vec<OutputStep>,
}

impl Output {
    fn record_effects(&mut self) {
        let recorded = self
            .order
            .iter()
            .filter(|step| matches!(step, OutputStep::Effect(_)))
            .count();
        self.order
            .extend((recorded..self.effects.len()).map(OutputStep::Effect));
    }

    fn finish(mut self) -> Self {
        self.record_effects();
        self
    }
}

pub(crate) struct Session {
    program: Resolved,
    inputs: Inputs,
    runtime: Runtime,
    queries: Queries,
    layout: LayoutContext,
    fallback: DisplayText,
    defaults: Option<Arc<Candidate>>,
    revision: u64,
    candidate: Option<Candidate>,
    pub committed: Option<Committed>,
    loaded: bool,
    epochs: BTreeSet<u64>,
    first_wait: bool,
    aborted: bool,
    pending_abort: Option<eval::Abort>,
}

impl Session {
    /// The accounts this instance charges, so the host's data preparation
    /// for a load is charged to the same interface aggregate (§9.1).
    pub(crate) fn accounts(&self) -> &crate::accounts::Accounts {
        &self.queries.accounts
    }

    /// Supply the same host handles to every instance in a chat/app lifetime.
    pub fn with_accounts(
        mut self,
        chat: crate::SharedAccount,
        app: crate::SharedAccount,
    ) -> Result<Self, crate::accounts::ScopeError> {
        self.queries.accounts =
            crate::accounts::Accounts::new(crate::AccountScope::Interface, chat, app)?;
        Ok(self)
    }
    /// Retained site roots, including admitted but unapplied notice frames.
    /// Values expose logical sizes and allocation metadata; aggregate accounts
    /// must price these roots separately from per-site caps (§9.1).
    pub fn retained_buffers(&self) -> impl Iterator<Item = (&Identity, &'static str, &Value)> {
        self.queries.retained_buffers()
    }

    /// Both a pending measurement frame and the last committed frame remain
    /// retained until replaced. Charging only visible buffers would miss them.
    pub fn retained_frames(&self) -> impl Iterator<Item = &Candidate> {
        self.candidate
            .iter()
            .chain(self.committed.iter().map(|c| &c.candidate))
            .chain(self.defaults.iter().map(Arc::as_ref))
    }

    /// Fallback text is precomputed by the caller; the scheduler never scans source.
    pub fn new(
        program: Resolved,
        inputs: Inputs,
        reads: Reads,
        layout: LayoutContext,
        fallback: DisplayText,
    ) -> Self {
        let initial = eval::initial_state(&program);
        let order = program
            .declarations
            .iter()
            .enumerate()
            .map(|(i, d)| (d.name.clone(), i))
            .collect();
        Self {
            program,
            inputs,
            runtime: Runtime::new(0, initial),
            queries: Queries::new(reads, order, 0),
            layout,
            fallback,
            defaults: None,
            revision: 0,
            candidate: None,
            committed: None,
            loaded: false,
            epochs: BTreeSet::new(),
            first_wait: false,
            aborted: false,
            pending_abort: None,
        }
    }

    pub fn batch(&mut self, events: Vec<Event>) -> Output {
        let mut out = Output::default();
        let mut cycle = false;
        let mut refresh = BTreeSet::new();
        let mut more = BTreeSet::new();
        for event in events {
            // An abort freezes the state that caused it until a fresh load;
            // old controls and refresh timers cannot restart the program (§9.6).
            if self.aborted
                && !matches!(
                    &event,
                    Event::Load { .. } | Event::MeasureComplete(_) | Event::Layout(_)
                )
            {
                if matches!(&event, Event::Input { .. } | Event::Activate(_)) {
                    out.rejected.push(Rejected::Stale);
                }
                continue;
            }
            match event {
                Event::Load {
                    epoch,
                    inputs,
                    restored,
                } => {
                    if !self.epochs.insert(epoch) {
                        out.rejected.push(Rejected::Stale);
                        continue;
                    }
                    self.queries.cancel_all(&mut out.effects);
                    if self.first_wait {
                        out.effects
                            .push(Effect::Stop(Timer::FirstAnswer(self.queries.epoch)));
                    }
                    self.queries = self.queries.reload(epoch);
                    self.runtime = Runtime::new(epoch, eval::initial_state(&self.program));
                    for (name, value) in restored {
                        // Invalid entries are discarded independently: one bad saved
                        // field must not erase the person's other state (§8.9).
                        let _ = self.runtime.assign(&Value::text(name), Some(&value));
                    }
                    self.inputs = *inputs;
                    self.inputs.sites.clear();
                    self.committed = None;
                    self.candidate = None;
                    self.loaded = true;
                    self.first_wait = true;
                    self.aborted = false;
                    self.pending_abort = None;
                    self.defaults = None;
                    if self.inputs.profile == crate::Profile::ShowUi
                        && self.layout.placement == Placement::Row
                    {
                        let mut inputs = self.inputs.clone();
                        inputs.state = eval::initial_state(&self.program);
                        let defaults = eval::evaluate(&self.program, &inputs);
                        out.evaluations += 1;
                        out.fuel += defaults.fuel;
                        if let Some(tree) = defaults.tree {
                            let retention = match self.queries.accounts.retain_tree(&tree) {
                                Ok(retention) => Some(retention),
                                Err(_) => {
                                    let abort = self
                                        .queries
                                        .accounts
                                        .take_abort()
                                        .unwrap_or(eval::Abort::LimitAlloc);
                                    self.account_abort(abort, &mut out);
                                    continue;
                                }
                            };
                            let candidate = Arc::new(Candidate {
                                instance_epoch: epoch,
                                revision: 0,
                                layout: self.layout.clone(),
                                state_hash: state_hash(&inputs.state),
                                tree: Arc::new(tree),
                                retention,
                                first: None,
                                loading: false,
                                controls: BTreeMap::new(),
                                bindings: BTreeMap::new(),
                                sentinels: BTreeMap::new(),
                            });
                            out.defaults = Some(candidate.clone());
                            self.defaults = Some(candidate);
                        } else {
                            self.aborted = true;
                            self.first_wait = false;
                            self.pending_abort = defaults.abort;
                            out.abort = defaults.abort;
                            out.diagnostics.extend(defaults.diagnostics);
                            self.publish(self.fallback_tree(), BTreeMap::new(), false, &mut out);
                            continue;
                        }
                    }
                    out.effects
                        .push(Effect::Arm(Timer::FirstAnswer(epoch), Decimal::from_i64(5)));
                    cycle = true;
                }
                Event::MeasureComplete(measurement) => {
                    let layout = &self.layout;
                    if let Some(mut candidate) = self
                        .candidate
                        .take_if(|candidate| measurement.key == candidate.layout_key(layout))
                    {
                        let abort = match measurement.reservation {
                            ReservationOutcome::Abort(abort) => Some(abort),
                            ReservationOutcome::Deferred => self.pending_abort,
                        };
                        let fallback = abort.is_some();
                        let height = if fallback {
                            candidate.tree = Arc::new(self.fallback_tree());
                            candidate.retention = None;
                            candidate.controls.clear();
                            candidate.bindings.clear();
                            candidate.sentinels.clear();
                            candidate.loading = false;
                            None
                        } else {
                            match measurement
                                .row_frame
                                .as_ref()
                                .map(|frame| frame.height)
                                .unwrap_or(measurement.height)
                            {
                                Height::Known(height) => Some(height),
                                Height::ProseDeferred => None,
                            }
                        };
                        candidate.layout = self.layout.clone();
                        self.runtime
                            .commit(candidate.revision, candidate.controls.clone());
                        let committed = Committed {
                            candidate,
                            height,
                            content_height: if fallback {
                                Height::ProseDeferred
                            } else {
                                measurement.height
                            },
                            row_frame: if fallback {
                                None
                            } else {
                                measurement.row_frame
                            },
                            fallback,
                        };
                        out.record_effects();
                        out.order
                            .push(OutputStep::Commit(committed.candidate.revision));
                        self.committed = Some(committed.clone());
                        out.committed = Some(committed);
                        if let Some(abort) = abort {
                            self.aborted = true;
                            self.pending_abort = None;
                            out.abort = Some(abort);
                            out.diagnostics.push(Diagnostic::new(
                                abort.code(),
                                crate::DiagnosticPhase::Evaluate,
                            ));
                            self.queries.cancel_all(&mut out.effects);
                            if self.first_wait {
                                self.first_wait = false;
                                out.effects
                                    .push(Effect::Stop(Timer::FirstAnswer(self.queries.epoch)));
                            }
                        }
                    }
                }
                Event::Input {
                    instance,
                    revision,
                    control,
                    binding,
                    value,
                } => {
                    let valid = self.committed.as_ref().is_some_and(|c| {
                        c.candidate.revision == revision
                            && c.candidate.bindings.get(&control) == Some(&binding)
                    });
                    if instance != self.queries.epoch || !valid {
                        out.rejected.push(Rejected::Stale);
                        continue;
                    }
                    if let Err(code) = self.runtime.assign(&Value::text(binding), Some(&value)) {
                        out.diagnostics
                            .push(Diagnostic::new(code, crate::DiagnosticPhase::Evaluate));
                    }
                    cycle = true;
                }
                Event::Activate(activation) => match self.runtime.activate(&activation) {
                    Ok(local) => {
                        out.diagnostics.extend(local.diagnostics);
                        out.effects.extend(
                            local
                                .handoffs
                                .into_iter()
                                .map(|handoff| Effect::Handoff(activation.clone(), handoff)),
                        );
                        for intent in local.intents {
                            match intent {
                                SiteIntent::Refresh(site) => {
                                    refresh.insert(site);
                                }
                                SiteIntent::LoadMore(site) => {
                                    more.insert(site);
                                }
                            }
                        }
                        cycle = true;
                    }
                    Err(error) => out.rejected.push(error),
                },
                Event::HandoffsComplete(activation) => self.runtime.handoffs_complete(&activation),
                Event::Answer {
                    site,
                    identity,
                    epoch,
                    answer,
                } => {
                    cycle |= self.queries.answer(
                        &site,
                        &identity,
                        epoch,
                        answer.as_deref().map_err(String::as_str),
                        &mut out.effects,
                    );
                }
                Event::Timer(Timer::FirstAnswer(epoch)) => {
                    if self.first_wait && epoch == self.queries.epoch {
                        self.first_wait = false;
                        cycle = true;
                    }
                }
                Event::Timer(timer) => cycle |= self.queries.timer(&timer, &mut out.effects),
                Event::WatchAck {
                    identity,
                    epoch,
                    after,
                } => {
                    cycle |= self
                        .queries
                        .watch_ack(&identity, epoch, after, &mut out.effects)
                }
                Event::WatchNotice {
                    identity,
                    epoch,
                    token,
                    first,
                    bytes,
                } => {
                    cycle |= self.queries.watch_notice(
                        &identity,
                        epoch,
                        token,
                        first,
                        &bytes,
                        &mut out.effects,
                    )
                }
                Event::WatchEnd {
                    identity,
                    epoch,
                    error,
                } => {
                    cycle |= self
                        .queries
                        .watch_end(&identity, epoch, error, &mut out.effects)
                }
                Event::PageAnswer {
                    identity,
                    epoch,
                    cursor,
                    answer,
                } => {
                    cycle |= self.queries.page_answer(
                        &identity,
                        epoch,
                        cursor.as_deref(),
                        answer.as_deref().map_err(String::as_str),
                        &mut out.effects,
                    )
                }
                Event::AutoLoadMore {
                    instance,
                    revision,
                    control,
                    token,
                } => {
                    if instance != self.queries.epoch {
                        out.rejected.push(Rejected::Stale);
                        continue;
                    }
                    if let Some(site) = self
                        .committed
                        .as_ref()
                        .filter(|c| c.candidate.revision == revision)
                        .and_then(|c| c.candidate.sentinels.get(&control))
                    {
                        if self.queries.auto_more(site, token) {
                            more.insert(site.clone());
                        }
                        cycle = true;
                    } else {
                        out.rejected.push(Rejected::Stale);
                    }
                }
                Event::Visibility(visible) => {
                    self.queries.visibility(visible, &mut out.effects);
                    cycle = true;
                }
                Event::Foreground(foreground) => {
                    self.queries.foreground(foreground, &mut out.effects);
                    cycle = true;
                }
                Event::Connection(connected) => {
                    self.queries.connection(connected, &mut out.effects);
                    cycle = true;
                }
                Event::Clock { now, zone } => {
                    self.inputs.now = now;
                    self.inputs.zone = zone;
                    cycle = true;
                }
                Event::Layout(layout) => {
                    if self.aborted && self.pending_abort.is_none() {
                        continue;
                    }
                    self.layout = layout;
                    if let Some(candidate) = self.candidate.as_mut() {
                        candidate.layout = self.layout.clone();
                        candidate.first = if self.layout.placement == Placement::Row {
                            self.defaults.clone()
                        } else {
                            None
                        };
                        out.candidate = Some(candidate.clone());
                    } else if let Some(committed) = &self.committed {
                        let mut candidate = committed.candidate.clone();
                        candidate.layout = self.layout.clone();
                        candidate.first = if self.layout.placement == Placement::Row {
                            self.defaults.clone()
                        } else {
                            None
                        };
                        self.candidate = Some(candidate.clone());
                        out.candidate = Some(candidate);
                    }
                }
                Event::Availability(active) => {
                    self.queries.availability(active, &mut out.effects);
                    cycle = true;
                }
            }
        }
        if let Some(abort) = self.queries.accounts.take_abort() {
            self.account_abort(abort, &mut out);
            return out.finish();
        }
        if !cycle || !self.loaded || self.queries.retired || self.aborted {
            return out.finish();
        }
        self.inputs.state = self.runtime.state.clone();
        self.queries.begin();
        self.queries.intents(&refresh, &more);
        let evaluation = eval::evaluate_sites(&self.program, &self.inputs, &mut self.queries);
        out.evaluations += 1;
        out.fuel += evaluation.fuel;
        out.diagnostics.extend(evaluation.diagnostics);
        let tree = if let Some(tree) = evaluation.tree {
            self.queries
                .finalize_more(&refresh, &more, &mut out.effects);
            tree
        } else {
            self.aborted = true;
            out.abort = evaluation.abort;
            self.pending_abort = evaluation.abort;
            self.fallback_tree()
        };
        if let Some(abort) = self.queries.accounts.take_abort() {
            self.account_abort(abort, &mut out);
            return out.finish();
        }
        if self.first_wait && self.queries.settled() {
            self.first_wait = false;
            out.effects
                .push(Effect::Stop(Timer::FirstAnswer(self.queries.epoch)));
        }
        let loading = self.first_wait && !self.aborted;
        let tree = if loading {
            Tree {
                blocks: vec![TreeNode::Component {
                    name: DisplayText::new("LoadingRoot"),
                    props: vec![],
                    children: vec![],
                }],
            }
        } else {
            tree
        };
        self.publish(tree, evaluation.controls, loading, &mut out);
        out.finish()
    }

    fn account_abort(&mut self, abort: eval::Abort, out: &mut Output) {
        self.aborted = true;
        self.pending_abort = Some(abort);
        out.abort = Some(abort);
        out.diagnostics.push(Diagnostic::new(
            abort.code(),
            crate::DiagnosticPhase::Evaluate,
        ));
        self.queries.cancel_all(&mut out.effects);
        if self.first_wait {
            self.first_wait = false;
            out.effects
                .push(Effect::Stop(Timer::FirstAnswer(self.queries.epoch)));
        }
        self.publish(self.fallback_tree(), BTreeMap::new(), false, out);
    }

    fn publish(
        &mut self,
        tree: Tree,
        controls: BTreeMap<ControlId, Action>,
        loading: bool,
        out: &mut Output,
    ) {
        let retention = if self.aborted {
            None
        } else {
            match self.queries.accounts.retain_tree(&tree) {
                Ok(retention) => Some(retention),
                Err(_) => {
                    let abort = self
                        .queries
                        .accounts
                        .take_abort()
                        .unwrap_or(eval::Abort::LimitAlloc);
                    self.account_abort(abort, out);
                    return;
                }
            }
        };
        let Some(revision) = self.revision.checked_add(1) else {
            self.queries.cancel_all(&mut out.effects);
            self.queries.retired = true;
            return;
        };
        self.revision = revision;
        let candidate = Candidate {
            instance_epoch: self.queries.epoch,
            revision,
            layout: self.layout.clone(),
            state_hash: state_hash(&self.runtime.state),
            first: (self.inputs.profile == crate::Profile::ShowUi
                && self.layout.placement == Placement::Row)
                .then(|| self.defaults.clone())
                .flatten(),
            bindings: bindings(&tree),
            sentinels: sentinels(&tree),
            tree: Arc::new(tree),
            retention,
            loading,
            controls: if loading { BTreeMap::new() } else { controls },
        };
        self.candidate = Some(candidate.clone());
        out.candidate = Some(candidate);
    }

    fn fallback_tree(&self) -> Tree {
        Tree {
            blocks: vec![TreeNode::Component {
                name: DisplayText::new("ProgramFallback"),
                props: vec![(
                    DisplayText::new("fallback"),
                    TreeValue::String(self.fallback.clone()),
                )],
                children: vec![],
            }],
        }
    }
}

fn state_hash(state: &BTreeMap<String, Value>) -> String {
    fn part(hash: &mut hmac_sha256::Hash, bytes: &[u8]) {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }

    let mut hash = hmac_sha256::Hash::new();
    hash.update(b"occipital-state-fields-v1\0");
    // Length framing preserves field boundaries without building a record whose
    // combined logical size could overflow. BTreeMap supplies stable key order.
    for (name, value) in state {
        part(&mut hash, name.as_bytes());
        match value.encode_ordered(crate::CodecMode::State) {
            Ok(encoded) => {
                hash.update(b"ordered-value\0");
                part(&mut hash, encoded.as_bytes());
            }
            Err(error) => {
                hash.update(b"codec-error\0");
                let kind = match error {
                    crate::CodecError::UnsupportedKind(kind) => {
                        part(&mut hash, b"UnsupportedKind");
                        part(&mut hash, format!("{kind:?}").as_bytes());
                        continue;
                    }
                    crate::CodecError::ErrorInState => "ErrorInState",
                    crate::CodecError::InvalidEncoding => "InvalidEncoding",
                    crate::CodecError::InvalidNumber => "InvalidNumber",
                    crate::CodecError::Build(_) => "Build",
                    crate::CodecError::Json(_) => "Json",
                };
                part(&mut hash, kind.as_bytes());
            }
        }
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bindings(tree: &Tree) -> BTreeMap<ControlId, String> {
    tree_targets(tree, false)
}

fn sentinels(tree: &Tree) -> BTreeMap<ControlId, String> {
    tree_targets(tree, true)
}

fn tree_targets(tree: &Tree, sites: bool) -> BTreeMap<ControlId, String> {
    fn node(
        current: &TreeNode,
        path: &mut Vec<PathPart>,
        out: &mut BTreeMap<ControlId, String>,
        sites: bool,
    ) {
        let children = match current {
            TreeNode::Component {
                props, children, ..
            } => {
                if !props.iter().any(|(key, value)| {
                    key.as_str() == "disabled" && *value == TreeValue::Boolean(true)
                }) {
                    for (key, value) in props {
                        path.push(PathPart::Prop(key.as_str().into()));
                        prop(value, path, out, sites);
                        path.pop();
                    }
                }
                children
            }
            TreeNode::Markdown { children, .. } => children,
            _ => return,
        };
        for (i, child) in children.iter().enumerate() {
            path.push(PathPart::Child(i));
            node(child, path, out, sites);
            path.pop();
        }
    }
    fn prop(
        value: &TreeValue,
        path: &mut Vec<PathPart>,
        out: &mut BTreeMap<ControlId, String>,
        sites: bool,
    ) {
        match value {
            TreeValue::Binding { state, .. } if !sites => {
                out.insert(ControlId(path.clone()), state.as_str().into());
            }
            TreeValue::Site(site) if sites => {
                out.insert(ControlId(path.clone()), site.as_str().into());
            }
            TreeValue::Node(child) => node(child, path, out, sites),
            TreeValue::List(items) => {
                for (i, value) in items.iter().enumerate() {
                    path.push(PathPart::Index(i));
                    prop(value, path, out, sites);
                    path.pop();
                }
            }
            TreeValue::Record(fields) => {
                for (key, value) in fields {
                    path.push(PathPart::Field(key.as_str().into()));
                    prop(value, path, out, sites);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    for (i, child) in tree.blocks.iter().enumerate() {
        let mut path = vec![PathPart::Child(i)];
        node(child, &mut path, &mut out, sites);
    }
    out
}

// Golden-trace support is compiled for tests and for the `traces` feature,
// so apps can run the committed §16 traces themselves (§14.1).
#[cfg(any(test, feature = "traces"))]
#[cfg_attr(not(test), allow(dead_code))]
mod support;
#[cfg(any(test, feature = "traces"))]
pub(crate) mod traces;

#[cfg(test)]
mod tests;
