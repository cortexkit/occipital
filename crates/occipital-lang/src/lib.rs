//! Shared, pure implementation of the apps' rich UI language, version 1.
//!
//! The crate checks, evaluates and measures programs and replies the same
//! way on every platform. It performs no I/O: the catalog is embedded at
//! compile time, and every request, timer and fetch is an effect the host
//! performs and answers with an event.
//!
//! The public API has one entry point per job:
//!
//! - [`check_program`] checks program source bytes and returns the §12.2
//!   [`CheckResult`], including the trial evaluation (§5.7).
//! - [`evaluate_template`] evaluates a tool row or page template once with
//!   a call's input and view.
//! - [`ProgramSession`] runs one program instance: events in; requests,
//!   effects, revisions to measure and committed revisions out (§8).
//! - [`measure`] measures a revision under a [`LayoutContext`] with the
//!   host's [`Metrics`] callbacks. It is pure and may run on any thread.
//! - [`ReplySession`] streams a reply, finishes it with the records stamped
//!   at store time, and takes late snapshot and measurement events (§3.10).
//! - [`program_fallback`] builds a program's §11.1 fallback text from source
//!   bytes, even when the source does not parse.
//! - [`SharedAccount`] is the host-owned chat or app account every instance
//!   reserves from (§9.1).
//! - [`export_json`] is the machine-readable function table and profile
//!   rules, generated from the checker's own tables.
//! - [`reply_teaching`] is the text that teaches a model the reply tags,
//!   generated from the catalog's `teaching.reply` section.
//! - `run_golden_traces` (cargo feature `traces`) replays the committed
//!   golden traces on the running target (§14.1).
//!
//! The remaining items are the types those entry points take and return:
//! values ([`Value`]), trees ([`Tree`]), diagnostics ([`Diagnostic`]), the
//! provisional host inputs ([`CheckContext`], [`HostValues`] and the records
//! they hold), and the §13 version descriptors ([`VersionDescriptor`]).
//! Revisions and tree values are `Send + Sync`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
// The library runs inside apps on untrusted programs and replies, where a
// panic crashes the person's app. A broken internal assumption must become an
// error the caller can handle, never a crash, so library code may not panic.
// Catalog reads treat a malformed rule as absent instead of failing only
// because the catalog is the compiled-in one, pinned by its digest and
// checked by its shape tests; a catalog from elsewhere would need real errors.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented
    )
)]

// The catalog stays private, and its runtime consumers are not implemented yet.
#[cfg_attr(not(test), allow(dead_code))]
mod catalog;

#[cfg_attr(not(test), allow(dead_code))]
mod canonical;

// Decimal is re-exported for value construction; arithmetic helpers stay internal.
#[cfg_attr(not(test), allow(dead_code))]
mod number;

// Pinned Unicode primitives are shared by text, addresses and built-ins.
#[cfg_attr(not(test), allow(dead_code))]
mod unicode;

mod address;
mod api;
mod diagnostic;
mod fallback;
mod text;
mod value;

mod host;

// The reply teaching text, read from the catalog's `teaching` section.
mod teaching;

#[cfg_attr(not(test), allow(dead_code))]
mod accounts;

// Catalog/profile checking adds diagnostics to resolved programs.
#[cfg_attr(not(test), allow(dead_code))]
mod check;

// One-shot syntax preparation parses the source and resolves its declarations.
#[cfg_attr(not(test), allow(dead_code))]
mod prepare;

// The expression evaluator will dispatch through these metered primitives.
#[cfg_attr(not(test), allow(dead_code))]
mod builtins;

// Facts a module reports, worded and toned the same way in every app.
mod fact;
pub use fact::{format_fact, FactError, FormattedFact, Tone};
mod schedule;
pub use schedule::{format_schedule, ScheduleError};

// Program evaluation: the host state machine and public sessions will drive it.
#[cfg_attr(not(test), allow(dead_code))]
mod eval;

// Local activation handling is separate from evaluation and site scheduling.
#[cfg_attr(not(test), allow(dead_code))]
mod actions;

// Pure event batching and read scheduling; measurement remains host driven.
#[cfg_attr(not(test), allow(dead_code))]
mod scheduler;

// Format-hinted snapshot parsing; `Parse`, `show_ui` hints and the hosts'
// data preparation accounts will call it.
#[cfg_attr(not(test), allow(dead_code))]
mod data;

// Pure catalog geometry, driven by host metrics rather than a platform UI.
#[cfg_attr(not(test), allow(dead_code))]
mod geometry;

// Reply streaming and late snapshot delivery share the pure measurement cycle.
#[cfg_attr(not(test), allow(dead_code))]
mod reply;

// The §16 examples' fixtures, golden-trace helpers and diagnostic coverage.
// The `traces` feature compiles the fixtures and trace builders into the
// library so the public golden-trace runner can replay them (§14.1).
#[cfg(any(test, feature = "traces"))]
#[cfg_attr(not(test), allow(dead_code))]
mod section16;

// Checking, program sessions, host inputs, versions, the export and the
// golden-trace runner.
pub use api::{
    check_program, evaluate_template, export_json, reply_stamp_digest, CatalogDescriptor,
    DataSnapshot, HostError, HostValues, ProgramSession, TemplateEvaluation, VersionDescriptor,
};
#[cfg(feature = "traces")]
pub use api::{run_golden_traces, GoldenTrace};

/// The README's example, run as a doc test so the published one can't rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeExample;

// What a program session takes and returns (§8).
pub use actions::{Activation, ControlId, Handoff, PathPart, Rejected};
pub use eval::{Abort, Inputs as PreparedInputs};
pub use scheduler::{
    Authority, Candidate as ProgramRevision, Committed as CommittedRevision,
    Effect as ProgramEffect, Epoch, Event as ProgramEvent, Identity as ReadIdentity,
    Output as ProgramOutput, OutputStep, ReadVersion, Timer,
};

// Measurement (catalog geometry).
pub use geometry::{
    measure, Height, LayoutContext, LayoutKey, MeasureError, MeasureRevision, MeasuredBox,
    Measurement, Metrics, Network, Placement, RenderState, ReservationOutcome, RowFrame, TextStyle,
};

// Reply sessions (§3.10).
pub use reply::{
    Error as ReplyError, Event as ReplyEvent, Revision as ReplyRevision, Session as ReplySession,
};

// The evaluated tree (§14.3 tree ABI).
pub use eval::tree::{MarkdownKind, Tree, TreeNode, TreeStep, TreeValue};

pub use catalog::Catalog;
pub use teaching::{reply_teaching, ReplyTeaching};

pub use accounts::{
    AccountExhausted, AccountScope, AccountUsage, Refused, ReservationError, RetainedAllocation,
    ScopeError, SharedAccount, SharedRetention, RETAINED_GRANULE, RETAINED_HEADER,
};
pub use address::Address;
pub use diagnostic::{
    CheckResult, DataPosition, Diagnostic, DiagnosticBuildError, DiagnosticCode, DiagnosticPhase,
    DiagnosticSpan, Severity, TrialOutcome, TrialResult,
};
pub use fallback::{program_fallback, FallbackText};
pub use host::{
    CatalogVersion, CheckContext, ColumnType, DataFormat, HandleKind, HandleRecord, HostProjection,
    Operation, OperationRegistry, Profile, ReferenceRecord, ReferenceState, Schema, SchemaType,
    ShowUiArguments, ToolInputs,
};
pub use number::{Decimal, NumberError};
pub use text::DisplayText;
pub use value::{
    Action, ActionOperation, ActionStep, CodecError, CodecMode, Component, Handle, HandleIdentity,
    Origin, Record, Relation, Revision, ScalarString, SiteEnvelope, Snapshot, SourceSpan, Value,
    ValueBuildError, ValueError, ValueFailure, ValueKind, ValueType,
};
