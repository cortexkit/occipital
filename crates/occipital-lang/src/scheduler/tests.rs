use super::identity::{Authority, ReadVersion};
use super::support::*;
use super::*;
use crate::{CheckContext, Operation, Profile, Schema, ValueKind};

mod accounts;
mod boundaries;
mod golden;
mod session;
mod streams;

fn fixture(source: &str) -> Session {
    let reads = reads();
    let mut context = CheckContext::new(Profile::ShowUi);
    context.registry = reads.registry.clone();
    let program = crate::check::check(source.as_bytes(), &context).unwrap();
    assert!(
        !program
            .diagnostics
            .iter()
            .any(|d| d.severity() == crate::Severity::Error),
        "{:?}",
        program.diagnostics
    );
    Session::new(
        program,
        Inputs::new(Profile::ShowUi),
        reads,
        layout(200),
        DisplayText::new("caller fallback"),
    )
}

fn load(session: &mut Session) -> Output {
    session.batch(vec![Event::Load {
        epoch: 42,
        inputs: Box::new(Inputs::new(Profile::ShowUi)),
        restored: BTreeMap::new(),
    }])
}

fn activation(candidate: &Candidate, id: u64) -> Activation {
    Activation {
        instance_epoch: 42,
        revision: candidate.revision,
        control: candidate.controls.keys().next().unwrap().clone(),
        id,
    }
}

#[test]
fn golden_committed_controls_survive_unmeasured_evaluations() {
    let source = "export let $n = 0\n\n<Button label=\"Add\" action={Set($n, $n + 1)} />";
    let mut session = fixture(source);
    let first = load(&mut session).candidate.unwrap();
    let committed = session.batch(vec![measure(&first)]);
    assert_eq!(committed.evaluations, 0);
    assert_eq!(
        committed.committed.unwrap().height.unwrap().to_string(),
        "60"
    );
    let action = activation(&first, 1);
    let second = session
        .batch(vec![Event::Activate(action.clone())])
        .candidate
        .unwrap();
    assert_eq!(session.runtime.state["n"].canonical_json().unwrap(), "1");
    let third = session
        .batch(vec![Event::Activate(Activation {
            id: 2,
            ..action.clone()
        })])
        .candidate
        .unwrap();
    // The second activation still uses n=0, frozen in the committed first tree.
    assert_eq!(session.runtime.state["n"].canonical_json().unwrap(), "1");
    assert_eq!(
        session
            .batch(vec![measure(&second)])
            .committed
            .map(|c| c.candidate.revision),
        None
    );
    let accepted = session.batch(vec![measure(&third)]);
    assert_eq!(accepted.evaluations, 0);
    assert_eq!(accepted.committed.unwrap().candidate.revision, 3);
    assert_eq!(
        session.batch(vec![Event::Activate(action)]).rejected,
        [Rejected::Stale]
    );
    assert_eq!(
        session
            .batch(vec![Event::Activate(activation(&third, 1))])
            .rejected,
        [Rejected::Repeated]
    );
}

#[test]
fn golden_first_answer_loading_root_and_transitive_dependency_wait() {
    let mut session = fixture("export const a = Query(\"read\", {id: 1}, {id: null})\nexport const b = Query(\"read\", {id: a.value.id ?? 7}, null)\n\n{b.value}");
    let initial = load(&mut session);
    assert_eq!(initial.evaluations, 2);
    let requests = issues(&initial);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "a");
    let root = initial.candidate.unwrap();
    assert!(root.loading);
    assert!(
        matches!(&root.tree.blocks[0], TreeNode::Component { name, .. } if name.as_str() == "LoadingRoot")
    );
    assert!(
        session
            .batch(vec![measure(&root)])
            .committed
            .unwrap()
            .candidate
            .loading
    );
    let middle = session.batch(vec![answer(&requests[0], Ok("{\"id\":9}"))]);
    assert_eq!(middle.evaluations, 1);
    let next = issues(&middle);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].1.arguments, "{\"id\":9}");
    assert!(middle.candidate.unwrap().loading);
    assert!(!middle
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Arm(Timer::FirstAnswer(_), _))));
    let end = session.batch(vec![answer(&next[0], Ok("12"))]);
    assert!(!end.candidate.unwrap().loading);
    assert!(end
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Stop(Timer::FirstAnswer(42)))));
    assert_field(&session, "b", "value", "12");
}

#[test]
fn first_wait_expires_only_on_matching_five_second_event() {
    let mut session = fixture("export const q = Query(\"read\", {}, 8)\n\n{q.value}");
    let initial = load(&mut session);
    assert!(initial
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Arm(Timer::FirstAnswer(42), n) if n.to_string() == "5")));
    let root = initial.candidate.unwrap();
    session.batch(vec![measure(&root)]);
    assert_eq!(
        session
            .batch(vec![Event::Timer(Timer::FirstAnswer(41))])
            .evaluations,
        0
    );
    assert!(session.committed.as_ref().unwrap().candidate.loading);
    let expired = session.batch(vec![Event::Timer(Timer::FirstAnswer(42))]);
    assert!(!expired.candidate.as_ref().unwrap().loading);
    assert_field(&session, "q", "value", "8");
    session.batch(vec![measure(expired.candidate.as_ref().unwrap())]);
    assert!(!session.committed.as_ref().unwrap().candidate.loading);
    assert_eq!(
        session
            .batch(vec![Event::Timer(Timer::FirstAnswer(42))])
            .evaluations,
        0
    );
}

#[test]
fn one_cycle_applies_all_actions_and_refreshes_only_the_final_identity() {
    let mut session = fixture("export let $n = 1\nexport const q = Query(\"read\", {id: $n}, 0)\n\n{q.value}\n\n<Button label=\"Go\" action={Action([Set($n, 2), Refresh(q), Set($n, 3)])} />");
    let initial = load(&mut session);
    let request = issues(&initial).remove(0);
    let settled = session
        .batch(vec![answer(&request, Ok("10"))])
        .candidate
        .unwrap();
    session.batch(vec![measure(&settled)]);
    let action = activation(&settled, 1);
    let out = session.batch(vec![
        Event::Activate(action.clone()),
        Event::Activate(Activation { id: 2, ..action }),
    ]);
    assert_eq!(out.evaluations, 1);
    let requests = issues(&out);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1.arguments, "{\"id\":3}");
    assert_field(&session, "q", "status", "\"loading\"");
    assert_field(&session, "q", "value", "10");
    assert_field(&session, "q", "stale", "true");
}

#[test]
fn golden_late_epochs_same_identity_removal_and_redemand() {
    let mut session = fixture("export let $show = true\nexport const q = Query(\"read\", {}, 0)\n\n{$show ? q.value : 9}\n\n<Button label=\"Toggle\" action={Set($show, !$show)} />");
    let initial = load(&mut session);
    let old = issues(&initial).remove(0);
    let frame = session
        .batch(vec![Event::Timer(Timer::FirstAnswer(42))])
        .candidate
        .unwrap();
    session.batch(vec![measure(&frame)]);
    let hide = session.batch(vec![Event::Activate(activation(&frame, 1))]);
    assert!(hide.effects.iter().any(|e| matches!(e, Effect::Cancel { site, identity, epoch } if site == "q" && *identity == old.1 && *epoch == old.2)));
    let hidden = hide.candidate.unwrap();
    session.batch(vec![measure(&hidden)]);
    let shown = session.batch(vec![Event::Activate(activation(&hidden, 2))]);
    let new = issues(&shown).remove(0);
    assert_eq!(old.1, new.1);
    assert_eq!(new.2.request, 2);
    assert_eq!(session.batch(vec![answer(&old, Ok("99"))]).evaluations, 0);
    assert_field(&session, "q", "value", "0");
    assert_eq!(session.batch(vec![answer(&new, Ok("3"))]).evaluations, 1);
    assert_field(&session, "q", "value", "3");
    assert_eq!(session.batch(vec![answer(&new, Ok("4"))]).evaluations, 0);
}

#[test]
fn source_order_four_in_flight_dedup_and_superseded_queue() {
    let mut source = "export let $id = 5\n".to_owned();
    for i in 0..6 {
        source += &format!(
            "export const q{i} = Query(\"read\", {{id: {}}}, 0)\n",
            if i == 5 { "$id".into() } else { i.to_string() }
        );
    }
    source += "export const twin = Query(\"read\", {id: 0}, 0)\n\n{[q5.value, q4.value, q3.value, q2.value, q1.value, q0.value, twin.value]}\n\n<Button label=\"Change\" action={Set($id, 8)} />";
    let mut session = fixture(&source);
    let initial = load(&mut session);
    let issued = issues(&initial);
    assert_eq!(
        issued.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        ["q0", "q1", "q2", "q3"]
    );
    assert_eq!(session.queries.shared.len(), 6);
    let frame = session
        .batch(vec![Event::Timer(Timer::FirstAnswer(42))])
        .candidate
        .unwrap();
    session.batch(vec![measure(&frame)]);
    let change = session.batch(vec![Event::Activate(activation(&frame, 1))]);
    assert!(issues(&change).is_empty());
    let freed = session.batch(vec![
        answer(&issued[0], Ok("5")),
        answer(&issued[1], Ok("6")),
    ]);
    assert_eq!(freed.evaluations, 1);
    let next = issues(&freed);
    assert_eq!(
        next.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        ["q4", "q5"]
    );
    assert_eq!(next[1].1.arguments, "{\"id\":8}");
    assert_field(&session, "twin", "value", "5");
    assert_eq!(
        session
            .queries
            .shared
            .values()
            .filter(|q| q.request.is_some())
            .count(),
        4
    );
}

#[test]
fn compatible_subscribers_cancel_only_at_zero_and_refresh_together() {
    let mut session = fixture("export let $showA = true\nexport let $showB = true\nexport const a = Query(\"read\", {}, 0)\nexport const b = Query(\"read\", {}, 0)\n\n{$showA ? a.value : 0} {$showB ? b.value : 0}\n\n<Button label=\"A\" action={Set($showA, false)} />\n\n<Button label=\"B\" action={Set($showB, false)} />");
    let initial = load(&mut session);
    let request = issues(&initial).remove(0);
    let frame = session
        .batch(vec![Event::Timer(Timer::FirstAnswer(42))])
        .candidate
        .unwrap();
    session.batch(vec![measure(&frame)]);
    let hide_a = session.batch(vec![Event::Activate(activation(&frame, 1))]);
    assert!(!hide_a
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Cancel { .. })));
    // The broker still answers the original representative after it leaves.
    session.batch(vec![answer(&request, Ok("7"))]);
    assert_field(&session, "b", "value", "7");
    let mut refresh = BTreeSet::new();
    refresh.insert("b".into());
    let mut effects = vec![];
    session.queries.finalize(&refresh, &mut effects);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Issue { .. }))
            .count(),
        1
    );
    session.queries.begin();
    session.queries.finalize(&BTreeSet::new(), &mut effects);
    assert!(effects.iter().any(|e| matches!(e, Effect::Cancel { .. })));
}

#[test]
fn measurement_only_batches_never_evaluate_and_layout_keys_must_match() {
    let mut session = fixture("hello");
    let old = load(&mut session).candidate.unwrap();
    let new = session
        .batch(vec![Event::Layout(layout(400))])
        .candidate
        .unwrap();
    let out = session.batch(vec![measure(&old), measure(&old)]);
    assert_eq!(out.evaluations, 0);
    assert!(out.candidate.is_none() && out.committed.is_none());
    assert!(session.committed.is_none());
    let out = session.batch(vec![measure(&new)]);
    assert_eq!(out.evaluations, 0);
    assert!(out.candidate.is_none());
    assert_eq!(out.committed.unwrap().candidate.layout, layout(400));
}

#[test]
fn timeout_backoff_success_and_explicit_refresh() {
    let mut session = fixture("export const q = Query(\"read\", {}, 0, 2)\n\n{q.value}\n\n<Button label=\"Refresh\" action={Refresh(q)} />");
    let initial = load(&mut session);
    let mut request = issues(&initial).remove(0);
    assert!(initial.effects.iter().any(|e| matches!(e, Effect::Arm(Timer::Timeout(epoch), n) if *epoch == request.2 && n.to_string() == "30")));
    let timeout = session.batch(vec![Event::Timer(Timer::Timeout(request.2))]);
    assert_field(&session, "q", "status", "\"error\"");
    assert_field(&session, "q", "error", "\"timed out\"");
    assert_field(&session, "q", "stale", "false");
    assert_field(&session, "q", "value", "0");
    let mut timer = timeout
        .effects
        .iter()
        .find_map(|e| {
            if let Effect::Arm(t @ Timer::Refresh(..), delay) = e {
                assert_eq!(delay.to_string(), "4");
                Some(t.clone())
            } else {
                None
            }
        })
        .unwrap();
    for delay in [8, 16, 32, 64, 128, 256, 300, 300] {
        let issued = session.batch(vec![Event::Timer(timer)]);
        request = issues(&issued).remove(0);
        assert_field(&session, "q", "status", "\"error\"");
        let failed = session.batch(vec![answer(&request, Err("bad"))]);
        timer = failed
            .effects
            .iter()
            .find_map(|e| {
                if let Effect::Arm(t @ Timer::Refresh(..), n) = e {
                    assert_eq!(n.to_string(), delay.to_string());
                    Some(t.clone())
                } else {
                    None
                }
            })
            .unwrap();
    }
    let next = session.batch(vec![Event::Timer(timer.clone())]);
    request = issues(&next).remove(0);
    let ready = session.batch(vec![answer(&request, Ok("9"))]);
    assert_field(&session, "q", "status", "\"ready\"");
    assert_field(&session, "q", "stale", "false");
    assert!(ready
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Arm(Timer::Refresh(..), n) if n.to_string() == "2")));
    let candidate = ready.candidate.unwrap();
    session.batch(vec![measure(&candidate)]);
    let refresh = session.batch(vec![Event::Activate(activation(&candidate, 1))]);
    let refreshed = issues(&refresh).remove(0);
    assert_field(&session, "q", "status", "\"ready\"");
    assert_field(&session, "q", "value", "9");
    let failed = session.batch(vec![answer(&refreshed, Err("oops"))]);
    assert_field(&session, "q", "stale", "true");
    assert_field(&session, "q", "value", "9");
    assert!(failed
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Arm(Timer::Refresh(..), n) if n.to_string() == "4")));
    assert_eq!(session.batch(vec![Event::Timer(timer)]).evaluations, 0);
}

#[test]
fn no_refresh_policy_means_no_answer_timer_and_large_base_is_not_shortened() {
    for (suffix, expected) in [("", None), (", 600", Some("600"))] {
        let mut session = fixture(&format!(
            "export const q = Query(\"read\", {{}}, 0{suffix})\n\n{{q.value}}"
        ));
        let request = issues(&load(&mut session)).remove(0);
        let out = session.batch(vec![answer(&request, Err("bad"))]);
        let delay = out.effects.iter().find_map(|e| {
            if let Effect::Arm(Timer::Refresh(..), n) = e {
                Some(n.to_string())
            } else {
                None
            }
        });
        assert_eq!(delay.as_deref(), expected);
        assert_field(&session, "q", "value", "0");
    }
}

#[test]
fn restoration_drops_bad_types_and_never_restores_sites_or_runs_actions() {
    let mut session = fixture("export let $n = 1\nexport let $name = \"start\"\nexport const q = Query(\"read\", {id: $n}, 0)\n\n{q.value}\n\n<Button label=\"Go\" action={Set($n, 9)} />");
    let old = issues(&load(&mut session)).remove(0);
    let out = session.batch(vec![Event::Load {
        epoch: 43,
        inputs: Box::new(Inputs::new(Profile::ShowUi)),
        restored: [
            ("n".into(), text("wrong")),
            ("name".into(), text("saved")),
            ("unknown".into(), number(7)),
        ]
        .into(),
    }]);
    assert_eq!(session.runtime.state["n"].canonical_json().unwrap(), "1");
    assert_eq!(
        session.runtime.state["name"].canonical_json().unwrap(),
        "\"saved\""
    );
    assert!(!session.runtime.state.contains_key("unknown"));
    let new = issues(&out).remove(0);
    assert_eq!(
        new.2,
        Epoch {
            instance: 43,
            request: 1
        }
    );
    assert!(!out.effects.iter().any(|e| matches!(e, Effect::Handoff(..))));
    assert_eq!(session.batch(vec![answer(&old, Ok("99"))]).evaluations, 0);
    assert_field(&session, "q", "value", "0");
    let repeat = session.batch(vec![Event::Load {
        epoch: 42,
        inputs: Box::new(Inputs::new(Profile::ShowUi)),
        restored: BTreeMap::new(),
    }]);
    assert_eq!(repeat.rejected, [Rejected::Stale]);
}

#[test]
fn clock_values_arrive_only_by_events_and_replay_is_identical() {
    fn run() -> String {
        let mut session = fixture("{now} {zone}");
        let first = load(&mut session);
        let first_tree = format!("{:?}", first.candidate.unwrap().tree);
        let updated = session.batch(vec![Event::Clock {
            now: Decimal::from_integer(123).unwrap(),
            zone: text("+02:00"),
        }]);
        assert_eq!(updated.evaluations, 1);
        assert_eq!(session.inputs.now.to_string(), "123");
        format!("{first_tree}\n{updated:?}")
    }
    assert_eq!(run(), run());
}

#[test]
fn abort_after_demand_dispatches_nothing_and_cancels_existing_reads() {
    let mut session = fixture("export const q = Query(\"read\", {}, 0)\n\n{q.value} {1 + 2}");
    let request = issues(&load(&mut session)).remove(0);
    session.inputs.budget = 6;
    let out = session.batch(vec![Event::Clock {
        now: Decimal::ZERO,
        zone: text("UTC"),
    }]);
    assert_eq!(out.evaluations, 1);
    assert!(issues(&out).is_empty());
    assert!(!out
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Cancel { .. })));
    let committed = session.batch(vec![measure(out.candidate.as_ref().unwrap())]);
    assert!(committed
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Cancel { epoch, .. } if *epoch == request.2)));
    assert!(matches!(
        &committed.committed.unwrap().candidate.tree.blocks[0],
        TreeNode::Component { name, .. } if name.as_str() == "ProgramFallback"
    ));
    assert_eq!(
        session
            .batch(vec![Event::Clock {
                now: Decimal::ZERO,
                zone: text("UTC")
            }])
            .evaluations,
        0
    );
}

#[test]
fn refresh_resets_errors_and_replaces_in_flight_epochs_without_changing_status() {
    let mut session = fixture("export const q = Query(\"read\", {}, 0, 2)\n\n{q.value}\n\n<Button label=\"Refresh\" action={Refresh(q)} />");
    let first = issues(&load(&mut session)).remove(0);
    let failed = session.batch(vec![answer(&first, Err("first"))]);
    let frame = failed.candidate.unwrap();
    session.batch(vec![measure(&frame)]);
    let second = issues(&session.batch(vec![Event::Activate(activation(&frame, 1))])).remove(0);
    let replaced = session.batch(vec![Event::Activate(activation(&frame, 2))]);
    assert!(replaced
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Cancel { epoch, .. } if *epoch == second.2)));
    let third = issues(&replaced).remove(0);
    assert_eq!(third.2.request, 3);
    assert_field(&session, "q", "status", "\"error\"");
    assert_field(&session, "q", "error", "\"first\"");
    assert_eq!(
        session.batch(vec![answer(&second, Ok("99"))]).evaluations,
        0
    );
    let failed = session.batch(vec![answer(&third, Err("again"))]);
    assert!(failed
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Arm(Timer::Refresh(..), n) if n.to_string() == "4")));
}

#[test]
fn changed_identity_error_uses_default_not_previous_identity_answer() {
    let mut session = fixture("export let $id = 1\nexport const q = Query(\"read\", {id: $id}, 0)\n\n{q.value}\n\n<Button label=\"Next\" action={Set($id, 2)} />");
    let first = issues(&load(&mut session)).remove(0);
    let ready = session
        .batch(vec![answer(&first, Ok("9"))])
        .candidate
        .unwrap();
    session.batch(vec![measure(&ready)]);
    let second = issues(&session.batch(vec![Event::Activate(activation(&ready, 1))])).remove(0);
    assert_field(&session, "q", "value", "9");
    assert_field(&session, "q", "stale", "true");
    session.batch(vec![answer(&second, Err("no answer"))]);
    assert_field(&session, "q", "value", "0");
    assert_field(&session, "q", "stale", "false");
}
