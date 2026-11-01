use super::*;
use crate::geometry::{Metrics, TextStyle};

fn d(value: i128) -> Decimal {
    Decimal::from_integer(value).unwrap()
}

struct Fixed;
impl Metrics for Fixed {
    fn l(&self, style: TextStyle) -> Decimal {
        d(match style {
            TextStyle::Headline => 18,
            TextStyle::Subheadline => 14,
            _ => 20,
        })
    }
    fn t(&self, text: &str, _: TextStyle, width: Decimal, max: Option<u32>) -> Decimal {
        d(text.chars().count() as i128 * 6)
            .checked_div(width)
            .unwrap()
            .ceil()
            .unwrap()
            .min(d(max.unwrap_or(u32::MAX) as i128))
            .checked_mul(d(20))
            .unwrap()
    }
    fn tw(&self, text: &str, _: TextStyle) -> Decimal {
        d(text.chars().count() as i128 * 6)
    }
}

struct Large;
impl Metrics for Large {
    fn l(&self, style: TextStyle) -> Decimal {
        Fixed.l(style).checked_mul(d(2)).unwrap()
    }
    fn t(&self, text: &str, style: TextStyle, width: Decimal, max: Option<u32>) -> Decimal {
        Fixed.t(text, style, width, max).checked_mul(d(2)).unwrap()
    }
    fn tw(&self, text: &str, style: TextStyle) -> Decimal {
        Fixed.tw(text, style)
    }
}

fn measured(candidate: &Candidate) -> Measurement {
    geometry::measure(&candidate.measure_revision(), &candidate.layout, &Fixed).unwrap()
}

#[test]
fn typed_layout_key_rejects_every_obsolete_completion() {
    let mut s = fixture("<Progress value={0.5} />");
    let narrow = load(&mut s).candidate.unwrap();
    let old_measure = measured(&narrow);
    let wide = s.batch(vec![Event::Layout(layout(400))]).candidate.unwrap();
    assert_eq!(narrow.revision, wide.revision);
    assert_eq!(narrow.tree, wide.tree);
    let current = measured(&wide);
    for key in [
        LayoutKey {
            instance_epoch: 41,
            ..current.key.clone()
        },
        LayoutKey {
            revision: 0,
            ..current.key.clone()
        },
        LayoutKey {
            state_hash: "old".into(),
            ..current.key.clone()
        },
        LayoutKey {
            content_size_category: "large".into(),
            ..current.key.clone()
        },
        LayoutKey {
            catalog_digest: "old".into(),
            ..current.key.clone()
        },
        LayoutKey {
            layout_version: "old".into(),
            ..current.key.clone()
        },
        LayoutKey {
            network: geometry::Network::No,
            ..current.key.clone()
        },
        LayoutKey {
            placement: Placement::Page,
            ..current.key.clone()
        },
    ] {
        let out = s.batch(vec![Event::MeasureComplete(Measurement {
            key,
            ..current.clone()
        })]);
        assert!(out.committed.is_none());
        assert!(out.effects.is_empty());
    }
    let out = s.batch(vec![Event::MeasureComplete(current)]);
    assert_eq!(out.evaluations, 0);
    assert_eq!(out.committed.unwrap().candidate.layout.width, d(400));
    let committed = s.committed.clone();
    let obsolete = s.batch(vec![Event::MeasureComplete(old_measure)]);
    assert!(obsolete.committed.is_none());
    assert!(obsolete.effects.is_empty());
    assert_eq!(
        s.committed.as_ref().unwrap().candidate.tree,
        committed.unwrap().candidate.tree
    );
    assert_eq!(s.committed.as_ref().unwrap().candidate.layout.width, d(400));
}

#[test]
fn defaults_revision_drives_row_frame_loading_clipping_and_context_changes() {
    let source = format!("export const q = Query(\"read\", {{}}, \"{}\")\n\n<TextContent text={{q.value}} maxLines={{100}} />", "x".repeat(320));
    let mut s = fixture(&source);
    let load = load(&mut s);
    let requests = issues(&load);
    assert_eq!(requests.len(), 1);
    let defaults = load.defaults.unwrap();
    let loading = load.candidate.unwrap();
    assert_eq!(defaults.revision, 0);
    assert_ne!(defaults.revision, loading.revision);
    assert!(!defaults.loading);
    assert!(defaults.first.is_none());
    assert_eq!(loading.first.as_ref().unwrap().tree, defaults.tree);
    assert_eq!(measured(&defaults).height, Height::Known(d(200)));
    let mut page = loading.layout.clone();
    page.placement = Placement::Page;
    let page = geometry::measure(&loading.measure_revision(), &page, &Fixed).unwrap();
    assert_eq!(page.height, Height::Known(d(60)));
    assert!(page.row_frame.is_none());
    let m = measured(&loading);
    assert_eq!(m.height, Height::Known(d(60)));
    assert_eq!(
        m.row_frame,
        Some(RowFrame {
            baseline: Height::Known(d(200)),
            height: Height::Known(d(244)),
            clipped: Some(false)
        })
    );
    let commit = s.batch(vec![Event::MeasureComplete(m)]).committed.unwrap();
    assert_eq!(commit.height, Some(d(244)));
    assert_eq!(commit.content_height, Height::Known(d(60)));
    assert!(!commit.fallback);
    let next = s
        .batch(vec![answer(
            &requests[0],
            Ok(&format!("\"{}\"", "x".repeat(3200))),
        )])
        .candidate
        .unwrap();
    let m = measured(&next);
    assert_eq!(m.height, Height::Known(d(1920)));
    assert_eq!(m.row_frame.as_ref().unwrap().height, Height::Known(d(244)));
    assert_eq!(m.row_frame.as_ref().unwrap().clipped, Some(true));
    let commit = s.batch(vec![Event::MeasureComplete(m)]).committed.unwrap();
    assert_eq!(commit.height, Some(d(244)));
    assert_eq!(commit.row_frame.unwrap().baseline, Height::Known(d(200)));
    let wide = s.batch(vec![Event::Layout(layout(400))]);
    assert_eq!(wide.evaluations, 0);
    let m = measured(wide.candidate.as_ref().unwrap());
    assert_eq!(
        m.row_frame.as_ref().unwrap().baseline,
        Height::Known(d(100))
    );
    assert_eq!(m.row_frame.as_ref().unwrap().height, Height::Known(d(144)));
    assert_eq!(
        s.batch(vec![Event::MeasureComplete(m)])
            .committed
            .unwrap()
            .height,
        Some(d(144))
    );
    let mut large = layout(400);
    large.content_size_category = "large".into();
    let large = s.batch(vec![Event::Layout(large)]).candidate.unwrap();
    let m = geometry::measure(&large.measure_revision(), &large.layout, &Large).unwrap();
    assert_eq!(
        m.row_frame.as_ref().unwrap().baseline,
        Height::Known(d(200))
    );
    assert_eq!(m.row_frame.as_ref().unwrap().height, Height::Known(d(244)));
}

fn all_sites() -> Session {
    let source = "export let $n = 0\nexport let $label = \"declared\"\nexport const q = Query(\"read\", {n:$n}, 0)\nexport const w = Watch(\"events\", {})\nexport const p = PagedList(\"pages\", {})\n\n<TextContent text={Text(q.value)} />\n\n<TextContent text={w.status} />\n\n<List more={p} />\n\n<Button label=\"Change\" action={Action([Set($n, 1), Set($label, \"changed\")])} />";
    let mut reads = reads();
    for (name, operation) in [
        (
            "events",
            Operation::Watch {
                arguments: Schema::default(),
                item: Schema::default(),
                resume: true,
                replay: false,
            },
        ),
        (
            "pages",
            Operation::Paged {
                arguments: Schema::default(),
                page: Schema::default(),
                item_key: None,
            },
        ),
    ] {
        reads.registry.0.insert(name.into(), operation);
        reads.versions.insert(
            name.into(),
            super::super::identity::ReadVersion {
                operation: "1".into(),
                schema: "1".into(),
            },
        );
    }
    let mut context = CheckContext::new(Profile::ShowUi);
    context.registry = reads.registry.clone();
    let program = crate::check::check(source.as_bytes(), &context).unwrap();
    assert!(
        program
            .diagnostics
            .iter()
            .all(|d| d.severity() != crate::Severity::Error),
        "{:?}",
        program.diagnostics
    );
    Session::new(
        program,
        Inputs::new(Profile::ShowUi),
        reads,
        layout(200),
        DisplayText::new("the caller's fallback"),
    )
}

fn fallback_text(committed: &Committed) -> &str {
    let [TreeNode::Component {
        name,
        props,
        children,
    }] = committed.candidate.tree.blocks.as_slice()
    else {
        panic!("fallback tree")
    };
    assert_eq!(name.as_str(), "ProgramFallback");
    assert!(children.is_empty());
    let [(key, TreeValue::String(text))] = props.as_slice() else {
        panic!("fallback prop")
    };
    assert_eq!(key.as_str(), "fallback");
    text.as_str()
}

fn abort_trace() -> String {
    let mut s = all_sites();
    let initial = load(&mut s);
    assert_eq!(issues(&initial).len(), 1);
    assert!(initial
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Subscribe { .. })));
    assert!(initial
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Page { .. })));
    let ready = s
        .batch(vec![Event::Timer(Timer::FirstAnswer(42))])
        .candidate
        .unwrap();
    s.batch(vec![measure(&ready)]);
    let out = s.batch(vec![Event::Activate(activation(&ready, 1))]);
    let cycle_request = issues(&out).remove(0);
    assert_eq!(cycle_request.1.arguments, "{\"n\":1}");
    assert_eq!(s.runtime.state["n"].canonical_json().unwrap(), "1");
    assert_eq!(
        s.runtime.state["label"].canonical_json().unwrap(),
        "\"changed\""
    );
    let candidate = out.candidate.unwrap();
    assert_ne!(candidate.state_hash, ready.state_hash);
    let Event::MeasureComplete(mut m) = measure(&candidate) else {
        unreachable!()
    };
    m.reservation = ReservationOutcome::Abort(eval::Abort::Fuel);
    let aborted = s.batch(vec![Event::MeasureComplete(m)]);
    assert_eq!(aborted.abort, Some(eval::Abort::Fuel));
    assert_eq!(aborted.evaluations, 0);
    let committed = aborted.committed.as_ref().unwrap();
    assert!(committed.fallback);
    assert_eq!(fallback_text(committed), "the caller's fallback");
    assert_eq!(committed.height, None);
    assert!(committed.candidate.controls.is_empty());
    let cancelled: Vec<_> = aborted
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Cancel { site, epoch, .. } => Some((site.as_str(), *epoch)),
            _ => None,
        })
        .collect();
    assert_eq!(
        cancelled,
        [
            (
                "w",
                Epoch {
                    instance: 42,
                    request: 2
                }
            ),
            (
                "p",
                Epoch {
                    instance: 42,
                    request: 3
                }
            ),
            ("q", cycle_request.2)
        ]
    );
    assert_eq!(
        aborted.order.first(),
        Some(&OutputStep::Commit(candidate.revision))
    );
    assert!(s.queries.sites.is_empty());
    assert!(s.queries.shared.is_empty());
    let frozen = s.batch(vec![
        Event::Clock {
            now: d(2),
            zone: text("UTC"),
        },
        Event::Activate(activation(&ready, 2)),
        answer(&cycle_request, Ok("8")),
    ]);
    assert_eq!(frozen.evaluations, 0);
    assert!(frozen.candidate.is_none());
    assert!(frozen.effects.is_empty());
    assert_eq!(frozen.rejected, [Rejected::Stale]);
    assert_eq!(s.runtime.state["n"].canonical_json().unwrap(), "1");
    assert_eq!(
        s.runtime.state["label"].canonical_json().unwrap(),
        "\"changed\""
    );

    let mut trace = format!(
        "step4 query.issue q {} {}:{}\nmeasure_complete E_FUEL\ncommit fallback {:?}\n",
        cycle_request.1.arguments,
        cycle_request.2.instance,
        cycle_request.2.request,
        fallback_text(committed)
    );
    for step in &aborted.order {
        if let OutputStep::Effect(index) = step {
            match &aborted.effects[*index] {
                Effect::Cancel { site, epoch, .. } => trace.push_str(&format!(
                    "cancel {site} {}:{}\n",
                    epoch.instance, epoch.request
                )),
                Effect::Stop(timer) => trace.push_str(&format!("stop {timer:?}\n")),
                other => panic!("unexpected abort effect {other:?}"),
            }
        }
    }
    let reset = s.batch(vec![Event::Load {
        epoch: 43,
        inputs: Box::new(Inputs::new(Profile::ShowUi)),
        restored: BTreeMap::new(),
    }]);
    assert!(reset.abort.is_none());
    assert_eq!(s.runtime.state["n"].canonical_json().unwrap(), "0");
    assert_eq!(
        s.runtime.state["label"].canonical_json().unwrap(),
        "\"declared\""
    );
    assert_eq!(issues(&reset)[0].1.arguments, "{\"n\":0}");
    trace.push_str("reset load state n=0 label=declared\n");
    trace
}

#[test]
fn measurement_abort_commits_caller_fallback_then_cancels_every_site_and_freezes_state() {
    let actual = abort_trace();
    assert_eq!(actual, abort_trace());
    assert_eq!(actual, include_str!("golden/measurement-abort.trace"));
}

#[test]
fn defaults_abort_cannot_silently_omit_the_row_baseline() {
    let mut s = fixture("export let $hide = false\n\n{$hide ? \"ready\" : Count(Range(0, 10001))}");
    let out = s.batch(vec![Event::Load {
        epoch: 42,
        inputs: Box::new(Inputs::new(Profile::ShowUi)),
        restored: [("hide".into(), Value::boolean(true))].into(),
    }]);
    assert_eq!(out.evaluations, 1);
    assert_eq!(out.abort, Some(eval::Abort::LimitList));
    assert!(out.defaults.is_none());
    assert_eq!(s.runtime.state["hide"].canonical_json().unwrap(), "true");
    let committed = s
        .batch(vec![measure(out.candidate.as_ref().unwrap())])
        .committed
        .unwrap();
    assert!(committed.fallback);
    assert_eq!(fallback_text(&committed), "caller fallback");
}

fn error_value() -> Value {
    Value::error(crate::ValueError::new(
        "E_TYPE",
        crate::SourceSpan {
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
        },
    ))
}

#[test]
fn state_hash_handles_combined_record_overflow_without_panicking() {
    let mut value = error_value();
    // Shared children grow logical size without allocating or encoding an
    // enormous payload. The error leaf makes ordered encoding stop immediately.
    for _ in 0..58 {
        value = Value::list(vec![value.clone(), value]).unwrap();
    }
    assert_eq!(value.logical_size(), (1u64 << 63) - 16);
    let mut state: BTreeMap<String, Value> = [
        ("a".into(), value.clone()),
        ("b".into(), value),
        ("counter".into(), Value::boolean(false)),
    ]
    .into();
    assert!(matches!(
        Value::record(
            state
                .iter()
                .map(|(name, value)| (crate::ScalarString::new(name.clone()), value.clone()))
                .collect()
        ),
        Err(crate::ValueBuildError::SizeOverflow)
    ));
    let first = state_hash(&state);
    assert_eq!(first.len(), 64);
    assert_eq!(first, state_hash(&state));
    state.insert("counter".into(), Value::boolean(true));
    assert_ne!(first, state_hash(&state));
}

#[test]
fn state_hash_preserves_field_names_order_and_encodable_values() {
    let state: BTreeMap<String, Value> = [("a".into(), text("x")), ("b".into(), text("y"))].into();
    let reversed = state
        .iter()
        .rev()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    assert_eq!(state_hash(&state), state_hash(&reversed));
    let mut changed = state.clone();
    changed.insert("b".into(), text("z"));
    assert_ne!(state_hash(&state), state_hash(&changed));
    assert_ne!(
        state_hash(&[("a".into(), text("x"))].into()),
        state_hash(&[("b".into(), text("x"))].into())
    );
    let ordered = |fields: &[&str]| {
        Value::record(
            fields
                .iter()
                .map(|key| (crate::ScalarString::new(*key), Value::null()))
                .collect(),
        )
        .unwrap()
    };
    assert_ne!(
        state_hash(&[("a".into(), ordered(&["a", "b"]))].into()),
        state_hash(&[("a".into(), ordered(&["b", "a"]))].into())
    );
    assert_ne!(
        state_hash(&BTreeMap::new()),
        state_hash(&[("a".into(), Value::missing())].into())
    );
}

#[test]
fn state_hash_handles_unencodable_error_and_unsupported_values() {
    let error: BTreeMap<String, Value> = [("a".into(), error_value())].into();
    let unsupported: BTreeMap<String, Value> =
        [("a".into(), Value::action(vec![]).unwrap())].into();
    assert!(matches!(
        error["a"].encode_ordered(crate::CodecMode::State),
        Err(crate::CodecError::ErrorInState)
    ));
    assert!(matches!(
        unsupported["a"].encode_ordered(crate::CodecMode::State),
        Err(crate::CodecError::UnsupportedKind(_))
    ));
    assert_eq!(state_hash(&error), state_hash(&error));
    assert_eq!(state_hash(&unsupported), state_hash(&unsupported));
    assert_ne!(state_hash(&error), state_hash(&unsupported));
    assert_ne!(
        state_hash(&error),
        state_hash(&[("a".into(), text("ErrorInState"))].into())
    );
}

fn abort_during_first_answer_wait() -> (Session, Output) {
    let mut session = fixture("export const q = Query(\"read\", {}, 0)\n\n{q.value} {now == 0 ? 0 : Count(Range(0, 10001))}");
    let initial = load(&mut session);
    assert_eq!(issues(&initial).len(), 1);
    assert!(session.first_wait);
    let aborted = session.batch(vec![Event::Clock {
        now: d(1),
        zone: text("UTC"),
    }]);
    assert_eq!(aborted.abort, Some(eval::Abort::LimitList));
    assert!(session.first_wait);
    assert_eq!(session.pending_abort, Some(eval::Abort::LimitList));
    (session, aborted)
}

#[test]
fn evaluation_abort_during_first_answer_wait_publishes_nonloading_fallback() {
    let (session, aborted) = abort_during_first_answer_wait();
    let candidate = aborted.candidate.unwrap();
    assert_eq!(*candidate.tree, session.fallback_tree());
    assert!(!candidate.loading);
    assert!(aborted.committed.is_none());
}

#[test]
fn layout_after_committed_abort_changes_nothing_and_emits_no_candidate() {
    let (mut session, aborted) = abort_during_first_answer_wait();
    let committed = session
        .batch(vec![measure(aborted.candidate.as_ref().unwrap())])
        .committed
        .unwrap();
    assert!(committed.fallback);
    assert!(session.aborted);
    assert_eq!(session.pending_abort, None);
    assert!(session.candidate.is_none());
    let old_layout = session.layout.clone();
    let out = session.batch(vec![Event::Layout(layout(400))]);
    assert_eq!(session.layout, old_layout);
    assert!(session.candidate.is_none());
    assert!(out.candidate.is_none());
    assert!(out.committed.is_none());
    assert!(out.effects.is_empty());
    assert_eq!(out.evaluations, 0);
    let retained = session.committed.as_ref().unwrap();
    assert_eq!(retained.candidate.tree, committed.candidate.tree);
    assert_eq!(retained.candidate.layout, committed.candidate.layout);
    assert_eq!(retained.candidate.revision, committed.candidate.revision);
}

#[test]
fn state_hash_length_prefixes_distinguish_identical_unframed_field_streams() {
    let two: BTreeMap<String, Value> =
        [("a".into(), Value::null()), ("z".into(), Value::null())].into();
    // Folding a complete field into another field's name makes their unframed
    // bytes identical. Length framing must still distinguish the two maps.
    let one: BTreeMap<String, Value> =
        [("aordered-value\0[\"null\"]z".into(), Value::null())].into();
    let unframed = |state: &BTreeMap<String, Value>| {
        let mut bytes = Vec::new();
        for (name, value) in state {
            let encoded = value.encode_ordered(crate::CodecMode::State).unwrap();
            assert_eq!(encoded, "[\"null\"]");
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(b"ordered-value\0");
            bytes.extend_from_slice(encoded.as_bytes());
        }
        bytes
    };
    assert_eq!(
        unframed(&two),
        b"aordered-value\0[\"null\"]zordered-value\0[\"null\"]"
    );
    assert_eq!(unframed(&two), unframed(&one));
    assert_ne!(state_hash(&two), state_hash(&one));
}

#[test]
fn pending_abort_remeasures_and_commits_fallback_under_changed_layout_key() {
    let (mut session, aborted) = abort_during_first_answer_wait();
    let original = aborted.candidate.unwrap();
    let out = session.batch(vec![Event::Layout(layout(400))]);
    assert_eq!(session.layout, layout(400));
    assert_eq!(out.evaluations, 0);
    assert!(out.effects.is_empty());
    let candidate = out.candidate.unwrap();
    assert_eq!(candidate.revision, original.revision);
    assert_eq!(candidate.layout, layout(400));
    assert_eq!(session.candidate.as_ref().unwrap().layout, layout(400));
    let stale = session.batch(vec![measure(&original)]);
    assert!(stale.committed.is_none());
    assert_eq!(session.pending_abort, Some(eval::Abort::LimitList));
    let committed = session.batch(vec![measure(&candidate)]).committed.unwrap();
    assert!(committed.fallback);
    assert_eq!(committed.candidate.layout, layout(400));
    assert_eq!(*committed.candidate.tree, session.fallback_tree());
    assert_eq!(session.pending_abort, None);
}
