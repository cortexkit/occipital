use serde::{Deserialize, Serialize};

use crate::{DisplayText, SourceSpan};

mod code;
pub use code::DiagnosticCode;

/// An error blocks drawing; a warning does not (§12.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The program cannot be drawn.
    Error,
    /// The program can still be drawn.
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Which stage produced a diagnostic (§12.1 `phase`).
pub enum DiagnosticPhase {
    /// Parsing the source.
    Parse,
    /// Checking against the catalog and profile.
    Check,
    /// Parsing a snapshot with a format hint.
    Data,
    /// The trial evaluation, or a report from an app.
    Evaluate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum DataSource {
    #[serde(rename = "data")]
    Data,
}

/// A one-based scalar position in a snapshot, not in the program (§12.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct DataPosition {
    source: DataSource,
    line: u32,
    column: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    record: Option<u64>,
}

impl DataPosition {
    /// A one-based line and column in a snapshot.
    pub fn new(line: u32, column: u32) -> Result<Self, DiagnosticBuildError> {
        if line == 0 || column == 0 {
            return Err(DiagnosticBuildError::InvalidPosition);
        }
        Ok(Self {
            source: DataSource::Data,
            line,
            column,
            record: None,
        })
    }

    /// Adds the one-based CSV record number.
    pub fn with_record(mut self, record: u64) -> Result<Self, DiagnosticBuildError> {
        if record == 0 {
            return Err(DiagnosticBuildError::InvalidPosition);
        }
        self.record = Some(record);
        Ok(self)
    }
}

/// Source ranges have an exclusive end; snapshot positions carry `source: data`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum DiagnosticSpan {
    /// A position in the program source.
    Source(SourceSpan),
    /// A position in a snapshot.
    Data(DataPosition),
}

impl From<SourceSpan> for DiagnosticSpan {
    fn from(span: SourceSpan) -> Self {
        Self::Source(span)
    }
}

impl From<DataPosition> for DiagnosticSpan {
    fn from(position: DataPosition) -> Self {
        Self::Data(position)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Why a diagnostic could not be built in the §12.1 shape.
pub enum DiagnosticBuildError {
    /// A line, column or record number is zero.
    InvalidPosition,
    /// The message exceeds 300 characters.
    MessageTooLong,
    /// An `Each` path was given to a diagnostic outside evaluation.
    PathOutsideEvaluation,
    /// A cycle was given to a code other than `E_CYCLE`.
    CycleForOtherCode,
}

/// An outbound problem with code, severity, phase, message and optional metadata
/// (language specification §12.1).
///
/// Default messages use fixed templates. Caller-supplied wording must also be
/// fixed and must never include runtime source, snapshot or answer text. Source
/// names, signatures, hints and cycle names are sanitized separately. There is
/// intentionally no deserializer bypassing these output invariants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    code: DiagnosticCode,
    severity: Severity,
    phase: DiagnosticPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    span: Option<DiagnosticSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<DisplayText>,
    message: DisplayText,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<DisplayText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<DisplayText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cycle: Option<Vec<DisplayText>>,
}

impl Diagnostic {
    /// A diagnostic with the code's fixed message.
    pub fn new(code: DiagnosticCode, phase: DiagnosticPhase) -> Self {
        Self {
            code,
            severity: code.severity(),
            phase,
            span: None,
            subject: None,
            message: DisplayText::new(code.message()),
            signature: None,
            hint: None,
            path: None,
            cycle: None,
        }
    }

    /// Refine a template using trusted, static wording (at most 300 scalars).
    ///
    /// Callers must not interpolate data into the wording. Requiring a static
    /// borrow rejects wording borrowed from short-lived source/answer values:
    /// ```compile_fail
    /// use occipital_lang::{Diagnostic, DiagnosticCode, DiagnosticPhase};
    /// let answer = String::from("private snapshot contents");
    /// let diagnostic = Diagnostic::new(DiagnosticCode::Type, DiagnosticPhase::Evaluate)
    ///     .with_message(&answer);
    /// ```
    pub fn with_message(mut self, message: &'static str) -> Result<Self, DiagnosticBuildError> {
        let message = DisplayText::new(message);
        if message.as_str().chars().count() > 300 {
            return Err(DiagnosticBuildError::MessageTooLong);
        }
        self.message = message;
        Ok(self)
    }

    /// Fill each `{}` in a fixed template with a count, in order.
    ///
    /// Data diagnostics report counts, row numbers and column positions this way
    /// (§10.6). Only integers can be interpolated, so the wording can never quote
    /// snapshot contents, and the 300-scalar bound still applies.
    pub(crate) fn with_counts(
        mut self,
        template: &'static str,
        counts: &[u64],
    ) -> Result<Self, DiagnosticBuildError> {
        // Templates are fixed in the crate, so a placeholder/count mismatch is a
        // programming error that tests catch, never something data can cause.
        debug_assert_eq!(template.matches("{}").count(), counts.len());
        let mut pieces = template.split("{}");
        let mut message = String::from(pieces.next().unwrap_or_default());
        for (piece, count) in pieces.zip(counts) {
            message.push_str(&count.to_string());
            message.push_str(piece);
        }
        if message.chars().count() > 300 {
            return Err(DiagnosticBuildError::MessageTooLong);
        }
        self.message = DisplayText::new(&message);
        Ok(self)
    }

    /// Adds the source or snapshot position.
    pub fn with_span(
        mut self,
        span: impl Into<DiagnosticSpan>,
    ) -> Result<Self, DiagnosticBuildError> {
        let span = span.into();
        if matches!(span, DiagnosticSpan::Source(source) if !source.is_valid()) {
            return Err(DiagnosticBuildError::InvalidPosition);
        }
        self.span = Some(span);
        Ok(self)
    }

    /// The name from program source or a registry, never a snapshot excerpt.
    pub fn with_subject(mut self, subject: &str) -> Self {
        self.subject = Some(DisplayText::new(subject));
        self
    }

    /// Adds the signature of the component, prop or function used wrongly.
    pub fn with_signature(mut self, signature: &str) -> Self {
        self.signature = Some(DisplayText::new(signature));
        self
    }

    /// Adds a one-sentence hint.
    pub fn with_hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(DisplayText::new(hint));
        self
    }

    /// Element indexes are zero-based and ordered outermost first (§12.1).
    pub fn with_path(mut self, path: Vec<u32>) -> Result<Self, DiagnosticBuildError> {
        if self.phase != DiagnosticPhase::Evaluate {
            return Err(DiagnosticBuildError::PathOutsideEvaluation);
        }
        self.path = Some(path);
        Ok(self)
    }

    /// Adds the names in an `E_CYCLE`, in order.
    pub fn with_cycle(mut self, names: &[&str]) -> Result<Self, DiagnosticBuildError> {
        if self.code != DiagnosticCode::Cycle {
            return Err(DiagnosticBuildError::CycleForOtherCode);
        }
        self.cycle = Some(names.iter().map(|name| DisplayText::new(name)).collect());
        Ok(self)
    }

    /// Applies a refinement the crate builds from its own positions, fixed
    /// wording or code, which always satisfies the §12.1 rules. If one ever
    /// did not, the diagnostic is kept without that refinement: an
    /// unpositioned or default-worded diagnostic is still worth reporting,
    /// and the library must not crash the app over it.
    pub(crate) fn refine(
        self,
        refinement: impl FnOnce(Self) -> Result<Self, DiagnosticBuildError>,
    ) -> Self {
        let unrefined = self.clone();
        refinement(self).unwrap_or(unrefined)
    }

    /// The §12.4 code.
    pub fn code(&self) -> DiagnosticCode {
        self.code
    }

    /// The code's severity.
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// The stage that produced it.
    pub fn phase(&self) -> DiagnosticPhase {
        self.phase
    }

    /// The source or snapshot position, if any.
    pub fn span(&self) -> Option<DiagnosticSpan> {
        self.span
    }

    /// The sanitized message.
    pub fn message(&self) -> &str {
        self.message.as_str()
    }

    fn sort_key(&self) -> (bool, u32, u32, Severity) {
        // Positionless problems follow positioned ones; ties preserve emission
        // order after putting errors first. End positions do not move a warning
        // ahead of an error starting at the same scalar (§12.2).
        let (absent, line, column) = match self.span {
            Some(DiagnosticSpan::Source(span)) => (false, span.line, span.column),
            Some(DiagnosticSpan::Data(position)) => (false, position.line, position.column),
            None => (true, 0, 0),
        };
        (absent, line, column, self.severity)
    }
}

/// A trial either did not run, succeeded, or stopped with an evaluation abort.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrialOutcome {
    #[serde(rename = "not_run")]
    /// The trial did not run: the profile has none, or the check found errors.
    NotRun,
    #[serde(rename = "ok")]
    /// The trial finished.
    Ok,
    #[serde(rename = "E_FUEL")]
    /// `E_FUEL`: the evaluation ran out of fuel.
    Fuel,
    #[serde(rename = "E_LIMIT_STRING")]
    /// `E_LIMIT_STRING`.
    LimitString,
    #[serde(rename = "E_LIMIT_LIST")]
    /// `E_LIMIT_LIST`.
    LimitList,
    #[serde(rename = "E_LIMIT_RECORD")]
    /// `E_LIMIT_RECORD`.
    LimitRecord,
    #[serde(rename = "E_LIMIT_DEPTH")]
    /// `E_LIMIT_DEPTH`.
    LimitDepth,
    #[serde(rename = "E_LIMIT_VALUE")]
    /// `E_LIMIT_VALUE`.
    LimitValue,
    #[serde(rename = "E_LIMIT_ALLOC")]
    /// `E_LIMIT_ALLOC`.
    LimitAlloc,
    #[serde(rename = "E_LIMIT_NODES")]
    /// `E_LIMIT_NODES`.
    LimitNodes,
    #[serde(rename = "E_LIMIT_TEXT")]
    /// `E_LIMIT_TEXT`.
    LimitText,
    #[serde(rename = "E_LIMIT_LAYOUT")]
    /// `E_LIMIT_LAYOUT`.
    LimitLayout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// The §12.2 `trial` member.
pub struct TrialResult {
    /// Fuel the trial used.
    pub fuel: u64,
    /// Output nodes the trial built.
    pub nodes: u32,
    /// `ok`, `not_run` or the abort code.
    pub outcome: TrialOutcome,
}

/// The program check response (§12.2), not an attestation of host approvals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    ok: bool,
    language: &'static str,
    execution_dialect: &'static str,
    approval_checked: bool,
    requires: &'static str,
    diagnostics: Vec<Diagnostic>,
    omitted: usize,
    trial: TrialResult,
}

impl CheckResult {
    /// Assemble a v1.0 check. An aborted trial cannot report success, even if
    /// the checker has not supplied its corresponding error diagnostic yet.
    pub fn new(diagnostics: Vec<Diagnostic>, trial: TrialResult) -> Self {
        Self::with_omitted(diagnostics, trial, 0)
    }

    pub(crate) fn with_omitted(
        mut diagnostics: Vec<Diagnostic>,
        trial: TrialResult,
        already_omitted: usize,
    ) -> Self {
        // §12.2: `ok` means no error. A trial that did not run fails nothing
        // by itself: tool rows and pages have no trial, and a skipped show_ui
        // trial always comes with the error that skipped it. Only an aborted
        // trial counts against `ok`, even before its error diagnostic is added.
        let ok = matches!(trial.outcome, TrialOutcome::Ok | TrialOutcome::NotRun)
            && !diagnostics.iter().any(|d| d.severity == Severity::Error);
        diagnostics.sort_by_key(Diagnostic::sort_key);
        let omitted = already_omitted + diagnostics.len().saturating_sub(50);
        diagnostics.truncate(50);
        Self {
            ok,
            language: "1.0",
            execution_dialect: crate::host::EXECUTION_DIALECT,
            approval_checked: false,
            requires: "1.0",
            diagnostics,
            omitted,
            trial,
        }
    }

    /// True when no diagnostic is an error; warnings do not block.
    pub fn ok(&self) -> bool {
        self.ok
    }

    /// The listed diagnostics, sorted by position, errors first, at most 50.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// How many diagnostics were not listed.
    pub fn omitted(&self) -> usize {
        self.omitted
    }

    /// The trial evaluation's result.
    pub fn trial(&self) -> TrialResult {
        self.trial
    }
}

#[cfg(test)]
#[path = "diagnostic/tests.rs"]
mod tests;
