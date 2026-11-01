//! The effect-free snapshot/default trial used by the program checker (§5.7).

use crate::eval::{self, Abort, Evaluation, Inputs};
use crate::geometry::{known_work_visits, MeasureError, Placement};
use crate::prepare::resolve::Resolved;
use crate::{
    CheckContext, CheckResult, Diagnostic, DiagnosticPhase, NumberError, Profile, Severity,
    TrialOutcome, TrialResult,
};

const PAGE_VISITS: u64 = 100_000;

/// A broken checker/evaluator invariant is an internal failure, not a producer
/// diagnostic. The caller can degrade without unwinding or blaming layout work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TrialFault {
    Preparation(crate::accounts::Refused),
    MissingTree,
    Reservation(MeasureError),
}

/// Run static checking first, then the `show_ui` load and first-answer timeout.
/// Host snapshot values are read, but restored state and answers are not: the
/// checker tests declared state and loading defaults, never a live session.
pub(crate) fn check_program(
    source: &[u8],
    context: &CheckContext,
    host: &Inputs,
) -> Result<CheckResult, TrialFault> {
    let program = super::check(source, context).map_err(TrialFault::Preparation)?;
    check_resolved(&program, context, host)
}

/// The trial of `check_program` for an already checked program. The public
/// checker prepares snapshot and `Parse` inputs from the resolved program
/// first, so it checks once and runs the trial here.
pub(crate) fn check_resolved(
    program: &Resolved,
    context: &CheckContext,
    host: &Inputs,
) -> Result<CheckResult, TrialFault> {
    let diagnostics = program.diagnostics.clone();
    let trial = TrialResult {
        fuel: 0,
        nodes: 0,
        outcome: TrialOutcome::NotRun,
    };
    if context.profile != Profile::ShowUi
        || diagnostics.iter().any(|d| d.severity() == Severity::Error)
    {
        return Ok(CheckResult::new(diagnostics, trial));
    }

    let mut inputs = host.clone();
    inputs.profile = context.profile;
    inputs.state = eval::initial_state(program);
    inputs.sites.clear();
    // Load is followed by the five-second deadline for the first site answers
    // (§5.7). No answers or actions arrive during the trial, so the timeout
    // re-evaluates the same defaults rather than waiting on a real clock.
    // Direct evaluation cannot issue requests, arm timers or commit a tree.
    let load = evaluate_trial(program, &inputs)?;
    if load.abort.is_some() {
        return Ok(finish(diagnostics, load, 0));
    }
    let timeout = evaluate_trial(program, &inputs)?;
    Ok(finish(diagnostics, timeout, load.fuel))
}

fn evaluate_trial(program: &Resolved, inputs: &Inputs) -> Result<Evaluation, TrialFault> {
    let mut evaluation = eval::evaluate(program, inputs);
    if evaluation.abort.is_none() {
        let tree = evaluation.tree.as_ref().ok_or(TrialFault::MissingTree)?;
        // Pre-layout text runs remain undefined (known gap 7); sum only catalog work, never decoded bytes.
        if let Some(abort) = reserve_visits(known_work_visits(tree, Placement::Page))? {
            evaluation.abort = Some(abort);
            evaluation
                .diagnostics
                .push(Diagnostic::new(abort.code(), DiagnosticPhase::Evaluate));
        }
    }
    Ok(evaluation)
}

fn reserve_visits(work: Result<u64, MeasureError>) -> Result<Option<Abort>, TrialFault> {
    match work {
        Ok(visits) if visits <= PAGE_VISITS => Ok(None),
        Ok(_) | Err(MeasureError::Number(NumberError::Overflow)) => Ok(Some(Abort::LimitLayout)),
        // Checked props are normalized before counting work. Invalid trees or
        // host-only geometry reaching this point indicate an internal fault.
        Err(error) => Err(TrialFault::Reservation(error)),
    }
}

fn finish(
    mut diagnostics: Vec<Diagnostic>,
    evaluation: Evaluation,
    prior_fuel: u64,
) -> CheckResult {
    let trial = TrialResult {
        fuel: prior_fuel + evaluation.fuel,
        nodes: evaluation.nodes,
        outcome: evaluation.abort.map_or(TrialOutcome::Ok, Abort::outcome),
    };
    // Without external answers the timeout repeats the load's errors;
    // report those diagnostics once, not once per evaluation.
    diagnostics.extend(evaluation.diagnostics);
    CheckResult::with_omitted(diagnostics, trial, evaluation.omitted)
}

#[cfg(test)]
mod tests;
