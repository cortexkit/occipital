//! The reply examples of §16.8–§16.11 as golden traces: the final parse with
//! the records stamped at store time, then each fetched snapshot as a later
//! revision, measured and committed.
//!
//! Text assertions are hand-derived from the sources, the fixtures and the
//! catalog's text forms (§11.3); trees, ids and heights are recorded.

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

use super::support::*;
use super::*;
#[cfg(test)]
use crate::section16::golden;
use crate::section16::{fixture, write_tree};
use crate::{DiagnosticCode as Code, HandleKind, Severity};

fn revision(trace: &mut String, s: &mut Session, label: &str, r: &Revision) {
    writeln!(
        trace,
        "{label}: id={} provisional={} literals={:?}",
        r.id, r.provisional, r.literals
    )
    .unwrap();
    for diagnostic in &r.diagnostics {
        writeln!(
            trace,
            "  diagnostic {}",
            serde_json::to_string(diagnostic).unwrap()
        )
        .unwrap();
    }
    writeln!(trace, "  plain-text={:?}", r.plain_text.as_str()).unwrap();
    write_tree(trace, &r.tree);
    let measured = measure(r);
    writeln!(
        trace,
        "  measured height={:?} states={:?}",
        measured.height,
        measured
            .boxes
            .iter()
            .map(|b| format!("{:?}", b.state))
            .collect::<Vec<_>>()
    )
    .unwrap();
    s.event(Event::MeasureComplete(Box::new(measured))).unwrap();
    assert_eq!(s.committed.as_ref().map(|c| c.0.id), Some(r.id));
    writeln!(trace, "  committed id={}", r.id).unwrap();
}

/// Stream the reply one byte at a time, finish it, and check that the result
/// equals a one-shot replay of the same text and records (§3.10).
fn finish_streamed(
    trace: &mut String,
    name: &str,
    source: &str,
    references: BTreeMap<u64, ReferenceRecord>,
    handles: Vec<HandleRecord>,
) -> (Session, Revision) {
    writeln!(trace, "example={name} profile=reply").unwrap();
    let mut streamed = session();
    let mut provisional = 0;
    for byte in source.as_bytes() {
        if let Some(r) = streamed.chunk(std::slice::from_ref(byte)).unwrap() {
            assert!(r.provisional);
            provisional += 1;
        }
    }
    let result = streamed
        .finish(&[], references.clone(), handles.clone())
        .unwrap();
    let replay = session()
        .finish(source.as_bytes(), references, handles)
        .unwrap();
    same(&result, &replay);
    writeln!(
        trace,
        "streamed one byte at a time: provisional={provisional}"
    )
    .unwrap();
    (streamed, result)
}

fn deliver(trace: &mut String, s: &mut Session, reference: u64, bytes: &str) -> Revision {
    writeln!(trace, "snapshot $$tc({reference}) bytes={bytes:?}").unwrap();
    let r = s
        .event(Event::Snapshot {
            reference,
            result: Ok(bytes.as_bytes().to_vec()),
        })
        .unwrap()
        .expect("a delivered snapshot yields a revision");
    revision(trace, s, &format!("snapshot-{reference}"), &r);
    r
}

fn records(references: &[u64]) -> BTreeMap<u64, ReferenceRecord> {
    references.iter().map(|n| (*n, record())).collect()
}

fn no_errors(r: &Revision) {
    assert!(
        r.diagnostics
            .iter()
            .all(|d| d.severity() != Severity::Error),
        "{:?}",
        r.diagnostics
    );
}

pub(crate) fn test_run() -> String {
    let source = fixture!("sources/16.08-reply-test-run.md");
    let mut trace = String::new();
    let (mut s, r) = finish_streamed(
        &mut trace,
        "16.08-reply-test-run",
        source,
        records(&[7]),
        vec![],
    );
    no_errors(&r);
    revision(&mut trace, &mut s, "final", &r);
    assert!(r
        .plain_text
        .as_str()
        .starts_with("The test run finished with one failure. [1 failed] [17 passed]"));
    let ready = deliver(
        &mut trace,
        &mut s,
        7,
        "suite,seconds\nPageLoaderTests,1.5\nCacheTests,0.25\n",
    );
    no_errors(&ready);
    // Two CSV records become two chart points (catalog text form).
    assert!(ready
        .plain_text
        .as_str()
        .ends_with("How long each suite took:\n\n[chart: bar, 2 points]"));
    assert_eq!(measure(&ready).height, measure(&r).height);
    trace
}

#[test]
fn section_16_8_reply_test_run_golden_trace() {
    let first = test_run();
    assert_eq!(first, test_run());
    golden(
        "16.08-reply-test-run.trace",
        fixture!("golden/16.08-reply-test-run.trace"),
        &first,
    );
}

fn artifact(id: &str, column: u32, end_column: u32) -> HandleRecord {
    HandleRecord {
        record_id: format!("record-{id}"),
        computer: "computer".into(),
        session: "session".into(),
        scope: "scope".into(),
        kind: HandleKind::Artifact,
        // The stamped span is the `Artifact(...)` call on line 5 of §16.9.
        span: SourceSpan {
            line: 5,
            column,
            end_line: 5,
            end_column,
        },
        resource_id: Some(id.into()),
        version: Some("1".into()),
        content_hash: None,
        mime: Some("image/jpeg".into()),
        bytes: Some(2048),
        available: true,
    }
}

pub(crate) fn places() -> String {
    let source = fixture!("sources/16.09-reply-places.md");
    let mut trace = String::new();
    let (mut s, r) = finish_streamed(
        &mut trace,
        "16.09-reply-places",
        source,
        records(&[12, 14]),
        vec![artifact("art_k1", 22, 40), artifact("art_k2", 42, 60)],
    );
    no_errors(&r);
    revision(&mut trace, &mut s, "final", &r);
    let text = r.plain_text.as_str();
    // Both citations name one source, so both are number 1.
    assert_eq!(text.matches("[1]").count(), 2, "{text}");
    assert!(
        text.contains("[2 images: Kahve Durağı, inside and out]"),
        "{text}"
    );
    deliver(
        &mut trace,
        &mut s,
        12,
        "Kahve Durağı: two minutes from the station, opens at 7.",
    );
    let mapped = deliver(
        &mut trace,
        &mut s,
        14,
        r#"[{"name":"Kahve Durağı","lat":41.0,"lon":29.0},{"name":"Liman Kahvesi","lat":41.01,"lon":29.01}]"#,
    );
    no_errors(&mapped);
    assert!(
        mapped.plain_text.as_str().contains("[map: 2 places]"),
        "{}",
        mapped.plain_text.as_str()
    );
    trace
}

#[test]
fn section_16_9_reply_places_golden_trace() {
    let first = places();
    assert_eq!(first, places());
    golden(
        "16.09-reply-places.trace",
        fixture!("golden/16.09-reply-places.trace"),
        &first,
    );
}

pub(crate) fn disk_use() -> String {
    let source = fixture!("sources/16.10-reply-disk-use.md");
    let mut trace = String::new();
    let (mut s, r) = finish_streamed(
        &mut trace,
        "16.10-reply-disk-use",
        source,
        records(&[22]),
        vec![],
    );
    no_errors(&r);
    revision(&mut trace, &mut s, "final", &r);
    let ready = deliver(
        &mut trace,
        &mut s,
        22,
        r#"[{"path":"~/Library/Caches","bytes":5368709120},{"path":"~/Work","bytes":2147483648},{"path":"~/Downloads","bytes":1073741824}]"#,
    );
    no_errors(&ready);
    let text = ready.plain_text.as_str();
    assert!(text.contains("[table: 3 rows]"), "{text}");
    // The inline badges in the Markdown table keep their cells.
    assert!(text.contains("[Yes]") && text.contains("[No]"), "{text}");
    trace
}

#[test]
fn section_16_10_reply_disk_use_golden_trace() {
    let first = disk_use();
    assert_eq!(first, disk_use());
    golden(
        "16.10-reply-disk-use.trace",
        fixture!("golden/16.10-reply-disk-use.trace"),
        &first,
    );
}

pub(crate) fn degrades() -> String {
    let source = fixture!("sources/16.11-reply-degrades.md");
    let mut trace = String::new();
    let (mut s, r) = finish_streamed(
        &mut trace,
        "16.11-reply-degrades",
        source,
        BTreeMap::new(),
        vec![],
    );
    revision(&mut trace, &mut s, "final", &r);
    // Exactly the two diagnostics §16.11 names; everything after the first
    // paragraph's badge is literal text.
    let mut codes: Vec<_> = r.diagnostics.iter().map(|d| d.code().as_str()).collect();
    codes.sort_unstable();
    assert_eq!(codes, ["E_REPLY_FORBIDDEN", "E_UNKNOWN_COMPONENT"]);
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d.code() == Code::UnknownComponent
            && serde_json::to_value(d).unwrap()["subject"] == "Gauge"));
    assert_eq!(
        r.plain_text.as_str(),
        "Here is the summary. [Draft]\n\nexport const total = Sum(data.rows.sales)\n\nThe total is {total}. <Gauge value={0.4} /> <Badge tone=\"info\" text={total} />"
    );
    trace
}

#[test]
fn section_16_11_reply_degrades_golden_trace() {
    let first = degrades();
    assert_eq!(first, degrades());
    golden(
        "16.11-reply-degrades.trace",
        fixture!("golden/16.11-reply-degrades.trace"),
        &first,
    );
}
