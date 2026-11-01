//! The golden-trace runner: replays every committed §16 golden trace on the
//! running target so an app can check its build against them (§14.1).

// This module runs only on the committed §16 golden fixtures (in tests and
// with the `traces` feature), never on program or reply input. A panic here
// is a failed trace, not a crash on untrusted input, so the library's
// no-panic lints do not apply.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use crate::section16::fixture;

/// A committed golden: its file name, its committed text and the builder
/// that produces the same trace on this build.
type Golden = (&'static str, &'static str, fn() -> String);

/// The outcome of one committed golden trace.
#[derive(Clone, Debug)]
pub struct GoldenTrace {
    /// The golden file's name, such as `16.05-calc-paint.trace`.
    pub name: &'static str,
    /// The committed golden trace, embedded at compile time.
    pub expected: &'static str,
    /// The trace this build produced.
    pub actual: String,
    /// A second run of the same trace in the same process, which must be
    /// identical to the first (determinism, §14.1).
    pub replayed: String,
}

impl GoldenTrace {
    /// Whether this build reproduced the committed trace byte for byte, on
    /// both runs.
    pub fn passed(&self) -> bool {
        self.actual == self.expected && self.replayed == self.actual
    }
}

/// Run every committed §16 golden trace and the reserved-writes negative
/// trace, twice each, and return one outcome per trace in file-name order.
///
/// The fixtures, sources and goldens are compiled into the library, so the
/// runner reads no files. Each trace also checks the hand-derived assertions
/// written beside it (visible texts, effect kinds, diagnostic codes).
///
/// # Panics
///
/// Panics when one of those hand-derived assertions fails, naming the
/// assertion and the trace; a byte difference from the committed golden is
/// reported through [`GoldenTrace::passed`] instead.
pub fn run_golden_traces() -> Vec<GoldenTrace> {
    use crate::reply::traces as reply;
    use crate::scheduler::traces as program;
    let traces: [Golden; 12] = [
        (
            "16.01-row-bash.trace",
            fixture!("golden/16.01-row-bash.trace"),
            program::row_bash,
        ),
        (
            "16.02-row-edit.trace",
            fixture!("golden/16.02-row-edit.trace"),
            program::row_edit,
        ),
        (
            "16.03-detail-bash.trace",
            fixture!("golden/16.03-detail-bash.trace"),
            program::detail_bash,
        ),
        (
            "16.04-detail-work.trace",
            fixture!("golden/16.04-detail-work.trace"),
            program::detail_work,
        ),
        (
            "16.05-calc-paint.trace",
            fixture!("golden/16.05-calc-paint.trace"),
            program::calc_paint,
        ),
        (
            "16.06-data-sales.trace",
            fixture!("golden/16.06-data-sales.trace"),
            program::data_sales,
        ),
        (
            "16.07-live-view.trace",
            fixture!("golden/16.07-live-view.trace"),
            program::live_view,
        ),
        (
            "16.08-reply-test-run.trace",
            fixture!("golden/16.08-reply-test-run.trace"),
            reply::test_run,
        ),
        (
            "16.09-reply-places.trace",
            fixture!("golden/16.09-reply-places.trace"),
            reply::places,
        ),
        (
            "16.10-reply-disk-use.trace",
            fixture!("golden/16.10-reply-disk-use.trace"),
            reply::disk_use,
        ),
        (
            "16.11-reply-degrades.trace",
            fixture!("golden/16.11-reply-degrades.trace"),
            reply::degrades,
        ),
        (
            "reserved-writes.trace",
            fixture!("golden/reserved-writes.trace"),
            crate::section16::writes::writes,
        ),
    ];
    traces
        .into_iter()
        .map(|(name, expected, build)| GoldenTrace {
            name,
            expected,
            actual: build(),
            replayed: build(),
        })
        .collect()
}
