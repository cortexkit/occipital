//! Negative traces: the reserved write names are refused in every profile
//! (§2.3, §7.4). v1 has no write constructor or step, so a program naming
//! one never evaluates, and a reply shows the tag as literal text.

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

use std::fmt::Write;

#[cfg(test)]
use super::{fixture, golden};
use crate::catalog::Catalog;
use crate::geometry::{LayoutContext, Network, Placement};
use crate::{CheckContext, Decimal, DiagnosticCode, Profile, TrialOutcome};

/// Write calls in every place a program could put them: an interpolation, a
/// declaration, a display prop and an action prop.
const PROGRAMS: [&str; 4] = [
    "{Run(\"rm -rf build\")}",
    "export const removal = Mutation(\"delete_file\", {path: \"a\"})\n\n{removal}",
    "<Badge text={Run(\"rm -rf build\")} />",
    "<Button label=\"Delete\" action={Mutation(\"delete_file\", {path: \"a\"})} />",
];

/// In a reply only tags carry values (§3.7), so a write can appear only in a
/// tag prop.
const REPLIES: [&str; 2] = [
    "Before. <Badge text={Run(\"rm -rf build\")} /> After.",
    "<Callout tone=\"warning\" title={Mutation(\"delete_file\", {path: \"a\"})}>Body</Callout>",
];

pub(crate) fn writes() -> String {
    let mut trace = String::new();
    for profile in [Profile::ShowUi, Profile::ToolRow, Profile::ToolPage] {
        for source in PROGRAMS {
            let context = CheckContext::new(profile);
            let result = crate::check::trial::check_program(
                source.as_bytes(),
                &context,
                &crate::eval::Inputs::new(profile),
            )
            .unwrap();
            let codes: Vec<_> = result.diagnostics().iter().map(|d| d.code()).collect();
            assert!(
                codes.contains(&DiagnosticCode::WriteNotAllowed),
                "{}: {source}: {codes:?}",
                profile.name()
            );
            // A refused program is never evaluated, so it cannot emit any
            // effect, write or otherwise.
            assert!(!result.ok());
            assert_eq!(result.trial().outcome, TrialOutcome::NotRun);
            writeln!(
                trace,
                "profile={} source={source:?}\n  check={}",
                profile.name(),
                serde_json::to_string(&result).unwrap()
            )
            .unwrap();
        }
    }
    for source in REPLIES {
        let mut session = crate::reply::Session::new(
            Profile::Reply,
            "1.0".into(),
            Catalog::shared(),
            LayoutContext {
                width: Decimal::from_integer(320).unwrap(),
                content_size_category: "normal".into(),
                network: Network::Yes,
                placement: Placement::Page,
            },
            crate::SharedAccount::chat(),
            crate::SharedAccount::app(),
        )
        .unwrap();
        let result = session
            .finish(source.as_bytes(), Default::default(), vec![])
            .unwrap();
        let codes: Vec<_> = result.diagnostics.iter().map(|d| d.code()).collect();
        assert!(
            codes.contains(&DiagnosticCode::WriteNotAllowed),
            "reply: {source}: {codes:?}"
        );
        // The failed tag degrades to its own source text (§2.1).
        assert!(!result.literals.is_empty());
        assert!(
            result.plain_text.as_str().contains("delete_file")
                || result.plain_text.as_str().contains("rm -rf build"),
            "{}",
            result.plain_text.as_str()
        );
        writeln!(trace, "profile=reply source={source:?}").unwrap();
        for diagnostic in &result.diagnostics {
            writeln!(
                trace,
                "  diagnostic {}",
                serde_json::to_string(diagnostic).unwrap()
            )
            .unwrap();
        }
        writeln!(trace, "  plain-text={:?}", result.plain_text.as_str()).unwrap();
        super::write_tree(&mut trace, &result.tree);
    }
    trace
}

#[test]
fn reserved_writes_are_refused_in_every_profile_golden_trace() {
    let first = writes();
    assert_eq!(first, writes());
    golden(
        "reserved-writes.trace",
        fixture!("golden/reserved-writes.trace"),
        &first,
    );
}
