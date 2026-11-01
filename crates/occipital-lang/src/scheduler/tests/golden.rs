use super::*;
use std::fmt::Write;

const SOURCE: &str = "export const q = Query(\"read\", {}, 0)\n\n{q.value} {q.value}\n\n<Button label=\"Refresh\" action={Refresh(q)} />";

fn record(trace: &mut String, label: &str, session: &Session, output: &Output) {
    writeln!(
        trace,
        "{label}: evaluations={} fuel={} committed={:?} rejected={:?}",
        output.evaluations,
        output.fuel,
        session.committed.as_ref().map(|c| c.candidate.revision),
        output.rejected
    )
    .unwrap();
    for effect in &output.effects {
        match effect {
            Effect::Issue {
                site,
                identity,
                epoch,
            } => writeln!(
                trace,
                "issue {site} {} {} {}:{}",
                identity.operation, identity.arguments, epoch.instance, epoch.request
            )
            .unwrap(),
            other => writeln!(trace, "effect {other:?}").unwrap(),
        }
    }
    if let Some(candidate) = &output.candidate {
        writeln!(
            trace,
            "revision={} layout={:?} loading={} controls={:?}\n{:?}",
            candidate.revision,
            candidate.layout,
            candidate.loading,
            candidate.controls.keys().collect::<Vec<_>>(),
            candidate.tree
        )
        .unwrap();
    }
}

fn replay() -> String {
    let mut session = fixture(SOURCE);
    let mut trace = format!("source={SOURCE:?}\n");
    let initial = load(&mut session);
    let first = issues(&initial).remove(0);
    writeln!(trace, "identity={:?}", first.1).unwrap();
    record(&mut trace, "load", &session, &initial);
    let root = initial.candidate.unwrap();
    let out = session.batch(vec![measure(&root)]);
    record(&mut trace, "measure-root", &session, &out);
    let ready = session.batch(vec![answer(&first, Ok("7"))]);
    record(&mut trace, "answer-1", &session, &ready);
    let frame = ready.candidate.unwrap();
    let out = session.batch(vec![measure(&frame)]);
    record(&mut trace, "measure-ready", &session, &out);
    let clock = session.batch(vec![Event::Clock {
        now: Decimal::from_integer(3).unwrap(),
        zone: text("UTC"),
    }]);
    record(&mut trace, "same-identity-demand", &session, &clock);
    let obsolete = clock.candidate.unwrap();
    let refreshed = session.batch(vec![Event::Activate(activation(&frame, 1))]);
    record(&mut trace, "activate-committed", &session, &refreshed);
    let second = issues(&refreshed).remove(0);
    assert_eq!(first.1, second.1);
    assert_eq!(second.2.request, 2);
    let out = session.batch(vec![answer(&first, Ok("99"))]);
    record(&mut trace, "late-epoch", &session, &out);
    let answer = session.batch(vec![answer(&second, Ok("8"))]);
    record(&mut trace, "answer-2", &session, &answer);
    assert_field(&session, "q", "value", "8");
    let out = session.batch(vec![measure(&obsolete)]);
    record(&mut trace, "obsolete-measure", &session, &out);
    let out = session.batch(vec![measure(answer.candidate.as_ref().unwrap())]);
    record(&mut trace, "measure-current", &session, &out);
    let out = session.batch(vec![Event::Activate(activation(&frame, 2))]);
    record(&mut trace, "stale-control", &session, &out);
    assert_eq!(out.rejected, [Rejected::Stale]);
    trace
}

#[test]
fn frozen_query_trace_replays_byte_for_byte_twice() {
    let actual = replay();
    assert_eq!(actual, replay());
    let expected = include_str!("golden/query-lifecycle.trace");
    if actual != expected {
        panic!("golden trace mismatch:\n{actual}");
    }
}
