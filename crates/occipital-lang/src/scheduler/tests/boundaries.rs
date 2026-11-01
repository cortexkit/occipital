use super::*;
use crate::{ScalarString, SchemaType};

#[test]
fn dispatch_refuses_unverified_holder_and_wrong_serving_computer() {
    let source = "export const q = Query(\"read\", {}, 0)\n\n{q.value}";
    for (case, authority) in [
        (
            "unverified holder",
            Authority {
                verified_holder: "other-computer".into(),
                ..reads().authority
            },
        ),
        (
            "wrong serving computer",
            Authority {
                computer: "other-computer".into(),
                ..reads().authority
            },
        ),
    ] {
        let mut session = fixture(source);
        session.queries.reads.authority = authority;
        let out = load(&mut session);
        assert_eq!(out.evaluations, 2, "{case}");
        assert!(issues(&out).is_empty(), "{case} must not dispatch");
        assert_field(&session, "q", "status", "\"error\"");
        assert_field(&session, "q", "error", "\"E_SITE_ARGS\"");
        assert!(!out.candidate.unwrap().loading, "{case} must settle");
    }
    let mut verified = fixture(source);
    let out = load(&mut verified);
    assert_eq!(issues(&out).len(), 1);
    assert_field(&verified, "q", "status", "\"loading\"");
}

#[test]
fn input_rejects_previous_instance_with_current_revision_and_binding() {
    let mut session = fixture("export let $name = \"start\"\n\n<Input value={$name} />");
    let first = load(&mut session).candidate.unwrap();
    session.batch(vec![measure(&first)]);
    let current = session
        .batch(vec![Event::Load {
            epoch: 43,
            inputs: Box::new(Inputs::new(Profile::ShowUi)),
            restored: [("name".into(), text("new load"))].into(),
        }])
        .candidate
        .unwrap();
    session.batch(vec![measure(&current)]);
    let control = current.bindings.keys().next().unwrap().clone();
    let input = |instance, value| Event::Input {
        instance,
        revision: current.revision,
        control: control.clone(),
        binding: "name".into(),
        value: text(value),
    };
    // Only the instance is stale: both events name the newly committed
    // revision, control and binding, so no other rejection can mask it.
    let stale = session.batch(vec![input(42, "late input")]);
    assert_eq!(stale.rejected, [Rejected::Stale]);
    assert_eq!(stale.evaluations, 0);
    assert!(stale.candidate.is_none() && issues(&stale).is_empty());
    assert_eq!(
        session.runtime.state["name"].canonical_json().unwrap(),
        "\"new load\""
    );
    let accepted = session.batch(vec![input(43, "accepted")]);
    assert!(accepted.rejected.is_empty());
    assert_eq!(accepted.evaluations, 1);
    assert_eq!(
        session.runtime.state["name"].canonical_json().unwrap(),
        "\"accepted\""
    );
}

#[test]
fn abort_freezes_input_activation_and_refresh_until_fresh_load() {
    let mut session = fixture("export let $name = \"start\"\nexport const q = Query(\"read\", {}, 0, 2)\n\n{q.value}\n\n<Input value={$name} />\n\n<Button label=\"Change\" action={Set($name, \"activated\")} />");
    let request = issues(&load(&mut session)).remove(0);
    let ready = session.batch(vec![answer(&request, Ok("7"))]);
    let refresh = ready
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Arm(timer @ Timer::Refresh(..), _) => Some(timer.clone()),
            _ => None,
        })
        .unwrap();
    let frame = ready.candidate.unwrap();
    session.batch(vec![measure(&frame)]);
    let control = frame.bindings.keys().next().unwrap().clone();
    session.inputs.budget = 6;
    let aborted = session.batch(vec![Event::Clock {
        now: Decimal::ZERO,
        zone: text("UTC"),
    }]);
    assert_eq!(aborted.abort, Some(eval::Abort::Fuel));
    assert_eq!(
        session.committed.as_ref().unwrap().candidate.revision,
        frame.revision
    );
    // Leave the abort candidate unmeasured so these controls are otherwise
    // valid in the still-committed tree. Retirement cannot mask the freeze.
    let input = session.batch(vec![Event::Input {
        instance: 42,
        revision: frame.revision,
        control,
        binding: "name".into(),
        value: text("late input"),
    }]);
    let after_input = session.runtime.state["name"].clone();
    let activate = session.batch(vec![Event::Activate(activation(&frame, 1))]);
    let after_activate = session.runtime.state["name"].clone();
    let timer = session.batch(vec![Event::Timer(refresh)]);
    for out in [&input, &activate, &timer] {
        assert_eq!(out.evaluations, 0);
        assert!(issues(out).is_empty());
        assert!(out.candidate.is_none());
    }
    assert_eq!(input.rejected, [Rejected::Stale]);
    assert_eq!(activate.rejected, [Rejected::Stale]);
    assert!(timer.rejected.is_empty());
    assert_eq!(after_input.canonical_json().unwrap(), "\"start\"");
    assert_eq!(after_activate.canonical_json().unwrap(), "\"start\"");
    assert_eq!(
        session.runtime.state["name"].canonical_json().unwrap(),
        "\"start\""
    );

    let reset = session.batch(vec![Event::Load {
        epoch: 43,
        inputs: Box::new(Inputs::new(Profile::ShowUi)),
        restored: BTreeMap::new(),
    }]);
    assert_eq!(reset.evaluations, 2);
    assert!(reset.abort.is_none());
    let request = issues(&reset).remove(0);
    assert_eq!(
        request.2,
        Epoch {
            instance: 43,
            request: 1
        }
    );
    let ready = session.batch(vec![answer(&request, Ok("8"))]);
    let refresh = ready
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Arm(timer @ Timer::Refresh(..), _) => Some(timer.clone()),
            _ => None,
        })
        .unwrap();
    let frame = ready.candidate.unwrap();
    session.batch(vec![measure(&frame)]);
    let input = session.batch(vec![Event::Input {
        instance: 43,
        revision: frame.revision,
        control: frame.bindings.keys().next().unwrap().clone(),
        binding: "name".into(),
        value: text("fresh input"),
    }]);
    assert!(input.rejected.is_empty());
    assert_eq!(input.evaluations, 1);
    assert_eq!(
        session.runtime.state["name"].canonical_json().unwrap(),
        "\"fresh input\""
    );
    let activate = session.batch(vec![Event::Activate(Activation {
        instance_epoch: 43,
        ..activation(&frame, 1)
    })]);
    assert!(activate.rejected.is_empty());
    assert_eq!(activate.evaluations, 1);
    assert_eq!(
        session.runtime.state["name"].canonical_json().unwrap(),
        "\"activated\""
    );
    let timer = session.batch(vec![Event::Timer(refresh)]);
    assert_eq!(timer.evaluations, 1);
    assert_eq!(issues(&timer).len(), 1);
    assert_eq!(
        issues(&timer)[0].2,
        Epoch {
            instance: 43,
            request: 2
        }
    );
}

#[test]
fn identity_canonical_data_and_exact_eight_kib_boundary() {
    let reads = reads();
    let args = |s: &str| Value::record(vec![(ScalarString::new("s"), text(s))]).unwrap();
    let base = vec![text("read"), args(&"x".repeat(8184)), Value::null()];
    let identity = reads.identity(&base).unwrap();
    assert_eq!(identity.arguments.len(), 8192);
    let mut too_large = base.clone();
    too_large[1] = args(&"x".repeat(8185));
    assert_eq!(reads.identity(&too_large), Err("E_SITE_ARGS"));
    let reordered = Value::record(vec![
        (ScalarString::new("b"), number(2)),
        (ScalarString::new("a"), number(1)),
    ])
    .unwrap();
    let canonical = reads
        .identity(&[text("read"), reordered, Value::null()])
        .unwrap();
    assert_eq!(canonical.arguments, "{\"a\":1,\"b\":2}");
    for invalid in [
        Value::missing(),
        Value::error(crate::ValueError::new(
            "E_TYPE",
            crate::SourceSpan {
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
        )),
        Value::list(vec![Value::missing()]).unwrap(),
    ] {
        assert_eq!(
            reads.identity(&[text("read"), args("ok"), invalid]),
            Err("E_SITE_ARGS")
        );
    }
    let ValueKind::Record(record) = args("ok").kind().clone() else {
        panic!()
    };
    for invalid in [
        Value::component("Badge", record.clone(), vec![]).unwrap(),
        Value::site_envelope(crate::SiteEnvelope {
            name: "q".into(),
            members: record.clone(),
        }),
    ] {
        let wrapped = Value::record(vec![(ScalarString::new("x"), invalid)]).unwrap();
        assert_eq!(
            reads.identity(&[text("read"), wrapped, Value::null()]),
            Err("E_SITE_ARGS")
        );
    }
}

#[test]
fn full_provenance_and_normalized_options_control_dedup() {
    let base = reads();
    let args = vec![text("read"), Value::record(vec![]).unwrap(), number(0)];
    let original = base.identity(&args).unwrap();
    for change in 0..8 {
        let mut changed = base.clone();
        match change {
            0 => changed.authority.computer += "2",
            1 => changed.authority.session += "2",
            2 => changed.authority.scope_epoch += "2",
            3 => changed.authority.provider += "2",
            4 => changed.authority.entity += "2",
            5 => {
                changed.versions.get_mut("read").unwrap().schema = "schema-2".into();
            }
            6 => changed.versions.get_mut("read").unwrap().operation = "operation-2".into(),
            _ => changed.authority.verified_holder += "2",
        }
        assert_ne!(changed.identity(&args).unwrap(), original);
    }
    let mut default = args.clone();
    default[2] = number(1);
    assert_ne!(base.identity(&default).unwrap(), original);
    let mut refresh = args.clone();
    refresh.push(number(1));
    let clamped = base.identity(&refresh).unwrap();
    refresh[3] = number(2);
    assert_eq!(base.identity(&refresh).unwrap(), clamped);
    assert_ne!(clamped, original);
    refresh[3] = number(9999);
    assert_eq!(
        base.identity(&refresh)
            .unwrap()
            .refresh
            .unwrap()
            .to_string(),
        "3600"
    );
}

#[test]
fn missing_field_provenance_survives_record_omission_coalescing_and_named_memo() {
    let mut session = fixture("export const a = Query(\"read\", {}, {})\nexport const omitted = {id: a.value.id}\nexport const fallback = omitted.id ?? 7\nexport const b = Query(\"read\", {id: fallback}, null)\n\n{fallback} {b.value}");
    let initial = load(&mut session);
    let first = issues(&initial);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].0, "a");
    assert_field(&session, "b", "status", "\"loading\"");
    let settled = session.batch(vec![answer(&first[0], Ok("{}"))]);
    let second = issues(&settled);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].1.arguments, "{\"id\":7}");
    assert!(settled.candidate.unwrap().loading);
}

#[test]
fn schema_missing_without_dependency_errors_but_optional_absence_is_omitted() {
    let mut session = fixture("export const q = Query(\"read\", {id: {}.id}, 0)\n\n{q.value}");
    session.queries.reads.registry.0.insert(
        "read".into(),
        Operation::Query {
            arguments: Schema {
                required: vec!["id".into()],
                ..Default::default()
            },
            result: Schema::default(),
        },
    );
    let out = load(&mut session);
    assert!(issues(&out).is_empty());
    assert_field(&session, "q", "status", "\"error\"");
    assert_field(&session, "q", "error", "\"E_SITE_ARGS\"");
    assert!(!out.candidate.unwrap().loading);
    let mut optional = fixture("export const q = Query(\"read\", {id: {}.id}, 0)\n\n{q.value}");
    assert_eq!(issues(&load(&mut optional))[0].1.arguments, "{}");
}

#[test]
fn runtime_argument_errors_propagate_and_invalid_data_becomes_site_args() {
    let mut session =
        fixture("export let $n = 0\nexport const q = Query(\"read\", {x: 1 / $n}, 0)\n\n{q.value}");
    let out = load(&mut session);
    assert!(issues(&out).is_empty());
    assert_field(&session, "q", "status", "\"error\"");
    assert_field(&session, "q", "error", "\"E_DIV_ZERO\"");
    let mut session =
        fixture("export let $s = \"x\"\nexport const q = Query(\"read\", {s: $s}, 0)\n\n{q.value}");
    let out = session.batch(vec![Event::Load {
        epoch: 42,
        inputs: Box::new(Inputs::new(Profile::ShowUi)),
        restored: [("s".into(), text(&"x".repeat(8185)))].into(),
    }]);
    assert!(issues(&out).is_empty());
    assert_field(&session, "q", "error", "\"E_SITE_ARGS\"");
}

#[test]
fn dependency_error_settles_instead_of_waiting_and_identity_change_is_not_ready() {
    let mut session = fixture("export let $id = 1\nexport const a = Query(\"read\", {id: $id}, {id: 9})\nexport const b = Query(\"read\", {id: a.value.id}, 0)\n\n{b.value}\n\n<Button label=\"Next\" action={Set($id, 2)} />");
    let first = issues(&load(&mut session)).remove(0);
    let failed = session.batch(vec![answer(&first, Err("failed"))]);
    assert!(issues(&failed).is_empty());
    assert_field(&session, "b", "status", "\"error\"");
    assert!(!failed.candidate.unwrap().loading);

    let mut session = fixture("export let $id = 1\nexport const a = Query(\"read\", {id: $id}, {id: 9})\nexport const b = Query(\"read\", {id: a.value.id}, 0)\n\n{b.value}\n\n<Button label=\"Next\" action={Set($id, 2)} />");
    let first = issues(&load(&mut session)).remove(0);
    let next = issues(&session.batch(vec![answer(&first, Ok("{\"id\":10}"))])).remove(0);
    let ready = session
        .batch(vec![answer(&next, Ok("7"))])
        .candidate
        .unwrap();
    session.batch(vec![measure(&ready)]);
    let changed = session.batch(vec![Event::Activate(activation(&ready, 1))]);
    let issue = issues(&changed);
    assert_eq!(issue.len(), 1);
    assert_eq!(issue[0].0, "a");
    assert_field(&session, "a", "value", "{\"id\":10}");
    assert_field(&session, "a", "status", "\"loading\"");
    assert_field(&session, "b", "status", "\"loading\"");
    assert_field(&session, "b", "stale", "true");
}

#[test]
fn input_requires_the_committed_binding_and_never_the_unmeasured_replacement() {
    let mut session = fixture("export let $name = \"start\"\n\n<Input value={$name} />");
    let frame = load(&mut session).candidate.unwrap();
    let control = frame.bindings.keys().next().expect("a binding").clone();
    let event = |revision, binding: &str, value| Event::Input {
        instance: 42,
        revision,
        control: control.clone(),
        binding: binding.into(),
        value,
    };
    assert_eq!(
        session
            .batch(vec![event(frame.revision, "name", text("early"))])
            .rejected,
        [Rejected::Stale]
    );
    session.batch(vec![measure(&frame)]);
    assert_eq!(
        session
            .batch(vec![event(frame.revision, "other", text("wrong"))])
            .rejected,
        [Rejected::Stale]
    );
    let next = session.batch(vec![
        event(frame.revision, "name", text("one")),
        event(frame.revision, "name", text("two")),
    ]);
    assert_eq!(next.evaluations, 1);
    assert_eq!(
        session.runtime.state["name"].canonical_json().unwrap(),
        "\"two\""
    );
    let next = next.candidate.unwrap();
    assert_eq!(
        session
            .batch(vec![event(next.revision, "name", text("unmeasured"))])
            .rejected,
        [Rejected::Stale]
    );
    session.batch(vec![measure(&next)]);
    assert_eq!(
        session
            .batch(vec![event(frame.revision, "name", text("stale"))])
            .rejected,
        [Rejected::Stale]
    );
    let bad_type = session.batch(vec![event(next.revision, "name", number(8))]);
    assert_eq!(
        bad_type.diagnostics[0].code(),
        crate::DiagnosticCode::StateType
    );
    assert_eq!(
        session.runtime.state["name"].canonical_json().unwrap(),
        "\"two\""
    );
}

#[test]
fn answer_caps_schema_and_stale_correlation_are_checked_before_application() {
    let mut session = fixture("export const q = Query(\"read\", {}, 0)\n\n{q.value}");
    session.queries.reads.registry.0.insert(
        "read".into(),
        Operation::Query {
            arguments: Schema::default(),
            result: Schema {
                kind: Some(SchemaType::Number),
                ..Default::default()
            },
        },
    );
    let request = issues(&load(&mut session)).remove(0);
    let mut wrong = request.clone();
    wrong.0 = "other".into();
    assert_eq!(session.batch(vec![answer(&wrong, Ok("1"))]).evaluations, 0);
    wrong = request.clone();
    wrong.1.version.schema = "schema-2".into();
    assert_eq!(session.batch(vec![answer(&wrong, Ok("1"))]).evaluations, 0);
    session.batch(vec![answer(&request, Ok("\"wrong\""))]);
    assert_field(&session, "q", "error", "\"E_SITE_ARGS\"");
    for bytes in [
        vec![b' '; 1_048_577],
        format!("\"{}\"", "x".repeat(65_537)).into_bytes(),
    ] {
        let mut session = fixture("export const q = Query(\"read\", {}, 0)\n\n{q.value}");
        let request = issues(&load(&mut session)).remove(0);
        session.batch(vec![Event::Answer {
            site: request.0,
            identity: Box::new(request.1),
            epoch: request.2,
            answer: Ok(bytes),
        }]);
        assert_field(&session, "q", "error", "\"answer too large\"");
    }
}

#[test]
fn availability_retires_timers_and_requests_without_reusing_epochs() {
    let mut session = fixture("export const q = Query(\"read\", {}, 0, 2)\n\n{q.value}");
    let old = issues(&load(&mut session)).remove(0);
    session.batch(vec![Event::Availability(false)]);
    assert_eq!(session.batch(vec![answer(&old, Ok("9"))]).evaluations, 0);
    assert_eq!(
        session
            .batch(vec![Event::Timer(Timer::Timeout(old.2))])
            .evaluations,
        0
    );
    let new = issues(&session.batch(vec![Event::Availability(true)])).remove(0);
    assert_eq!(new.2.request, 2);
    let ready = session.batch(vec![answer(&new, Ok("9"))]);
    let timer = ready
        .effects
        .iter()
        .find_map(|e| {
            if let Effect::Arm(t @ Timer::Refresh(..), _) = e {
                Some(t.clone())
            } else {
                None
            }
        })
        .unwrap();
    session.batch(vec![Event::Availability(false)]);
    assert_eq!(session.batch(vec![Event::Timer(timer)]).evaluations, 0);
    assert_field(&session, "q", "stale", "true");
}

#[test]
fn handoffs_complete_only_through_host_events_not_answers() {
    let mut session = fixture("<Button label=\"Compose\" action={Compose(\"draft\")} />");
    let frame = load(&mut session).candidate.unwrap();
    session.batch(vec![measure(&frame)]);
    let action = activation(&frame, 1);
    let out = session.batch(vec![Event::Activate(action.clone())]);
    assert!(
        matches!(out.effects.as_slice(), [Effect::Handoff(a, Handoff::Compose(text))] if a == &action && text.as_str() == "draft")
    );
    assert_eq!(
        session
            .batch(vec![Event::Activate(Activation {
                id: 2,
                ..action.clone()
            })])
            .rejected,
        [Rejected::Busy]
    );
    assert_eq!(
        session
            .batch(vec![Event::HandoffsComplete(action.clone())])
            .evaluations,
        0
    );
    assert!(session
        .batch(vec![Event::Activate(Activation { id: 3, ..action })])
        .rejected
        .is_empty());
}
