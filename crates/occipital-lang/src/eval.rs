//! Program evaluation (language specification §5): one bounded, deterministic
//! pass from a resolved program and its host inputs to a tree, a demand set
//! and runtime diagnostics, or an abort (§9.6).
//!
//! Every charge and limit check happens before the work it pays for or
//! guards (§9.2, §9.4). Value errors stay local values (§4.6); aborts stop the
//! whole evaluation and discard its tree and demands.

mod calls;
mod expr;
pub(crate) mod snapshot;
pub(crate) mod tree;

use std::collections::BTreeMap;
use std::sync::Arc;

use cortexkit_rich_ui_syntax::{Expression as E, Kind, Node};

use crate::data::Prepared;
use crate::prepare::resolve::Resolved;
use crate::{
    Decimal, Diagnostic, DiagnosticCode, DiagnosticPhase, HandleRecord, Profile, ScalarString,
    SourceSpan, TrialOutcome, Value, ValueError, ValueKind,
};

use snapshot::ParseKey;
use tree::Tree;

/// Fuel per evaluation (§9.1, §9.4).
pub(crate) const FUEL_BUDGET: u64 = 1_000_000;
/// The §9.4 limits on what one evaluation may create or output.
const STRING_SCALARS: u64 = 65_536;
const LIST_ELEMENTS: usize = 10_000;
const RECORD_FIELDS: usize = 1_024;
const VALUE_DEPTH: u32 = 32;
const VALUE_BYTES: u64 = 4 * 1024 * 1024;
const ALLOCATION_BYTES: u64 = 16 * 1024 * 1024;
const TREE_NODES: u32 = 2_000;
const TREE_DEPTH: u32 = 32;
const TREE_TEXT: u64 = 262_144;
/// Runtime diagnostics listed per evaluation; the rest are only counted (§9.4).
const DIAGNOSTICS: usize = 50;

/// Why an evaluation stopped as a whole (§9.4, §9.6). Each is a §12.4 code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abort {
    /// `E_FUEL`.
    Fuel,
    /// `E_LIMIT_STRING`.
    LimitString,
    /// `E_LIMIT_LIST`.
    LimitList,
    /// `E_LIMIT_RECORD`.
    LimitRecord,
    /// `E_LIMIT_DEPTH`.
    LimitDepth,
    /// `E_LIMIT_VALUE`.
    LimitValue,
    /// `E_LIMIT_ALLOC`.
    LimitAlloc,
    /// `E_LIMIT_NODES`.
    LimitNodes,
    /// `E_LIMIT_TEXT`.
    LimitText,
    /// `E_LIMIT_LAYOUT`.
    LimitLayout,
}

impl Abort {
    pub(crate) fn code(self) -> DiagnosticCode {
        match self {
            Self::Fuel => DiagnosticCode::Fuel,
            Self::LimitString => DiagnosticCode::LimitString,
            Self::LimitList => DiagnosticCode::LimitList,
            Self::LimitRecord => DiagnosticCode::LimitRecord,
            Self::LimitDepth => DiagnosticCode::LimitDepth,
            Self::LimitValue => DiagnosticCode::LimitValue,
            Self::LimitAlloc => DiagnosticCode::LimitAlloc,
            Self::LimitNodes => DiagnosticCode::LimitNodes,
            Self::LimitText => DiagnosticCode::LimitText,
            Self::LimitLayout => DiagnosticCode::LimitLayout,
        }
    }

    pub(crate) fn outcome(self) -> TrialOutcome {
        match self {
            Self::Fuel => TrialOutcome::Fuel,
            Self::LimitString => TrialOutcome::LimitString,
            Self::LimitList => TrialOutcome::LimitList,
            Self::LimitRecord => TrialOutcome::LimitRecord,
            Self::LimitDepth => TrialOutcome::LimitDepth,
            Self::LimitValue => TrialOutcome::LimitValue,
            Self::LimitAlloc => TrialOutcome::LimitAlloc,
            Self::LimitNodes => TrialOutcome::LimitNodes,
            Self::LimitText => TrialOutcome::LimitText,
            Self::LimitLayout => TrialOutcome::LimitLayout,
        }
    }

    /// The built-ins report their size aborts by code.
    fn from_code(code: &str) -> Self {
        match code {
            "E_LIMIT_STRING" => Self::LimitString,
            "E_LIMIT_LIST" => Self::LimitList,
            "E_LIMIT_RECORD" => Self::LimitRecord,
            "E_LIMIT_DEPTH" => Self::LimitDepth,
            "E_LIMIT_ALLOC" => Self::LimitAlloc,
            _ => Self::LimitValue,
        }
    }
}

/// Everything an evaluation reads besides the program (§5.3). The host
/// builds these values; none of them counts as evaluation allocation (§9.3).
///
/// Hosts receive this type already prepared from their provisional host
/// values (see `ProgramSession::load_event`), so its fields stay private:
/// snapshot preparation and `Parse` results must go through the crate.
#[derive(Clone, Debug)]
pub struct Inputs {
    pub(crate) profile: Profile,
    /// The recorded network policy also controls address activation (§7.5).
    pub(crate) network: bool,
    /// The `show_ui` call's snapshot (§4.8), or null when none was passed.
    pub(crate) data: Value,
    pub(crate) input: Value,
    pub(crate) view: Value,
    pub(crate) now: Decimal,
    pub(crate) zone: Value,
    /// Snapshot values for `$$tc(N)`, keyed by N as written in the source.
    pub(crate) references: BTreeMap<String, Value>,
    pub(crate) handles: Vec<HandleRecord>,
    /// The host's prepared result for each distinct `Parse` call, made in
    /// the data preparation account before evaluation (§10.2). Evaluation
    /// only reads them.
    pub(crate) prepared: BTreeMap<ParseKey, Prepared>,
    /// Current state values by name without `$`. A missing entry reads the
    /// declared initial value.
    pub(crate) state: BTreeMap<String, Value>,
    /// Current site envelopes by site name. A site without one reads its
    /// first-demand loading envelope (§8.3, §8.5, §8.6).
    pub(crate) sites: BTreeMap<String, Value>,
    pub(crate) budget: u64,
}

impl Inputs {
    pub(crate) fn new(profile: Profile) -> Self {
        Self {
            profile,
            network: true,
            data: Value::null(),
            input: Value::null(),
            view: Value::null(),
            now: Decimal::ZERO,
            zone: Value::text("UTC"),
            references: BTreeMap::new(),
            handles: Vec::new(),
            prepared: BTreeMap::new(),
            state: BTreeMap::new(),
            sites: BTreeMap::new(),
            budget: FUEL_BUDGET,
        }
    }
}

/// One site reached in this evaluation, in first-demand order (§8.2). The
/// arguments are the evaluated constructor arguments, in source order.
#[derive(Clone, Debug)]
pub(crate) struct Demand {
    pub site: Arc<str>,
    pub constructor: Arc<str>,
    pub arguments: Vec<Value>,
    /// Dependencies read while computing arguments, including missing fields
    /// and coalesced fallbacks (§8.2). Values alone cannot retain this fact.
    pub dependencies: Vec<Value>,
}

pub(crate) trait SiteResolver {
    fn resolve(&mut self, demand: &Demand) -> Value;
}

/// The result of one evaluation. After an abort there is no tree and no
/// demand, and the abort is the last diagnostic (§9.6).
#[derive(Debug)]
pub(crate) struct Evaluation {
    pub tree: Option<Tree>,
    /// Lossless frozen actions keyed by their paths in this tree. Display
    /// strings in the tree must not be used to reconstruct state writes.
    pub controls: BTreeMap<crate::actions::ControlId, crate::Action>,
    pub abort: Option<Abort>,
    pub demands: Vec<Demand>,
    pub diagnostics: Vec<Diagnostic>,
    pub omitted: usize,
    /// Fuel charged, up to but not including a charge that was refused.
    pub fuel: u64,
    /// §9.3 allocation by values this evaluation created.
    pub allocation: u64,
    /// Output nodes added to the tree.
    pub nodes: u32,
    /// Output text scalars added to the tree.
    pub text: u64,
    /// The longest string this evaluation created, in scalars.
    pub longest_string: u64,
}

/// Evaluate a resolved program once (§5.3).
pub(crate) fn evaluate(program: &Resolved, inputs: &Inputs) -> Evaluation {
    let mut evaluator = Evaluator::new(program, inputs);
    let result = evaluator.body();
    evaluator.finish(result)
}

pub(crate) fn evaluate_sites(
    program: &Resolved,
    inputs: &Inputs,
    resolver: &mut dyn SiteResolver,
) -> Evaluation {
    let mut evaluator = Evaluator::new(program, inputs);
    evaluator.resolver = Some(resolver);
    let result = evaluator.body();
    evaluator.finish(result)
}

/// The declared initial value of every state, by name without `$`. State
/// initializers are literals (§3.6), so this costs no evaluation fuel.
pub(crate) fn initial_state(program: &Resolved) -> BTreeMap<String, Value> {
    program
        .declarations
        .iter()
        .filter(|declaration| declaration.state)
        .map(|declaration| (declaration.name.clone(), literal(&declaration.expression)))
        .collect()
}

/// A literal's value without fuel or allocation, for host-side state values.
fn literal(node: &Node) -> Value {
    let value = match &node.kind {
        Kind::Expression(E::Number(token)) => Decimal::from_literal(token).map(Value::number).ok(),
        Kind::Expression(E::String(text)) => Value::string(text.as_str()).ok(),
        Kind::Expression(E::Boolean(value)) => Some(Value::boolean(*value)),
        Kind::Expression(E::Unary(op)) if op == "-" => match literal(&node.children[0]).kind() {
            ValueKind::Number(number) => Some(Value::number(-*number)),
            _ => None,
        },
        Kind::Expression(E::List) => Value::list(node.children.iter().map(literal).collect()).ok(),
        Kind::Expression(E::Record(keys)) => Value::record(
            keys.iter()
                .map(|key| ScalarString::new(key.as_str()))
                .zip(node.children.iter().map(literal))
                .collect(),
        )
        .ok(),
        _ => Some(Value::null()),
    };
    value.unwrap_or_else(Value::null)
}

/// Fuel and the per-evaluation size accounts (§9.2–§9.4).
#[derive(Debug)]
struct Meter {
    fuel: u64,
    budget: u64,
    allocation: u64,
    nodes: u32,
    text: u64,
    longest_string: u64,
}

impl Meter {
    /// A charge that would pass the budget aborts there and is not recorded.
    fn charge(&mut self, fuel: u64) -> Result<(), Abort> {
        match self.fuel.checked_add(fuel) {
            Some(total) if total <= self.budget => {
                self.fuel = total;
                Ok(())
            }
            _ => Err(Abort::Fuel),
        }
    }

    /// Admit a value about to be created: its nesting, its logical size and
    /// its shallow allocation (§9.3), in that order, before it is built.
    fn admit(&mut self, shallow: u64, logical: u64, depth: u32) -> Result<(), Abort> {
        if depth > VALUE_DEPTH {
            return Err(Abort::LimitDepth);
        }
        if logical > VALUE_BYTES {
            return Err(Abort::LimitValue);
        }
        match self.allocation.checked_add(shallow) {
            Some(total) if total <= ALLOCATION_BYTES => {
                self.allocation = total;
                Ok(())
            }
            _ => Err(Abort::LimitAlloc),
        }
    }
}

#[derive(Clone, Debug)]
enum Memo {
    Pending,
    Running,
    Done(Value, BTreeMap<String, Value>),
}

struct Evaluator<'a> {
    program: &'a Resolved,
    inputs: &'a Inputs,
    /// Named values (not state) by name.
    names: BTreeMap<&'a str, usize>,
    memo: Vec<Memo>,
    /// `Each` binders in scope, innermost last (§5.5).
    binders: Vec<(&'a str, Value)>,
    meter: Meter,
    demands: Vec<Demand>,
    actions: Vec<crate::Action>,
    diagnostics: Vec<Diagnostic>,
    omitted: usize,
    resolver: Option<&'a mut dyn SiteResolver>,
    provenance: BTreeMap<String, Value>,
}

impl<'a> Evaluator<'a> {
    fn new(program: &'a Resolved, inputs: &'a Inputs) -> Self {
        Self {
            program,
            inputs,
            names: program
                .declarations
                .iter()
                .enumerate()
                .filter(|(_, declaration)| !declaration.state)
                .map(|(index, declaration)| (declaration.name.as_str(), index))
                .collect(),
            memo: vec![Memo::Pending; program.declarations.len()],
            binders: Vec::new(),
            meter: Meter {
                fuel: 0,
                budget: inputs
                    .budget
                    .min(crate::accounts::UnitKind::Evaluation.budget()),
                allocation: 0,
                nodes: 0,
                text: 0,
                longest_string: 0,
            },
            demands: Vec::new(),
            actions: Vec::new(),
            diagnostics: Vec::new(),
            omitted: 0,
            resolver: None,
            provenance: BTreeMap::new(),
        }
    }

    fn finish(mut self, result: Result<Tree, Abort>) -> Evaluation {
        let (tree, abort) = match result {
            Ok(tree) => (Some(tree), None),
            Err(abort) => {
                // An abort discards the evaluation's results: no tree, and no
                // request intent may be dispatched (§8.2, §9.6).
                self.demands.clear();
                self.push(Diagnostic::new(abort.code(), DiagnosticPhase::Evaluate));
                (None, Some(abort))
            }
        };
        let controls = tree
            .as_ref()
            .map(|tree| crate::actions::controls(tree, self.actions))
            .unwrap_or_default();
        Evaluation {
            tree,
            controls,
            abort,
            demands: self.demands,
            diagnostics: self.diagnostics,
            omitted: self.omitted,
            fuel: self.meter.fuel,
            allocation: self.meter.allocation,
            nodes: self.meter.nodes,
            text: self.meter.text,
            longest_string: self.meter.longest_string,
        }
    }

    fn charge(&mut self, fuel: u64) -> Result<(), Abort> {
        self.meter.charge(fuel)
    }

    fn span(&self, node: &Node) -> SourceSpan {
        self.program.parsed.positions.span(node.span)
    }

    fn push(&mut self, diagnostic: Diagnostic) {
        if self.diagnostics.len() < DIAGNOSTICS {
            self.diagnostics.push(diagnostic);
        } else {
            self.omitted += 1;
        }
    }

    /// Report an error value that reached a prop or text (§4.6).
    fn report(&mut self, error: &ValueError, subject: Option<&str>) {
        let code = code(&error.code);
        let mut diagnostic = Diagnostic::new(code, DiagnosticPhase::Evaluate);
        if let Ok(positioned) = diagnostic.clone().with_span(error.span) {
            diagnostic = positioned;
        }
        if let Some(subject) = subject {
            diagnostic = diagnostic.with_subject(subject);
        }
        self.push(diagnostic);
    }

    /// A new atom (number, boolean, null, missing or error): 16 bytes (§9.3).
    fn atom(&mut self, value: Value) -> Result<Value, Abort> {
        self.meter.admit(16, 16, 0)?;
        Ok(value)
    }

    fn error(&mut self, code: &str, node: &Node) -> Result<Value, Abort> {
        let span = self.span(node);
        self.atom(Value::error(ValueError::new(code, span)))
    }

    /// Check a string's length before anything is charged or copied, then
    /// charge its production, ⌈L/8⌉ (§9.2, §9.4).
    fn produce(&mut self, scalars: u64) -> Result<(), Abort> {
        if scalars > STRING_SCALARS {
            return Err(Abort::LimitString);
        }
        self.charge(scalars.div_ceil(8))
    }

    /// Create a string whose length was already checked.
    fn string(&mut self, text: String, scalars: u64) -> Result<Value, Abort> {
        if scalars > STRING_SCALARS {
            return Err(Abort::LimitString);
        }
        let size = 16 + 4 * scalars;
        self.meter.admit(size, size, 0)?;
        self.meter.longest_string = self.meter.longest_string.max(scalars);
        Value::string(text).map_err(|_| Abort::LimitValue)
    }

    fn list(&mut self, values: Vec<Value>) -> Result<Value, Abort> {
        if values.len() > LIST_ELEMENTS {
            return Err(Abort::LimitList);
        }
        let logical = values.iter().fold(16u64, |size, value| {
            size.saturating_add(value.logical_size())
        });
        let shallow = 16 + 8 * values.len() as u64;
        self.meter.admit(shallow, logical, nesting(&values))?;
        Value::list(values).map_err(|_| Abort::LimitValue)
    }

    fn record(&mut self, fields: Vec<(ScalarString, Value)>) -> Result<Value, Abort> {
        let present: Vec<_> = fields
            .iter()
            .filter(|(_, value)| value.value_type() != crate::ValueType::Missing)
            .collect();
        if present.len() > RECORD_FIELDS {
            return Err(Abort::LimitRecord);
        }
        let logical = present.iter().fold(16u64, |size, (key, value)| {
            size.saturating_add(16 + 4 * key.len())
                .saturating_add(value.logical_size())
        });
        let shallow = 16 + 16 * present.len() as u64;
        let depth = nesting(present.iter().map(|(_, value)| value));
        self.meter.admit(shallow, logical, depth)?;
        // Literal keys are unique (the parser refuses duplicates), so a build
        // failure can only be a size overflow already refused above.
        Value::record(fields).map_err(|_| Abort::LimitValue)
    }

    fn component(
        &mut self,
        name: &str,
        props: Vec<(ScalarString, Value)>,
        bindings: Vec<(Arc<str>, Arc<str>)>,
        children: Vec<Value>,
    ) -> Result<Value, Abort> {
        let props: Vec<_> = props
            .into_iter()
            .filter(|(_, value)| value.value_type() != crate::ValueType::Missing)
            .collect();
        let logical = props
            .iter()
            .map(|(_, value)| value)
            .chain(&children)
            .fold(32u64, |size, value| {
                size.saturating_add(value.logical_size())
            })
            .saturating_add(16 * bindings.len() as u64);
        let shallow = 32 + 8 * (props.len() + bindings.len() + children.len()) as u64;
        let depth = nesting(props.iter().map(|(_, value)| value).chain(&children));
        self.meter.admit(shallow, logical, depth)?;
        let record = crate::value::Record::new(props).map_err(|_| Abort::LimitValue)?;
        Value::bound_component(name, record, bindings, children).map_err(|_| Abort::LimitValue)
    }

    fn action(&mut self, steps: Vec<crate::ActionStep>) -> Result<Value, Abort> {
        let arguments = || steps.iter().flat_map(|step| step.arguments.iter());
        let logical = arguments().fold(16u64, |size, value| {
            size.saturating_add(value.logical_size())
        });
        let shallow = 16 + 8 * arguments().count() as u64;
        self.meter.admit(shallow, logical, nesting(arguments()))?;
        Value::action(steps).map_err(|_| Abort::LimitValue)
    }

    /// A named value: evaluated at its first use and memoized for the rest of
    /// the evaluation, so its expression is charged once (§5.3 rule 3).
    fn named(&mut self, index: usize, node: &'a Node) -> Result<Value, Abort> {
        match &self.memo[index] {
            Memo::Done(value, provenance) => {
                self.provenance.extend(provenance.clone());
                return Ok(value.clone());
            }
            // The checker refuses cycles (§5.4); this guard keeps an unchecked
            // program from recursing without bound.
            Memo::Running => return self.error("E_CYCLE", node),
            Memo::Pending => {}
        }
        self.memo[index] = Memo::Running;
        let outer = std::mem::take(&mut self.provenance);
        // An Each binder never reaches a named value's expression (§5.5).
        let binders = std::mem::take(&mut self.binders);
        let declaration = &self.program.declarations[index];
        let result = match &declaration.expression.kind {
            Kind::Expression(E::Call(constructor))
                if matches!(constructor.as_str(), "Query" | "Watch" | "PagedList") =>
            {
                self.site(index, constructor, &declaration.expression)
            }
            _ => self.expr(&declaration.expression),
        };
        self.binders = binders;
        let value = result?;
        let provenance = self.provenance.clone();
        self.provenance.extend(outer);
        self.memo[index] = Memo::Done(value.clone(), provenance);
        Ok(value)
    }
}

/// One more than the deepest of the parts (§9.4 value nesting).
fn nesting<'v>(values: impl IntoIterator<Item = &'v Value>) -> u32 {
    values
        .into_iter()
        .map(Value::depth)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

/// Map an error value's code to its §12.4 identity.
fn code(code: &str) -> DiagnosticCode {
    DiagnosticCode::ALL
        .iter()
        .copied()
        .find(|known| known.as_str() == code)
        .unwrap_or(DiagnosticCode::Type)
}

#[cfg(test)]
mod tests;
