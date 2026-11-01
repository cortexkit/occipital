//! The public API end to end, using only items the crate exports: check a
//! program, run a session through `measure_complete`, stream and finish a
//! reply, build a program fallback and regenerate the function-table/profile
//! export. Expected texts are derived by hand from the §16.5 and §16.8
//! example sources, not recorded from the implementation.

use std::collections::BTreeMap;

use occipital_lang::{
    check_program, export_json, measure, program_fallback, Authority, Catalog, CheckContext,
    Decimal, DiagnosticCode, HostError, HostValues, LayoutContext, Metrics, Network, Placement,
    Profile, ProgramEvent, ProgramRevision, ProgramSession, ReferenceRecord, ReferenceState,
    ReplyEvent, ReplyRevision, ReplySession, SharedAccount, TextStyle, Tree, TreeNode, TreeValue,
    TrialOutcome, Value, VersionDescriptor,
};

const CALC_PAINT: &str = include_str!("../fixtures/section16/sources/16.05-calc-paint.ui");
const TEST_RUN: &str = include_str!("../fixtures/section16/sources/16.08-reply-test-run.md");

/// Fixed text metrics, so heights do not depend on a platform text system.
struct Fixed;

impl Metrics for Fixed {
    fn l(&self, _: TextStyle) -> Decimal {
        Decimal::from_integer(20).unwrap()
    }
    fn t(&self, _: &str, _: TextStyle, _: Decimal, _: Option<u32>) -> Decimal {
        Decimal::from_integer(20).unwrap()
    }
    fn tw(&self, text: &str, _: TextStyle) -> Decimal {
        Decimal::from_integer(text.len() as i128).unwrap()
    }
}

fn layout(placement: Placement) -> LayoutContext {
    LayoutContext {
        width: Decimal::from_integer(320).unwrap(),
        content_size_category: "large".into(),
        network: Network::Yes,
        placement,
    }
}

fn authority() -> Authority {
    Authority {
        computer: "computer".into(),
        session: "session".into(),
        scope_epoch: "scope-1".into(),
        provider: "provider".into(),
        entity: "entity".into(),
        serving_computer: "computer".into(),
        verified_holder: "computer".into(),
    }
}

/// Every string anywhere in a tree, for substring assertions: adjacent text
/// runs are joined, and each prop string and block ends a line.
fn strings(tree: &Tree) -> String {
    fn node(item: &TreeNode, out: &mut String) {
        match item {
            TreeNode::Text(text) => out.push_str(text.as_str()),
            TreeNode::Component {
                props, children, ..
            } => {
                for (_, prop) in props {
                    value(prop, out);
                }
                children.iter().for_each(|child| node(child, out));
            }
            TreeNode::Markdown { children, .. } => {
                children.iter().for_each(|child| node(child, out))
            }
            _ => {}
        }
    }
    fn value(item: &TreeValue, out: &mut String) {
        match item {
            TreeValue::String(text) => {
                out.push_str(text.as_str());
                out.push('\n');
            }
            TreeValue::List(items) => items.iter().for_each(|item| value(item, out)),
            TreeValue::Node(inner) => node(inner, out),
            _ => {}
        }
    }
    let mut out = String::new();
    for block in &tree.blocks {
        node(block, &mut out);
        out.push('\n');
    }
    out
}

/// Measure a candidate as a host would and commit it.
fn commit(session: &mut ProgramSession, candidate: &ProgramRevision) {
    let measured = measure(
        &candidate.measure_revision(),
        &layout(Placement::Row),
        &Fixed,
    )
    .unwrap();
    assert_eq!(measured.key, candidate.layout_key(&layout(Placement::Row)));
    let out = session.batch(vec![ProgramEvent::MeasureComplete(measured)]);
    assert_eq!(
        out.committed.map(|c| c.candidate.revision),
        Some(candidate.revision)
    );
}

#[test]
fn public_api_checks_runs_streams_falls_back_and_exports() {
    // Check: §16.5 checks clean in show_ui, and its trial runs to the end.
    let context = CheckContext::new(Profile::ShowUi);
    let checked = check_program(CALC_PAINT.as_bytes(), &context, &HostValues::default()).unwrap();
    assert!(checked.ok(), "{:?}", checked.diagnostics());
    assert_eq!(checked.trial().outcome, TrialOutcome::Ok);

    // Session: load, measure, commit; then set $coats to 1 and commit again.
    let mut session = ProgramSession::new(
        CALC_PAINT.as_bytes(),
        &context,
        authority(),
        BTreeMap::new(),
        layout(Placement::Row),
        SharedAccount::chat(),
        SharedAccount::app(),
    )
    .unwrap();
    assert_eq!(session.descriptor(), &VersionDescriptor::current());
    let load = session
        .load_event(42, &HostValues::default(), BTreeMap::new())
        .unwrap();
    let out = session.batch(vec![load]);
    // A show_ui row measures its frame from the defaults tree (rowFrame.first).
    assert!(out.defaults.is_some());
    let first = out.candidate.expect("the load evaluates a revision");
    assert!(session.committed().is_none());
    commit(&mut session, &first);
    // Width 4, length 5, height 2.5 and two coats: 40.1 m² of wall, 8.02 L,
    // so ⌈8.02 / 2.5⌉ = 4 cans.
    let shown = strings(&session.committed().unwrap().candidate.tree);
    assert!(shown.contains("Buy 4 cans of 2.5 L"), "{shown}");
    assert!(shown.contains("40.1 m²"), "{shown}");

    let committed = &session.committed().unwrap().candidate;
    let (control, _) = committed
        .bindings()
        .find(|(_, state)| *state == "coats")
        .expect("the coats slider binds $coats");
    let input = ProgramEvent::Input {
        instance: 42,
        revision: committed.revision,
        control: control.clone(),
        binding: "coats".into(),
        value: Value::number(Decimal::from_integer(1).unwrap()),
    };
    let next = session
        .batch(vec![input])
        .candidate
        .expect("an input re-evaluates");
    commit(&mut session, &next);
    // One coat: 4.01 L, so ⌈4.01 / 2.5⌉ = 2 cans.
    let shown = strings(&session.committed().unwrap().candidate.tree);
    assert!(shown.contains("Buy 2 cans of 2.5 L"), "{shown}");
    assert!(shown.contains("4.0 L"), "{shown}");

    // Reply: stream one byte at a time, finish, and match a one-shot replay.
    let records = || -> BTreeMap<u64, ReferenceRecord> {
        [(
            7,
            ReferenceRecord {
                state: ReferenceState::Resolved,
                reason: None,
                tool: Some("bash".into()),
                bytes: Some(52),
                call_key: Some("call-7".into()),
            },
        )]
        .into()
    };
    let reply = || {
        ReplySession::new(
            Profile::Reply,
            "1.0".into(),
            Catalog::v1(),
            layout(Placement::Page),
            SharedAccount::chat(),
            SharedAccount::app(),
        )
        .unwrap()
    };
    let mut streamed = reply();
    let mut provisional = 0;
    for byte in TEST_RUN.as_bytes() {
        if let Some(revision) = streamed.chunk(std::slice::from_ref(byte)).unwrap() {
            assert!(revision.provisional);
            provisional += 1;
        }
    }
    assert!(provisional > 0);
    let last = streamed.finish(&[], records(), vec![]).unwrap();
    let replayed = reply()
        .finish(TEST_RUN.as_bytes(), records(), vec![])
        .unwrap();
    assert!(!last.provisional);
    assert_eq!(last.tree, replayed.tree);
    assert_eq!(last.plain_text, replayed.plain_text);
    assert_eq!(last.diagnostics, replayed.diagnostics);
    assert!(last
        .plain_text
        .as_str()
        .starts_with("The test run finished with one failure. [1 failed] [17 passed]"));
    let measured = measure(&last.measure_revision(), &layout(Placement::Page), &Fixed).unwrap();
    streamed
        .event(ReplyEvent::MeasureComplete(Box::new(measured)))
        .unwrap();
    assert_eq!(streamed.committed.as_ref().map(|c| c.0.id), Some(last.id));

    // Fallback: the header comes from title and summary (§11.1), and works
    // on bytes that are not valid UTF-8.
    let mut broken = CALC_PAINT.as_bytes().to_vec();
    broken.push(0xFF);
    let fallback = program_fallback(&broken, "Paint calculator", "One room");
    assert!(fallback
        .text
        .as_str()
        .starts_with("Paint calculator\nOne room"));

    // Export: regenerated from the crate and identical to the committed copy.
    assert_eq!(
        export_json(),
        include_str!("../export/functions-and-profiles.json")
    );
}

#[test]
fn an_unsupported_dialect_is_refused_before_any_effect() {
    let mut context = CheckContext::new(Profile::ShowUi);
    context.execution_dialect = "2.0".into();
    let checked = check_program(CALC_PAINT.as_bytes(), &context, &HostValues::default()).unwrap();
    assert!(!checked.ok());
    assert!(checked
        .diagnostics()
        .iter()
        .any(|d| d.code() == DiagnosticCode::UnsupportedVersion));
    let refused = ProgramSession::new(
        CALC_PAINT.as_bytes(),
        &context,
        authority(),
        BTreeMap::new(),
        layout(Placement::Row),
        SharedAccount::chat(),
        SharedAccount::app(),
    );
    let Err(HostError::Rejected(diagnostics)) = refused else {
        panic!("a program in an unsupported dialect must not run");
    };
    assert!(diagnostics
        .iter()
        .any(|d| d.code() == DiagnosticCode::UnsupportedVersion));
}

#[test]
fn version_descriptor_names_dialect_catalog_digest_and_layout() {
    let descriptor = VersionDescriptor::current();
    assert_eq!(descriptor.execution_dialect, "1.0");
    assert_eq!(descriptor.catalog.version, "1.0");
    assert_eq!(descriptor.catalog.layout, "1");
    // The checked-in digest literal was produced by the canonicalizer in
    // tools/catalog_digest.py, never by this crate (catalog digestRule).
    assert_eq!(
        descriptor.catalog.digest,
        include_str!("../catalog/catalog-v1.digest").trim()
    );
    assert_eq!(
        serde_json_shape(&descriptor),
        format!(
            "{{\"executionDialect\":\"1.0\",\"catalog\":{{\"version\":\"1.0\",\"digest\":\"{}\",\"layout\":\"1\",\"aliases\":[\"beb0904e8b402916d84d7609a8b06db30ac308f692dcc048db2dc1138cd0c986\"]}}}}",
            descriptor.catalog.digest
        )
    );
}

fn serde_json_shape(descriptor: &VersionDescriptor) -> String {
    serde_json::to_string(descriptor).unwrap()
}

#[test]
fn revisions_and_trees_can_cross_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<Tree>();
    shared::<ProgramRevision>();
    shared::<ReplyRevision>();
}

#[cfg(feature = "traces")]
#[test]
fn every_committed_golden_trace_replays_byte_for_byte() {
    let traces = occipital_lang::run_golden_traces();
    assert_eq!(traces.len(), 12);
    for trace in traces {
        assert!(
            trace.passed(),
            "{} differs from its committed golden",
            trace.name
        );
    }
}
