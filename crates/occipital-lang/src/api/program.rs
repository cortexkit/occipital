//! The program checker and the program session (§5.7, §8, §12.2).

use std::collections::BTreeMap;

use super::host::HostContext;
use super::{HostError, HostValues, VersionDescriptor};
use crate::accounts::{Accounts, Refused};
use crate::check::trial::TrialFault;
use crate::geometry::LayoutContext;
use crate::scheduler::{Authority, Committed, Event, Output, ReadVersion, Reads, Session};
use crate::{
    program_fallback, AccountScope, CheckContext, CheckResult, Severity, SharedAccount, Value,
};

/// Check a program and run its trial evaluation, returning the §12.2 result.
///
/// The source is taken as bytes, so invalid UTF-8 is repaired and reported
/// as `W_ENCODING` (§3.1). For `show_ui` the trial evaluates the load and
/// the first-answer timeout with the stored snapshots and site defaults,
/// with no layout context or metrics, and makes the page-placement visit
/// reservation; a failed reservation is reported in `trial.outcome` (§5.7).
/// A dialect other than `1.0` is `E_UNSUPPORTED_VERSION` (§13).
///
/// The check runs with private, fresh accounts: checking a program never
/// spends a chat's or an app's shared budget.
pub fn check_program(
    source: &[u8],
    context: &CheckContext,
    host: &HostValues,
) -> Result<CheckResult, HostError> {
    let program = crate::check::check(source, context)?;
    let accounts = Accounts::new(
        AccountScope::Interface,
        SharedAccount::chat(),
        SharedAccount::app(),
    )
    .map_err(Refused::Scope)?;
    let inputs = HostContext::new(context, &program).inputs(host, &accounts)?;
    crate::check::trial::check_resolved(&program, context, &inputs).map_err(|fault| match fault {
        TrialFault::Preparation(refused) => HostError::Refused(refused),
        TrialFault::MissingTree => HostError::Internal("the trial evaluation produced no tree"),
        TrialFault::Reservation(_) => HostError::Internal("the trial reservation failed"),
    })
}

/// One evaluation of a tool template: its tree, or the abort that stopped it.
#[derive(Debug)]
pub struct TemplateEvaluation {
    /// The evaluated tree, absent only when evaluation aborted.
    pub tree: Option<crate::Tree>,
    /// The checker's warnings and the evaluation's diagnostics, in order.
    pub diagnostics: Vec<crate::Diagnostic>,
    /// The abort that stopped evaluation, such as running out of fuel.
    pub abort: Option<crate::eval::Abort>,
}

impl TemplateEvaluation {
    /// `ok`, or the abort as a trial outcome, the form the check result uses.
    pub fn outcome(&self) -> crate::TrialOutcome {
        self.abort
            .map_or(crate::TrialOutcome::Ok, crate::eval::Abort::outcome)
    }
}

/// Check and evaluate a tool template (`tool_row` or `tool_page`) once, with
/// the `input` and `view` in `context.tool` and the time and zone in `host`.
///
/// A tool template makes no reads and keeps no state, so it needs no session:
/// the same template and values always give the same tree. An app evaluates
/// a row once for each fresh draw, and the template checker evaluates sample
/// calls with it. A template with an error diagnostic is refused with
/// [`HostError::Rejected`], and any other profile with
/// [`HostError::Internal`]. Like [`check_program`], it runs on private, fresh
/// accounts.
pub fn evaluate_template(
    source: &[u8],
    context: &CheckContext,
    host: &HostValues,
) -> Result<TemplateEvaluation, HostError> {
    if !matches!(
        context.profile,
        crate::Profile::ToolRow | crate::Profile::ToolPage
    ) {
        return Err(HostError::Internal(
            "evaluate_template takes a tool_row or tool_page context",
        ));
    }
    let program = crate::check::check(source, context)?;
    if program
        .diagnostics
        .iter()
        .any(|d| d.severity() == Severity::Error)
    {
        return Err(HostError::Rejected(program.diagnostics.clone()));
    }
    let accounts = Accounts::new(
        AccountScope::Interface,
        SharedAccount::chat(),
        SharedAccount::app(),
    )
    .map_err(Refused::Scope)?;
    let mut inputs = HostContext::new(context, &program).inputs(host, &accounts)?;
    inputs.profile = context.profile;
    inputs.state = crate::eval::initial_state(&program);
    let evaluation = crate::eval::evaluate(&program, &inputs);
    let mut diagnostics = program.diagnostics.clone();
    diagnostics.extend(evaluation.diagnostics);
    Ok(TemplateEvaluation {
        tree: evaluation.tree,
        diagnostics,
        abort: evaluation.abort,
    })
}

/// One running program instance: one program in one row or page, with one
/// scheduler (§8.1).
///
/// The host feeds it events in batches and gets back the requests and other
/// effects to perform, revisions to measure and committed revisions (§8.2).
/// It never measures: the host measures each candidate with [`measure`]
/// (any thread) and sends the result back as
/// [`ProgramEvent::MeasureComplete`]. Only a completion for the current
/// revision whose layout key matches the current layout context commits.
///
/// [`measure`]: crate::measure
/// [`ProgramEvent::MeasureComplete`]: crate::ProgramEvent::MeasureComplete
pub struct ProgramSession {
    session: Session,
    host: HostContext,
    descriptor: VersionDescriptor,
}

impl ProgramSession {
    /// Check `source` and create an instance for it. A program with any
    /// error diagnostic is refused with [`HostError::Rejected`]: the host
    /// shows its fallback and no effect runs (§13.4).
    ///
    /// `authority` and `versions` are the broker-validated provenance and
    /// read versions every request identity carries (§8.2); the operations
    /// themselves come from the context's registry. `chat` and `app` are the
    /// shared accounts every instance of the chat and app reserves from.
    pub fn new(
        source: &[u8],
        context: &CheckContext,
        authority: Authority,
        versions: BTreeMap<String, ReadVersion>,
        layout: LayoutContext,
        chat: SharedAccount,
        app: SharedAccount,
    ) -> Result<Self, HostError> {
        let program = crate::check::check(source, context)?;
        if program
            .diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Error)
        {
            return Err(HostError::Rejected(program.diagnostics.clone()));
        }
        let host = HostContext::new(context, &program);
        let reads = Reads {
            authority,
            versions,
            registry: context.registry.clone(),
        };
        // The §11.1 fallback is computed once here: after an abort the
        // session commits it without scanning the source again.
        let fallback = program_fallback(source, &context.show_ui.title, &context.show_ui.summary);
        let session = Session::new(
            program,
            crate::eval::Inputs::new(context.profile),
            reads,
            layout,
            fallback.text,
        )
        .with_accounts(chat, app)
        .map_err(Refused::Scope)?;
        Ok(Self {
            session,
            host,
            descriptor: VersionDescriptor::for_dialect(&context.execution_dialect),
        })
    }

    /// The load event for a fresh instance epoch: the host values prepared
    /// as evaluation inputs, with snapshot parsing charged to this
    /// instance's data preparation account (§9.1, §10.2). `restored` is the
    /// saved state by name without `$`; invalid entries are dropped one by
    /// one (§8.9).
    pub fn load_event(
        &self,
        epoch: u64,
        host: &HostValues,
        restored: BTreeMap<String, Value>,
    ) -> Result<Event, HostError> {
        Ok(Event::Load {
            epoch,
            inputs: Box::new(self.host.inputs(host, self.session.accounts())?),
            restored,
        })
    }

    /// Apply one batch of events and return what the host must do (§8.2).
    pub fn batch(&mut self, events: Vec<Event>) -> Output {
        self.session.batch(events)
    }

    /// The revision currently on screen, if one has committed.
    pub fn committed(&self) -> Option<&Committed> {
        self.session.committed.as_ref()
    }

    /// The dialect and catalog this program runs under (§13.2).
    pub fn descriptor(&self) -> &VersionDescriptor {
        &self.descriptor
    }
}
