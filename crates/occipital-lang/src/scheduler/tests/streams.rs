use super::*;
use std::fmt::Write;

type Request = (String, Identity, Epoch);
const WATCH: &str = "export const w = Watch(\"events\", {})\n\n{w.status}\n\n<Button label=\"Refresh\" action={Refresh(w)} />";
const PAGE: &str = "export const p = PagedList(\"pages\", {}, 200)\n\n<List more={p} />\n\n<Button label=\"Refresh\" action={Refresh(p)} />\n\n<Button label=\"More\" action={LoadMore(p)} />";

#[test]
fn aggregate_budget_failure_stops_watch_and_releases_unapplied_notices_immediately() {
    use crate::accounts::{Accounts, Resources};
    use crate::{AccountScope, SharedAccount};
    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    let mut session = stream_fixture(WATCH, true, false, None)
        .with_accounts(chat.clone(), app.clone())
        .unwrap();
    let request = watches(&load(&mut session)).remove(0).0;
    session.batch(vec![ack(&request, 0), notice(&request, 1, 1, "[7]")]);
    assert_eq!(
        session.queries.shared[&request.1]
            .watch
            .as_ref()
            .unwrap()
            .pending_frames()
            .count(),
        1
    );
    assert!(chat.usage().retained > 512 * 1024);
    let remaining = 24_000_000 - chat.usage().spent_fuel;
    for fuel in [8_000_000, 8_000_000, remaining - 16_000_000] {
        let accounts = Accounts::new(AccountScope::Interface, chat.clone(), app.clone()).unwrap();
        let mut lease = accounts
            .reserve(Resources {
                fuel,
                ..Resources::default()
            })
            .unwrap();
        lease.spend(fuel, 0).unwrap();
    }
    let out = session.batch(vec![notice(&request, 2, 2, "[8]")]);
    assert!(out
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Cancel { identity, .. } if identity == &request.1)));
    assert_field(&session, "w", "error", "\"too_large\"");
    assert!(session.queries.shared[&request.1].request.is_none());
    assert!(chat.usage().retained < 512 * 1024);
    assert_eq!(app.usage().retained, chat.usage().retained);
    drop((out, session));
    assert_eq!(chat.usage().retained, 0);
    assert_eq!(app.usage().retained, 0);
}

fn stream_fixture(source: &str, resume: bool, replay: bool, key: Option<&str>) -> Session {
    let mut reads = reads();
    for (name, operation) in [
        (
            "events",
            Operation::Watch {
                arguments: Schema::default(),
                item: Schema::default(),
                resume,
                replay,
            },
        ),
        (
            "pages",
            Operation::Paged {
                arguments: Schema::default(),
                page: Schema::default(),
                item_key: key.map(str::to_owned),
            },
        ),
    ] {
        reads.registry.0.insert(name.into(), operation);
        reads.versions.insert(
            name.into(),
            ReadVersion {
                operation: "v1".into(),
                schema: "s1".into(),
            },
        );
    }
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

fn watches(out: &Output) -> Vec<(Request, Option<u64>)> {
    out.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Subscribe {
                site,
                identity,
                epoch,
                after,
            } => Some(((site.clone(), identity.clone(), *epoch), *after)),
            _ => None,
        })
        .collect()
}
fn pages(out: &Output) -> Vec<(Request, Option<String>, u16)> {
    out.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Page {
                site,
                identity,
                epoch,
                cursor,
                limit,
            } => Some((
                (site.clone(), identity.clone(), *epoch),
                cursor.clone(),
                *limit,
            )),
            _ => None,
        })
        .collect()
}
fn ack(r: &Request, after: u64) -> Event {
    Event::WatchAck {
        identity: Box::new(r.1.clone()),
        epoch: r.2,
        after,
    }
}
fn notice(r: &Request, token: u64, first: u64, bytes: &str) -> Event {
    Event::WatchNotice {
        identity: Box::new(r.1.clone()),
        epoch: r.2,
        token,
        first,
        bytes: bytes.as_bytes().to_vec(),
    }
}
fn end(r: &Request, error: Option<&str>) -> Event {
    Event::WatchEnd {
        identity: Box::new(r.1.clone()),
        epoch: r.2,
        error: error.map(str::to_owned),
    }
}
fn page_answer(r: &Request, cursor: Option<&str>, result: Result<&str, &str>) -> Event {
    Event::PageAnswer {
        identity: Box::new(r.1.clone()),
        epoch: r.2,
        cursor: cursor.map(str::to_owned),
        answer: result.map(|s| s.as_bytes().to_vec()).map_err(str::to_owned),
    }
}
fn tick(s: &Session, r: &Request) -> Event {
    Event::Timer(
        s.queries
            .apply_timer
            .clone()
            .unwrap_or(Timer::WatchApply(r.2, s.queries.apply_generation + 1)),
    )
}
fn commit(session: &mut Session, out: &Output) {
    if let Some(c) = &out.candidate {
        session.batch(vec![measure(c)]);
    }
}
fn act(session: &mut Session, index: usize, id: u64) -> Output {
    let c = &session.committed.as_ref().unwrap().candidate;
    session.batch(vec![Event::Activate(Activation {
        control: c.controls.keys().nth(index).unwrap().clone(),
        ..activation(c, id)
    })])
}
fn auto(session: &mut Session, token: u64) -> Output {
    let c = &session.committed.as_ref().unwrap().candidate;
    session.batch(vec![Event::AutoLoadMore {
        instance: 42,
        revision: c.revision,
        control: c.sentinels.keys().next().unwrap().clone(),
        token,
    }])
}
fn armed(out: &Output, predicate: impl Fn(&Timer) -> bool) -> Timer {
    out.effects
        .iter()
        .find_map(|e| match e {
            Effect::Arm(t, _) if predicate(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn watch_ack_empty_live_end_and_transitive_readiness() {
    let source = "export const w = Watch(\"events\", {})\nexport const q = Query(\"read\", {status: w.status}, 0)\n\n{q.value}";
    let mut s = stream_fixture(source, true, false, None);
    let initial = load(&mut s);
    assert!(issues(&initial).is_empty());
    let r = watches(&initial)[0].0.clone();
    assert!(initial.candidate.unwrap().loading);
    let live = s.batch(vec![ack(&r, 0)]);
    assert_eq!(issues(&live).len(), 1);
    assert_field(&s, "w", "status", "\"live\"");
    assert_field(&s, "w", "value", "[]");
    let q = issues(&live)[0].clone();
    let ready = s.batch(vec![answer(&q, Ok("3"))]);
    assert!(!ready.candidate.unwrap().loading);
    s.batch(vec![end(&r, None)]);
    assert_field(&s, "w", "status", "\"ended\"");
    assert!(watches(&s.batch(vec![Event::Clock {
        now: Decimal::from_integer(8).unwrap(),
        zone: text("UTC")
    }]))
    .is_empty());
    assert_eq!(
        s.batch(vec![Event::Timer(Timer::Timeout(r.2))]).evaluations,
        0
    );
}

#[test]
fn watch_item_sequences_gaps_prefixes_and_credit_are_applied_once_per_tick() {
    let mut s = stream_fixture(WATCH, true, false, None);
    let initial = load(&mut s);
    let r = watches(&initial)[0].0.clone();
    assert!(initial.effects.iter().any(
        |e| matches!(e, Effect::Credit { epoch, notices: 16, bytes: 524288 } if *epoch == r.2)
    ));
    s.batch(vec![ack(&r, 0)]);
    let pending = s.batch(vec![notice(&r, 1, 1, "[1,2]"), notice(&r, 2, 5, "[5,6]")]);
    assert_eq!(pending.evaluations, 0);
    assert!(pending
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Arm(Timer::WatchApply(_, _), d) if d.to_string() == "0.25")));
    assert_field(&s, "w", "value", "[]");
    let applied = s.batch(vec![tick(&s, &r)]);
    assert_field(&s, "w", "value", "[1,2,5,6]");
    assert_field(&s, "w", "dropped", "2");
    assert!(applied
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Credit { notices: 2, .. })));
    assert_eq!(s.batch(vec![tick(&s, &r)]).evaluations, 0);
    s.batch(vec![
        notice(&r, 3, 6, "[6,7,8]"),
        notice(&r, 4, 1, "[1,2]"),
        notice(&r, 4, 9, "[9]"),
    ]);
    let applied = s.batch(vec![tick(&s, &r)]);
    assert_field(&s, "w", "value", "[1,2,5,6,7,8]");
    assert_field(&s, "w", "dropped", "2");
    assert!(applied
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Credit { notices: 2, .. })));
    s.batch(vec![notice(&r, 5, u64::MAX, "[]"), tick(&s, &r)]);
    s.batch(vec![notice(&r, 6, 9, "[9]"), tick(&s, &r)]);
    assert_field(&s, "w", "dropped", "2");
}

#[test]
fn watch_protocol_errors_oversizes_timeout_and_late_epochs() {
    for payload in [
        "[1]".to_owned(),
        format!("[\"{}\"]", "x".repeat(2045)),
        " ".repeat(32769),
    ] {
        let mut s = stream_fixture(WATCH, true, false, None);
        let r = watches(&load(&mut s))[0].0.clone();
        if payload != "[1]" {
            s.batch(vec![ack(&r, 0)]);
        }
        s.batch(vec![notice(&r, 1, 1, &payload)]);
        assert_field(&s, "w", "status", "\"error\"");
        assert_field(&s, "w", "value", "[]");
    }
    let mut s = stream_fixture(WATCH, true, false, None);
    let r = watches(&load(&mut s))[0].0.clone();
    s.batch(vec![Event::Timer(Timer::Timeout(r.2))]);
    assert_field(&s, "w", "error", "\"timed out\"");
    assert_eq!(
        s.batch(vec![ack(&r, 0), end(&r, None), notice(&r, 1, 1, "[1]")])
            .evaluations,
        0
    );
    let mut s = stream_fixture(WATCH, true, false, None);
    let r = watches(&load(&mut s))[0].0.clone();
    s.batch(vec![ack(&r, 0), notice(&r, 1, u64::MAX, "[1,2]")]);
    assert_field(&s, "w", "error", "\"protocol error\"");
}

#[test]
fn watch_count_and_byte_credit_bound_unapplied_backlog() {
    for byte_overflow in [false, true] {
        let mut s = stream_fixture(WATCH, true, false, None);
        let r = watches(&load(&mut s))[0].0.clone();
        s.batch(vec![ack(&r, 0)]);
        // Sixteen 7,000-byte logical items expand a notice to 112,016 bytes,
        // although the wire payload is under 32 KiB. Five exceed byte credit.
        let payload = if byte_overflow {
            serde_json::to_string(&vec!["x".repeat(1746); 16]).unwrap()
        } else {
            "[]".into()
        };
        for i in 1..=if byte_overflow { 5 } else { 17 } {
            s.batch(vec![notice(&r, i, i, &payload)]);
        }
        let applied = s.batch(vec![tick(&s, &r)]);
        assert_field(&s, "w", "status", "\"error\"");
        assert_field(&s, "w", "error", "\"too_large\"");
        assert!(applied
            .effects
            .iter()
            .all(|e| !matches!(e, Effect::Credit { .. })));
        assert_eq!(s.batch(vec![tick(&s, &r)]).evaluations, 0);
    }
}

#[test]
fn watch_buffer_enforces_both_item_and_logical_byte_caps() {
    for large in [false, true] {
        let mut s = stream_fixture(WATCH, true, false, None);
        let r = watches(&load(&mut s))[0].0.clone();
        s.batch(vec![ack(&r, 0)]);
        let payload = if large {
            serde_json::to_string(&vec!["x".repeat(1746); 16]).unwrap()
        } else {
            serde_json::to_string(&vec![0; 100]).unwrap()
        };
        let count = if large { 16 } else { 100 };
        let batches = if large { 4 } else { 11 };
        for i in 0..batches {
            s.batch(vec![
                notice(&r, i + 1, i * count + 1, &payload),
                tick(&s, &r),
            ]);
        }
        let value = field(&s, "w", "value");
        let ValueKind::List(items) = value.kind() else {
            panic!()
        };
        if large {
            assert_eq!(items.len(), 37);
            assert_field(&s, "w", "dropped", "27");
        } else {
            assert_eq!(items.len(), 1000);
            assert_field(&s, "w", "dropped", "100");
        }
        assert!(value.logical_size() <= 256 * 1024);
    }
}

#[test]
fn watch_teardown_resume_replay_new_only_refresh_and_undemand() {
    for (resume, replay, expected, after) in [
        (true, false, "[1,2]", Some(2)),
        (false, true, "[]", None),
        (false, false, "[1,2]", None),
    ] {
        let mut s = stream_fixture(WATCH, resume, replay, None);
        let r = watches(&load(&mut s))[0].0.clone();
        s.batch(vec![ack(&r, 0), notice(&r, 1, 1, "[1,2]"), tick(&s, &r)]);
        let hidden = s.batch(vec![Event::Foreground(false)]);
        assert!(hidden
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Cancel { epoch, .. } if *epoch == r.2)));
        assert_field(&s, "w", "stale", "true");
        let resumed = s.batch(vec![Event::Foreground(true)]);
        let (next, position) = watches(&resumed)[0].clone();
        assert_eq!(position, after);
        assert!(next.2 > r.2);
        assert_field(&s, "w", "value", expected);
        assert_field(&s, "w", "status", "\"loading\"");
        let out = s.batch(vec![ack(&next, after.unwrap_or(0))]);
        commit(&mut s, &out);
        let refresh = act(&mut s, 0, 1);
        let (fresh, position) = watches(&refresh)[0].clone();
        assert_eq!(position, None);
        assert_field(&s, "w", "value", "[]");
        assert_field(&s, "w", "dropped", "0");
        assert_eq!(
            s.batch(vec![notice(&next, 2, 3, "[3]"), end(&r, None), ack(&r, 0)])
                .evaluations,
            0
        );
        s.batch(vec![end(&fresh, None)]);
        assert_field(&s, "w", "status", "\"ended\"");
    }
    let source = "export const w = Watch(\"events\", {})\n\n{now < 1 ? w.status : \"hidden\"}";
    let mut s = stream_fixture(source, true, false, None);
    let r = watches(&load(&mut s))[0].0.clone();
    s.batch(vec![ack(&r, 0), notice(&r, 1, 1, "[1]"), tick(&s, &r)]);
    let gone = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(2).unwrap(),
        zone: text("UTC"),
    }]);
    assert!(gone
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Cancel { .. })));
    let back = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(0).unwrap(),
        zone: text("UTC"),
    }]);
    assert_eq!(watches(&back)[0].1, Some(1));
}

#[test]
fn paged_first_later_final_totals_and_equal_unkeyed_rows() {
    let mut s = stream_fixture(PAGE, false, false, None);
    let initial = load(&mut s);
    let (first, cursor, limit) = pages(&initial)[0].clone();
    assert_eq!((cursor, limit), (None, 200));
    let ready = s.batch(vec![page_answer(
        &first,
        None,
        Ok(r#"{"items":["same"],"next":"a","total":3}"#),
    )]);
    assert_field(&s, "p", "items", "[\"same\"]");
    assert_field(&s, "p", "total", "3");
    assert_field(&s, "p", "stale", "false");
    commit(&mut s, &ready);
    let more = act(&mut s, 1, 1);
    let (second, cursor, _) = pages(&more)[0].clone();
    assert_eq!(cursor.as_deref(), Some("a"));
    assert_field(&s, "p", "loadingMore", "true");
    assert!(pages(&act(&mut s, 1, 2)).is_empty());
    assert_eq!(
        s.batch(vec![page_answer(
            &second,
            Some("wrong"),
            Ok(r#"{"items":[],"next":null}"#)
        )])
        .evaluations,
        0
    );
    let done = s.batch(vec![page_answer(
        &second,
        Some("a"),
        Ok(r#"{"items":["same"],"next":null}"#),
    )]);
    assert_field(&s, "p", "items", "[\"same\",\"same\"]");
    assert_field(&s, "p", "done", "true");
    assert_field(&s, "p", "total", "null");
    assert_field(&s, "p", "loadingMore", "false");
    commit(&mut s, &done);
    assert!(pages(&act(&mut s, 1, 3)).is_empty());
    assert_eq!(
        s.batch(vec![page_answer(
            &first,
            None,
            Ok(r#"{"items":[99],"next":null}"#)
        )])
        .evaluations,
        0
    );
}

#[test]
fn paging_sentinel_tokens_empty_pages_keyed_duplicates_and_explicit_progress() {
    for key in [None, Some("id")] {
        let mut s = stream_fixture(PAGE, false, false, key);
        let r = pages(&load(&mut s))[0].0.clone();
        let ready = s.batch(vec![page_answer(
            &r,
            None,
            Ok(r#"{"items":[{"id":1}],"next":"a"}"#),
        )]);
        commit(&mut s, &ready);
        let out = auto(&mut s, 1);
        let r = pages(&out)[0].0.clone();
        assert!(pages(&auto(&mut s, 1)).is_empty());
        // A fresh token ignored during an in-flight request stays consumed.
        assert!(pages(&auto(&mut s, 2)).is_empty());
        let payload = if key.is_some() {
            r#"{"items":[{"id":1}],"next":"b"}"#
        } else {
            r#"{"items":[],"next":"b"}"#
        };
        let empty = s.batch(vec![page_answer(&r, Some("a"), Ok(payload))]);
        commit(&mut s, &empty);
        assert_field(&s, "p", "items", "[{\"id\":1}]");
        assert!(pages(&auto(&mut s, 2)).is_empty());
        assert!(pages(&auto(&mut s, 3)).is_empty());
        let explicit = act(&mut s, 1, 1);
        let (r, cursor, _) = pages(&explicit)[0].clone();
        assert_eq!(cursor.as_deref(), Some("b"));
        let ready = s.batch(vec![page_answer(
            &r,
            Some("b"),
            Ok(r#"{"items":[{"id":2}],"next":"c"}"#),
        )]);
        commit(&mut s, &ready);
        assert_eq!(pages(&auto(&mut s, 4)).len(), 1);
    }
}

#[test]
fn paging_errors_timeouts_retries_invalid_cursor_and_refresh() {
    let mut s = stream_fixture(PAGE, false, false, None);
    let r = pages(&load(&mut s))[0].0.clone();
    let failed = s.batch(vec![Event::Timer(Timer::Timeout(r.2))]);
    assert_field(&s, "p", "error", "\"timed out\"");
    assert_field(&s, "p", "items", "[]");
    commit(&mut s, &failed);
    assert!(pages(&auto(&mut s, 1)).is_empty());
    let retry = pages(&act(&mut s, 1, 1))[0].clone();
    assert_eq!(retry.1, None);
    assert!(retry.0 .2 > r.2);
    let ready = s.batch(vec![page_answer(
        &retry.0,
        None,
        Ok(r#"{"items":[1],"next":"a","total":9}"#),
    )]);
    commit(&mut s, &ready);
    let later = pages(&act(&mut s, 1, 2))[0].0.clone();
    let failed = s.batch(vec![page_answer(&later, Some("a"), Err("failed"))]);
    assert_field(&s, "p", "items", "[1]");
    assert_field(&s, "p", "stale", "true");
    assert_field(&s, "p", "loadingMore", "false");
    commit(&mut s, &failed);
    let retry = pages(&act(&mut s, 1, 3))[0].clone();
    assert_eq!(retry.1.as_deref(), Some("a"));
    let invalid = s.batch(vec![page_answer(
        &retry.0,
        Some("a"),
        Ok(r#"{"items":[2],"next":"a"}"#),
    )]);
    assert_field(&s, "p", "error", "\"invalid_page\"");
    assert_field(&s, "p", "items", "[1]");
    commit(&mut s, &invalid);
    assert!(pages(&act(&mut s, 1, 4)).is_empty());
    let refreshing = act(&mut s, 0, 5);
    let r = pages(&refreshing)[0].0.clone();
    assert_field(&s, "p", "items", "[1]");
    assert_field(&s, "p", "stale", "true");
    assert_field(&s, "p", "status", "\"loading\"");
    assert_field(&s, "p", "total", "null");
    let failed = s.batch(vec![page_answer(&r, None, Err("refresh failed"))]);
    assert_field(&s, "p", "items", "[1]");
    assert_field(&s, "p", "stale", "true");
    commit(&mut s, &failed);
    let r = pages(&act(&mut s, 1, 6))[0].0.clone();
    s.batch(vec![page_answer(
        &r,
        None,
        Ok(r#"{"items":[],"next":null}"#),
    )]);
    assert_field(&s, "p", "items", "[]");
    assert_field(&s, "p", "done", "true");
    assert_field(&s, "p", "stale", "false");
}

#[test]
fn paging_rejects_malformed_pages_byte_caps_items_and_cursor_cycles_atomically() {
    let mut payloads = vec![
        r#"{"items":[],"next":null,"total":-1}"#.to_owned(),
        r#"{"items":[],"next":null,"total":1.5}"#.to_owned(),
        r#"{"items":[],"next":2}"#.to_owned(),
        r#"{"items":[]}"#.to_owned(),
        r#"{"items":0,"next":null}"#.to_owned(),
        r#"{"items":[],"next":null,"total":"9"}"#.to_owned(),
        format!(r#"{{"items":[],"next":"{}"}}"#, "a".repeat(1025)),
        format!(r#"{{"items":["{}"],"next":null}}"#, "x".repeat(16381)),
        " ".repeat(256 * 1024 + 1),
    ];
    // Many individually valid items can breach the expanded page cap while
    // staying below its serialized cap.
    payloads.push(format!(
        r#"{{"items":{},"next":null}}"#,
        serde_json::to_string(&vec!["x".repeat(3968); 66]).unwrap()
    ));
    for (index, payload) in payloads.into_iter().enumerate() {
        let mut s = stream_fixture(PAGE, false, false, None);
        let r = pages(&load(&mut s))[0].0.clone();
        s.batch(vec![page_answer(&r, None, Ok(&payload))]);
        assert_eq!(
            field(&s, "p", "error").canonical_json().unwrap(),
            "\"invalid_page\"",
            "payload {index} length {}",
            payload.len()
        );
        assert_field(&s, "p", "items", "[]");
    }
    let mut s = stream_fixture(PAGE, false, false, None);
    let mut r = pages(&load(&mut s))[0].0.clone();
    for (i, (cursor, next)) in [(None, "a"), (Some("a"), "b"), (Some("b"), "a")]
        .into_iter()
        .enumerate()
    {
        let out = s.batch(vec![page_answer(
            &r,
            cursor,
            Ok(&format!(r#"{{"items":[{}],"next":"{next}"}}"#, i + 1)),
        )]);
        commit(&mut s, &out);
        if i < 2 {
            r = pages(&act(&mut s, 1, i as u64 + 1))[0].0.clone();
        }
    }
    assert_field(&s, "p", "items", "[1,2]");
    assert_field(&s, "p", "error", "\"invalid_page\"");
}

#[test]
fn paging_item_page_and_retained_caps_do_not_partially_append() {
    for mode in 0..3 {
        let mut s = stream_fixture(PAGE, false, false, None);
        let mut r = pages(&load(&mut s))[0].0.clone();
        let (items, rounds) = match mode {
            0 => (serde_json::to_string(&vec![0; 200]).unwrap(), 10),
            1 => ("[0]".into(), 20),
            _ => (
                serde_json::to_string(&vec!["x".repeat(4000); 60]).unwrap(),
                5,
            ),
        };
        for i in 0..rounds {
            let cursor = (i > 0).then(|| i.to_string());
            let next = (i + 1).to_string();
            let out = s.batch(vec![page_answer(
                &r,
                cursor.as_deref(),
                Ok(&format!(r#"{{"items":{items},"next":"{next}"}}"#)),
            )]);
            commit(&mut s, &out);
            if i + 1 < rounds {
                r = pages(&act(&mut s, 1, i as u64 + 1))[0].0.clone();
            }
        }
        assert_field(&s, "p", "capped", "true");
        let retained = field(&s, "p", "items");
        let ValueKind::List(items) = retained.kind() else {
            panic!()
        };
        assert_eq!(
            items.len(),
            match mode {
                0 => 2000,
                1 => 20,
                _ => 240,
            }
        );
        assert!(retained.logical_size() <= 4 * 1024 * 1024);
        assert!(pages(&act(&mut s, 1, 30)).is_empty());
        let refresh = act(&mut s, 0, 31);
        assert_eq!(pages(&refresh).len(), 1);
        assert_field(&s, "p", "capped", "false");
    }
}

#[test]
fn paging_identity_changes_undemand_and_stale_answers_clear_or_preserve_as_specified() {
    let source =
        "export const p = PagedList(\"pages\", {n: now})\n\n{now < 2 ? p.status : \"hidden\"}";
    let mut s = stream_fixture(source, false, false, None);
    let r = pages(&load(&mut s))[0].0.clone();
    s.batch(vec![page_answer(
        &r,
        None,
        Ok(r#"{"items":[1],"next":null}"#),
    )]);
    let changed = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(1).unwrap(),
        zone: text("UTC"),
    }]);
    assert_eq!(pages(&changed)[0].2, 50);
    assert_field(&s, "p", "items", "[]");
    let old = pages(&changed)[0].0.clone();
    s.batch(vec![Event::Clock {
        now: Decimal::from_integer(2).unwrap(),
        zone: text("UTC"),
    }]);
    assert!(s.queries.shared.is_empty());
    let back = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(1).unwrap(),
        zone: text("UTC"),
    }]);
    let r = pages(&back)[0].0.clone();
    assert!(r.2 > old.2);
    assert_eq!(
        s.batch(vec![page_answer(
            &old,
            None,
            Ok(r#"{"items":[99],"next":null}"#)
        )])
        .evaluations,
        0
    );
}

#[test]
fn visibility_grace_background_connection_offline_and_resume_for_all_site_kinds() {
    let source = "export const q = Query(\"read\", {}, 0, 2)\nexport const w = Watch(\"events\", {})\nexport const p = PagedList(\"pages\", {})\n\n{q.status} {w.status} {p.status}";
    for policy in [0, 1, 2] {
        let mut s = stream_fixture(source, true, false, None);
        let initial = load(&mut s);
        let q = issues(&initial)[0].clone();
        let w = watches(&initial)[0].0.clone();
        let p = pages(&initial)[0].0.clone();
        let stopped = if policy == 0 {
            let grace = s.batch(vec![Event::Visibility(false)]);
            assert!(grace
                .effects
                .iter()
                .all(|e| !matches!(e, Effect::Cancel { .. })));
            let timer = armed(&grace, |t| matches!(t, Timer::Hidden(_)));
            s.batch(vec![Event::Timer(timer)])
        } else if policy == 1 {
            s.batch(vec![Event::Foreground(false)])
        } else {
            s.batch(vec![Event::Connection(false)])
        };
        assert_eq!(
            stopped
                .effects
                .iter()
                .filter(|e| matches!(e, Effect::Cancel { .. }))
                .count(),
            3
        );
        assert_eq!(
            s.batch(vec![
                answer(&q, Ok("9")),
                ack(&w, 0),
                page_answer(&p, None, Ok(r#"{"items":[],"next":null}"#))
            ])
            .evaluations,
            0
        );
        if policy == 2 {
            let timer = armed(&stopped, |t| matches!(t, Timer::Offline(_)));
            s.batch(vec![Event::Timer(timer)]);
            for site in ["q", "w", "p"] {
                assert_field(&s, site, "status", "\"error\"");
                assert_field(&s, site, "error", "\"offline\"");
            }
        }
        let out = s.batch(vec![match policy {
            0 => Event::Visibility(true),
            1 => Event::Foreground(true),
            _ => Event::Connection(true),
        }]);
        assert!(issues(&out)[0].2 > q.2);
        assert!(watches(&out)[0].0 .2 > w.2);
        assert!(pages(&out)[0].0 .2 > p.2);
    }
    let mut s = stream_fixture(source, true, false, None);
    load(&mut s);
    let hidden = s.batch(vec![Event::Visibility(false)]);
    let timer = armed(&hidden, |t| matches!(t, Timer::Hidden(_)));
    let visible = s.batch(vec![Event::Visibility(true)]);
    assert!(visible
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Stop(t) if *t == timer)));
    assert_eq!(s.batch(vec![Event::Timer(timer)]).evaluations, 0);
}

#[test]
fn paging_visibility_retries_only_pending_pages_not_ready_done_or_capped() {
    for next in ["null", "\"a\""] {
        let mut s = stream_fixture(PAGE, false, false, None);
        let first = pages(&load(&mut s))[0].0.clone();
        let ready = s.batch(vec![page_answer(
            &first,
            None,
            Ok(&format!(r#"{{"items":[1],"next":{next}}}"#)),
        )]);
        commit(&mut s, &ready);
        s.batch(vec![Event::Foreground(false)]);
        let out = s.batch(vec![Event::Foreground(true)]);
        assert!(pages(&out).is_empty());
        assert_field(&s, "p", "stale", "true");
        if next != "null" {
            let more = act(&mut s, 1, 1);
            let old = pages(&more)[0].0.clone();
            s.batch(vec![Event::Connection(false)]);
            let reconnect = s.batch(vec![Event::Connection(true)]);
            let r = pages(&reconnect)[0].clone();
            assert_eq!(r.1.as_deref(), Some("a"));
            assert!(r.0 .2 > old.2);
            assert_field(&s, "p", "items", "[1]");
        }
    }
}

#[test]
fn provenance_and_four_request_queue_apply_to_pages_but_not_subscriptions() {
    let source = "export const a = Query(\"read\", {n:1}, 0)\nexport const b = Query(\"read\", {n:2}, 0)\nexport const c = Query(\"read\", {n:3}, 0)\nexport const d = Query(\"read\", {n:4}, 0)\nexport const p = PagedList(\"pages\", {})\nexport const w = Watch(\"events\", {})\n\n{a.value} {b.value} {c.value} {d.value} {p.status} {w.status}";
    let mut s = stream_fixture(source, true, false, None);
    let initial = load(&mut s);
    assert_eq!(issues(&initial).len(), 4);
    assert!(pages(&initial).is_empty());
    assert_eq!(watches(&initial).len(), 1);
    let out = s.batch(vec![answer(&issues(&initial)[0], Ok("1"))]);
    assert_eq!(pages(&out).len(), 1);
    for source in [WATCH, PAGE] {
        for wrong_holder in [false, true] {
            let mut s = stream_fixture(source, true, false, None);
            if wrong_holder {
                s.queries.reads.authority.verified_holder = "other".into();
            } else {
                s.queries.reads.authority.computer = "other".into();
            }
            let out = load(&mut s);
            assert!(watches(&out).is_empty() && pages(&out).is_empty());
            assert_field(
                &s,
                if source == WATCH { "w" } else { "p" },
                "error",
                "\"E_SITE_ARGS\"",
            );
        }
    }
}

fn trace_step(trace: &mut String, label: &str, s: &Session, out: &Output, site: &str) {
    writeln!(
        trace,
        "{label}: eval={} fuel={} revision={:?} loading={:?} rejected={:?}",
        out.evaluations,
        out.fuel,
        out.candidate.as_ref().map(|c| c.revision),
        out.candidate.as_ref().map(|c| c.loading),
        out.rejected
    )
    .unwrap();
    for effect in &out.effects {
        match effect {
            Effect::Subscribe {
                site, epoch, after, ..
            } => writeln!(
                trace,
                "subscribe {site} {}:{} after={after:?}",
                epoch.instance, epoch.request
            ),
            Effect::Page {
                site,
                epoch,
                cursor,
                limit,
                ..
            } => writeln!(
                trace,
                "page {site} {}:{} cursor={cursor:?} limit={limit}",
                epoch.instance, epoch.request
            ),
            Effect::Cancel { site, epoch, .. } => {
                writeln!(trace, "cancel {site} {}:{}", epoch.instance, epoch.request)
            }
            other => writeln!(trace, "{other:?}"),
        }
        .unwrap();
    }
    if let Some(c) = &out.candidate {
        let tree = format!("{:?}", c.tree);
        let digest = hmac_sha256::Hash::hash(tree.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if !trace.contains(&format!("tree-definition={digest} ")) {
            writeln!(trace, "tree-definition={digest} {tree}").unwrap();
        }
        writeln!(trace, "tree-sha256={digest}").unwrap();
    }
    if s.queries.sites.contains_key(site) {
        for key in if site == "w" {
            &["value", "status", "error", "stale", "dropped"][..]
        } else {
            &[
                "items",
                "status",
                "error",
                "stale",
                "done",
                "total",
                "loadingMore",
                "capped",
            ][..]
        } {
            let value = field(s, site, key);
            let json = value.canonical_json().unwrap();
            if json.len() < 256 {
                write!(trace, "{key}={json} ").unwrap();
            } else {
                write!(trace, "{key}=logical:{} ", value.logical_size()).unwrap();
            }
        }
        trace.pop();
        writeln!(trace).unwrap();
    }
}

fn watch_replay() -> String {
    let mut trace =
        format!("source={WATCH:?}\nregistry: events resume=true replay=false item=any\n");
    let mut s = stream_fixture(WATCH, true, false, None);
    let out = load(&mut s);
    trace_step(&mut trace, "empty-loading", &s, &out, "w");
    let r = watches(&out)[0].0.clone();
    let out = s.batch(vec![ack(&r, 0)]);
    trace_step(&mut trace, "empty-live", &s, &out, "w");
    assert!(!out.candidate.as_ref().unwrap().loading);
    let out = s.batch(vec![notice(&r, 1, 1, "[1,2]"), notice(&r, 2, 5, "[5]")]);
    trace_step(&mut trace, "reserved-not-applied", &s, &out, "w");
    let out = s.batch(vec![tick(&s, &r)]);
    trace_step(&mut trace, "live-gap", &s, &out, "w");
    assert_field(&s, "w", "value", "[1,2,5]");
    assert_field(&s, "w", "dropped", "2");
    let out = s.batch(vec![end(&r, None)]);
    trace_step(&mut trace, "ended", &s, &out, "w");
    commit(&mut s, &out);
    let out = act(&mut s, 0, 1);
    trace_step(&mut trace, "refresh", &s, &out, "w");
    let r = watches(&out)[0].0.clone();
    let out = s.batch(vec![end(&r, None)]);
    trace_step(&mut trace, "empty-ended-without-ack", &s, &out, "w");
    commit(&mut s, &out);
    let out = act(&mut s, 0, 2);
    let r = watches(&out)[0].0.clone();
    s.batch(vec![ack(&r, 0)]);
    let payload = serde_json::to_string(&vec!["x".repeat(1746); 16]).unwrap();
    writeln!(trace, "overflow fixture: five notices, sixteen 1746-scalar ASCII strings each, first=1, tokens=1..5").unwrap();
    for token in 1..=5 {
        s.batch(vec![notice(&r, token, 1, &payload)]);
    }
    let out = s.batch(vec![tick(&s, &r)]);
    trace_step(&mut trace, "byte-credit-overflow", &s, &out, "w");
    assert_field(&s, "w", "error", "\"too_large\"");
    trace
}

fn paging_replay() -> String {
    let mut trace = format!("source={PAGE:?}\nregistry: pages item_key=id page=any\n");
    let mut s = stream_fixture(PAGE, false, false, Some("id"));
    let out = load(&mut s);
    trace_step(&mut trace, "first-demand", &s, &out, "p");
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![page_answer(&r, None, Err("first failed"))]);
    trace_step(&mut trace, "first-error", &s, &out, "p");
    commit(&mut s, &out);
    let out = auto(&mut s, 1);
    trace_step(
        &mut trace,
        "auto-no-error-retry-consumes-token",
        &s,
        &out,
        "p",
    );
    let out = act(&mut s, 1, 1);
    trace_step(&mut trace, "explicit-first-retry", &s, &out, "p");
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![page_answer(
        &r,
        None,
        Ok(r#"{"items":[{"id":1}],"next":"a","total":4}"#),
    )]);
    trace_step(&mut trace, "valid-first", &s, &out, "p");
    commit(&mut s, &out);
    let out = auto(&mut s, 2);
    trace_step(&mut trace, "automatic-more", &s, &out, "p");
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![page_answer(
        &r,
        Some("a"),
        Ok(r#"{"items":[{"id":1}],"next":"b"}"#),
    )]);
    trace_step(&mut trace, "duplicate-only", &s, &out, "p");
    commit(&mut s, &out);
    let out = auto(&mut s, 3);
    trace_step(&mut trace, "no-auto-without-progress", &s, &out, "p");
    let out = act(&mut s, 1, 2);
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![Event::Timer(Timer::Timeout(r.2))]);
    trace_step(&mut trace, "later-timeout", &s, &out, "p");
    commit(&mut s, &out);
    let out = act(&mut s, 1, 3);
    trace_step(&mut trace, "later-retry-same-cursor", &s, &out, "p");
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![page_answer(
        &r,
        Some("b"),
        Ok(r#"{"items":[{"id":2}],"next":"c","total":2}"#),
    )]);
    trace_step(&mut trace, "valid-later", &s, &out, "p");
    commit(&mut s, &out);
    let out = auto(&mut s, 4);
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![page_answer(
        &r,
        Some("c"),
        Ok(r#"{"items":[],"next":"d"}"#),
    )]);
    trace_step(&mut trace, "empty-nonfinal", &s, &out, "p");
    commit(&mut s, &out);
    let out = act(&mut s, 1, 4);
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![Event::Connection(false)]);
    trace_step(&mut trace, "disconnect-pending", &s, &out, "p");
    let out = s.batch(vec![page_answer(
        &r,
        Some("d"),
        Ok(r#"{"items":[{"id":99}],"next":null}"#),
    )]);
    trace_step(&mut trace, "late-epoch", &s, &out, "p");
    let out = s.batch(vec![Event::Connection(true)]);
    trace_step(&mut trace, "reconnect-pending", &s, &out, "p");
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![page_answer(
        &r,
        Some("d"),
        Ok(r#"{"items":[],"next":null}"#),
    )]);
    trace_step(&mut trace, "empty-final", &s, &out, "p");
    assert_field(&s, "p", "items", "[{\"id\":1},{\"id\":2}]");
    assert_field(&s, "p", "done", "true");
    commit(&mut s, &out);
    let out = act(&mut s, 0, 5);
    trace_step(&mut trace, "refresh-done-stale", &s, &out, "p");
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![page_answer(
        &r,
        None,
        Ok(r#"{"items":[],"next":null}"#),
    )]);
    trace_step(&mut trace, "empty-first-replaces", &s, &out, "p");
    commit(&mut s, &out);
    let out = act(&mut s, 0, 6);
    let r = pages(&out)[0].0.clone();
    let out = s.batch(vec![page_answer(&r, None, Ok(r#"{"items":[],"next":9}"#))]);
    trace_step(&mut trace, "invalid-cursor", &s, &out, "p");
    commit(&mut s, &out);
    let out = act(&mut s, 1, 7);
    trace_step(&mut trace, "invalid-requires-refresh", &s, &out, "p");
    let out = act(&mut s, 0, 8);
    let r = pages(&out)[0].0.clone();
    let oversized = format!(
        r#"{{"items":{},"next":"cap"}}"#,
        serde_json::to_string(
            &(0..2001)
                .map(|id| serde_json::json!({"id":id}))
                .collect::<Vec<_>>()
        )
        .unwrap()
    );
    let out = s.batch(vec![page_answer(&r, None, Ok(&oversized))]);
    trace_step(&mut trace, "cap-without-partial-append", &s, &out, "p");
    assert_field(&s, "p", "items", "[]");
    assert_field(&s, "p", "capped", "true");
    commit(&mut s, &out);
    let out = act(&mut s, 1, 9);
    trace_step(&mut trace, "capped-load-ignored", &s, &out, "p");
    let out = act(&mut s, 0, 10);
    trace_step(&mut trace, "refresh-capped", &s, &out, "p");
    let mut r = pages(&out)[0].0.clone();
    writeln!(
        trace,
        "page-cap fixture: twenty empty pages with next=1..20, explicit LoadMore between each"
    )
    .unwrap();
    for i in 0..20 {
        let cursor = (i > 0).then(|| i.to_string());
        let out = s.batch(vec![page_answer(
            &r,
            cursor.as_deref(),
            Ok(&format!(r#"{{"items":[],"next":"{}"}}"#, i + 1)),
        )]);
        commit(&mut s, &out);
        if i == 19 {
            trace_step(&mut trace, "reached-page-cap", &s, &out, "p");
        } else {
            r = pages(&act(&mut s, 1, i as u64 + 11))[0].0.clone();
        }
    }
    assert_field(&s, "p", "capped", "true");
    let out = s.batch(vec![Event::Foreground(false)]);
    trace_step(&mut trace, "hidden-capped", &s, &out, "p");
    let out = s.batch(vec![Event::Foreground(true)]);
    trace_step(&mut trace, "visible-capped-no-pending", &s, &out, "p");

    let source =
        "export const p = PagedList(\"pages\", {n: now})\n\n{now < 2 ? p.status : \"hidden\"}";
    writeln!(trace, "identity fixture source={source:?}").unwrap();
    let mut s = stream_fixture(source, false, false, None);
    let r = pages(&load(&mut s))[0].0.clone();
    s.batch(vec![page_answer(
        &r,
        None,
        Ok(r#"{"items":[1],"next":null}"#),
    )]);
    let out = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(1).unwrap(),
        zone: text("UTC"),
    }]);
    trace_step(&mut trace, "new-identity-clears", &s, &out, "p");
    let out = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(2).unwrap(),
        zone: text("UTC"),
    }]);
    trace_step(&mut trace, "undemand-forgets", &s, &out, "p");
    let out = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(1).unwrap(),
        zone: text("UTC"),
    }]);
    trace_step(&mut trace, "redemand-first", &s, &out, "p");
    trace
}

#[test]
fn frozen_watch_trace_empty_live_ended_and_byte_overflow_replays_twice() {
    let actual = watch_replay();
    assert_eq!(actual, watch_replay());
    if actual != include_str!("golden/watch-lifecycle.trace") {
        panic!("golden mismatch:\n{actual}");
    }
}

#[test]
fn frozen_paging_transition_trace_replays_twice() {
    let actual = paging_replay();
    assert_eq!(actual, paging_replay());
    if actual != include_str!("golden/paging-lifecycle.trace") {
        panic!("golden mismatch:\n{actual}");
    }
}

#[test]
fn watch_application_is_interface_wide_and_obsolete_ticks_cannot_flush_new_notices() {
    let source = "export const w = Watch(\"events\", {n:1})\nexport const x = Watch(\"events\", {n:2})\n\n{w.status} {x.status}";
    let mut s = stream_fixture(source, true, false, None);
    let out = load(&mut s);
    let w = watches(&out)[0].0.clone();
    let x = watches(&out)[1].0.clone();
    s.batch(vec![ack(&w, 0), ack(&x, 0)]);
    let out = s.batch(vec![notice(&w, 1, 1, "[1]"), notice(&x, 1, 1, "[2]")]);
    let timer = armed(&out, |t| matches!(t, Timer::WatchApply(_, _)));
    assert_eq!(
        out.effects
            .iter()
            .filter(|e| matches!(e, Effect::Arm(Timer::WatchApply(_, _), _)))
            .count(),
        1
    );
    let out = s.batch(vec![Event::Timer(timer.clone())]);
    assert_eq!(out.evaluations, 1);
    assert_field(&s, "w", "value", "[1]");
    assert_field(&s, "x", "value", "[2]");
    s.batch(vec![notice(&w, 2, 2, "[3]")]);
    assert_eq!(s.batch(vec![Event::Timer(timer)]).evaluations, 0);
    assert_field(&s, "w", "value", "[1]");
    let timer = s.queries.apply_timer.clone().unwrap();
    s.batch(vec![Event::Timer(timer)]);
    assert_field(&s, "w", "value", "[1,3]");
}

#[test]
fn expanded_notice_limit_is_independent_of_item_wire_and_credit_limits() {
    let mut s = stream_fixture(WATCH, true, false, None);
    let r = watches(&load(&mut s))[0].0.clone();
    s.batch(vec![ack(&r, 0)]);
    // Lists of many tiny numbers expand more than their JSON spelling; each
    // item stays tiny while the notice alone crosses 128 KiB.
    let payload = serde_json::to_string(&vec![0; 8192]).unwrap();
    assert!(payload.len() <= 32 * 1024);
    s.batch(vec![notice(&r, 1, 1, &payload)]);
    assert_field(&s, "w", "error", "\"too_large\"");
    assert_field(&s, "w", "value", "[]");
}

#[test]
fn scheduler_timers_have_exact_delays_and_answered_sites_do_not_become_offline() {
    let source = "export const q = Query(\"read\", {}, 0)\nexport const w = Watch(\"events\", {})\nexport const p = PagedList(\"pages\", {})\n\n{q.status} {w.status} {p.status}";
    let mut s = stream_fixture(source, true, false, None);
    let out = load(&mut s);
    let q = issues(&out)[0].clone();
    let w = watches(&out)[0].0.clone();
    let p = pages(&out)[0].0.clone();
    for r in [&q, &w, &p] {
        assert!(out.effects.iter().any(|e| matches!(e, Effect::Arm(Timer::Timeout(epoch), delay) if *epoch == r.2 && delay.to_string() == "30")));
    }
    s.batch(vec![
        answer(&q, Err("provider error")),
        end(&w, Some("provider error")),
        page_answer(&p, None, Err("provider error")),
    ]);
    let out = s.batch(vec![Event::Visibility(false)]);
    assert!(out
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Arm(Timer::Hidden(_), delay) if delay.to_string() == "2")));
    let out = s.batch(vec![Event::Connection(false)]);
    assert!(out
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Arm(Timer::Offline(_), delay) if delay.to_string() == "30")));
    let timer = armed(&out, |t| matches!(t, Timer::Offline(_)));
    s.batch(vec![Event::Timer(timer)]);
    for site in ["q", "w", "p"] {
        assert_field(&s, site, "error", "\"provider error\"");
    }
}

#[test]
fn watch_wire_cap_accepts_exactly_32_kib_and_rejects_one_more_byte() {
    // Padding the same single-number JSON list changes only its wire size;
    // its 32 logical bytes stay well below notice and item expansion caps.
    for extra_byte in [false, true] {
        let mut s = stream_fixture(WATCH, true, false, None);
        let r = watches(&load(&mut s))[0].0.clone();
        s.batch(vec![ack(&r, 0)]);
        let wire_size = 32 * 1024 + usize::from(extra_byte);
        let payload = format!("[1]{}", " ".repeat(wire_size - 3));
        assert_eq!(payload.len(), wire_size);
        let out = s.batch(vec![notice(&r, 1, 1, &payload)]);
        if extra_byte {
            assert_field(&s, "w", "status", "\"error\"");
            assert_field(&s, "w", "error", "\"too_large\"");
            assert_field(&s, "w", "value", "[]");
            assert!(out.effects.iter().any(|effect| matches!(
                effect,
                Effect::Cancel { epoch, .. } if *epoch == r.2
            )));
        } else {
            let out = s.batch(vec![tick(&s, &r)]);
            assert_field(&s, "w", "status", "\"live\"");
            assert_field(&s, "w", "error", "null");
            assert_field(&s, "w", "value", "[1]");
            assert_eq!(field(&s, "w", "value").logical_size(), 32);
            assert!(out.effects.iter().any(|effect| matches!(
                effect,
                Effect::Credit {
                    notices: 1,
                    bytes: 32768,
                    ..
                }
            )));
        }
    }
}

#[test]
fn exact_watch_wire_expanded_and_item_boundaries_and_serialized_credit() {
    let mut s = stream_fixture(WATCH, true, false, None);
    let r = watches(&load(&mut s))[0].0.clone();
    s.batch(vec![ack(&r, 0)]);
    let padded = format!("[1]{}", " ".repeat(32765));
    s.batch(vec![notice(&r, 1, 1, &padded)]);
    let out = s.batch(vec![tick(&s, &r)]);
    assert!(out.effects.iter().any(|e| matches!(
        e,
        Effect::Credit {
            notices: 1,
            bytes: 32768,
            ..
        }
    )));
    let boundary_item = serde_json::to_string(&vec!["x".repeat(2044)]).unwrap();
    s.batch(vec![notice(&r, 2, 2, &boundary_item)]);
    s.batch(vec![tick(&s, &r)]);
    assert_field(&s, "w", "status", "\"live\"");
    let boundary_notice = serde_json::to_string(&vec![0; 8191]).unwrap();
    s.batch(vec![notice(&r, 3, 3, &boundary_notice)]);
    s.batch(vec![tick(&s, &r)]);
    assert_field(&s, "w", "status", "\"live\"");
    assert_field(&s, "w", "dropped", "7193");
    let clock = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(1).unwrap(),
        zone: text("UTC"),
    }]);
    assert!(!clock
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Credit { .. })));
}

#[test]
fn exact_page_wire_item_and_utf8_cursor_boundaries_and_stale_sentinels() {
    let mut s = stream_fixture(PAGE, false, false, None);
    let r = pages(&load(&mut s))[0].0.clone();
    let cursor = "é".repeat(512);
    let mut payload = format!(r#"{{"items":["{}"],"next":"{cursor}"}}"#, "x".repeat(16380));
    payload.push_str(&" ".repeat(256 * 1024 - payload.len()));
    let ready = s.batch(vec![page_answer(&r, None, Ok(&payload))]);
    assert_field(&s, "p", "status", "\"ready\"");
    commit(&mut s, &ready);
    let obsolete = ready.candidate.unwrap();
    let current = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(1).unwrap(),
        zone: text("UTC"),
    }]);
    commit(&mut s, &current);
    let stale = s.batch(vec![Event::AutoLoadMore {
        instance: 42,
        revision: obsolete.revision,
        control: obsolete.sentinels.keys().next().unwrap().clone(),
        token: 1,
    }]);
    assert_eq!(stale.rejected, [Rejected::Stale]);
    assert!(pages(&stale).is_empty());
    let out = auto(&mut s, 1);
    assert_eq!(pages(&out)[0].1.as_deref(), Some(cursor.as_str()));
}

#[test]
fn watch_changed_identity_clears_resume_buffer_and_unapplied_credit() {
    let source = "export const w = Watch(\"events\", {n:now})\n\n{w.status}";
    let mut s = stream_fixture(source, true, false, None);
    let r = watches(&load(&mut s))[0].0.clone();
    s.batch(vec![ack(&r, 0), notice(&r, 1, 3, "[3]"), tick(&s, &r)]);
    s.batch(vec![notice(&r, 2, 4, "[4]")]);
    let old_timer = s.queries.apply_timer.clone().unwrap();
    let out = s.batch(vec![Event::Clock {
        now: Decimal::from_integer(1).unwrap(),
        zone: text("UTC"),
    }]);
    let (fresh, after) = watches(&out)[0].clone();
    assert!(fresh.2 > r.2);
    assert_eq!(after, None);
    assert_field(&s, "w", "value", "[]");
    assert_field(&s, "w", "dropped", "0");
    assert!(out
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Cancel { epoch, .. } if *epoch == r.2)));
    assert_eq!(
        s.batch(vec![
            Event::Timer(old_timer),
            ack(&r, 0),
            end(&r, None),
            notice(&r, 3, 5, "[5]")
        ])
        .evaluations,
        0
    );
}

#[test]
fn watch_clean_end_requires_refresh_and_replay_stays_stale_until_a_notice() {
    let mut s = stream_fixture(WATCH, false, true, None);
    let r = watches(&load(&mut s))[0].0.clone();
    s.batch(vec![ack(&r, 0)]);
    assert_eq!(
        s.batch(vec![Event::Timer(Timer::Timeout(r.2))]).evaluations,
        0
    );
    s.batch(vec![Event::Foreground(false)]);
    let out = s.batch(vec![Event::Foreground(true)]);
    let r = watches(&out)[0].0.clone();
    assert_field(&s, "w", "value", "[]");
    assert_field(&s, "w", "stale", "true");
    s.batch(vec![ack(&r, 0)]);
    assert_field(&s, "w", "stale", "true");
    s.batch(vec![notice(&r, 1, 1, "[]"), tick(&s, &r)]);
    assert_field(&s, "w", "stale", "false");
    let out = s.batch(vec![end(&r, None)]);
    commit(&mut s, &out);
    s.batch(vec![Event::Connection(false)]);
    let out = s.batch(vec![Event::Connection(true)]);
    assert!(watches(&out).is_empty());
    assert_field(&s, "w", "status", "\"ended\"");
    assert_eq!(watches(&act(&mut s, 0, 1)).len(), 1);
}

#[test]
fn reload_preserves_host_availability_and_does_not_restart_offline_deadline() {
    let mut s = stream_fixture(PAGE, false, false, None);
    load(&mut s);
    let hidden = s.batch(vec![Event::Visibility(false)]);
    let timer = armed(&hidden, |t| matches!(t, Timer::Hidden(_)));
    s.batch(vec![Event::Timer(timer)]);
    let out = s.batch(vec![Event::Load {
        epoch: 43,
        inputs: Box::new(Inputs::new(Profile::ShowUi)),
        restored: BTreeMap::new(),
    }]);
    assert!(pages(&out).is_empty());
    let out = s.batch(vec![Event::Visibility(true)]);
    assert_eq!(pages(&out).len(), 1);
    let offline = s.batch(vec![Event::Connection(false)]);
    let timer = armed(&offline, |t| matches!(t, Timer::Offline(_)));
    let out = s.batch(vec![Event::Load {
        epoch: 44,
        inputs: Box::new(Inputs::new(Profile::ShowUi)),
        restored: BTreeMap::new(),
    }]);
    assert!(!out
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Arm(Timer::Offline(_), _))));
    s.batch(vec![Event::Timer(timer)]);
    assert_field(&s, "p", "error", "\"offline\"");
}

#[test]
fn retained_roots_expose_watch_pages_pending_notices_and_both_tree_frames() {
    let source = "export const w = Watch(\"events\", {})\nexport const p = PagedList(\"pages\", {})\n\n{w.status} {p.status}";
    let mut s = stream_fixture(source, true, false, None);
    let initial = load(&mut s);
    let w = watches(&initial)[0].0.clone();
    let p = pages(&initial)[0].0.clone();
    commit(&mut s, &initial);
    s.batch(vec![
        ack(&w, 0),
        page_answer(&p, None, Ok(r#"{"items":[7],"next":null}"#)),
        notice(&w, 1, 1, "[1,2]"),
    ]);
    let sizes: Vec<_> = s
        .retained_buffers()
        .map(|(_, kind, value)| (kind, value.logical_size()))
        .collect();
    assert_eq!(
        sizes,
        [
            ("watch_buffer", 16),
            ("pending_notice", 48),
            ("retained_pages", 32)
        ]
    );
    assert_eq!(
        s.retained_frames().map(|c| c.revision).collect::<Vec<_>>(),
        [2, 1, 0]
    );
    s.batch(vec![tick(&s, &w)]);
    let sizes: Vec<_> = s
        .retained_buffers()
        .map(|(_, kind, value)| (kind, value.logical_size()))
        .collect();
    assert_eq!(sizes, [("watch_buffer", 48), ("retained_pages", 32)]);
}

#[test]
fn invalid_total_can_retry_explicitly_without_resetting_the_cursor() {
    let mut s = stream_fixture(PAGE, false, false, None);
    let r = pages(&load(&mut s))[0].0.clone();
    let out = s.batch(vec![page_answer(
        &r,
        None,
        Ok(r#"{"items":[],"next":null,"total":-1}"#),
    )]);
    assert_field(&s, "p", "error", "\"invalid_page\"");
    commit(&mut s, &out);
    assert!(pages(&auto(&mut s, 1)).is_empty());
    let r = pages(&act(&mut s, 1, 1))[0].clone();
    assert_eq!(r.1, None);
    s.batch(vec![page_answer(
        &r.0,
        None,
        Ok(r#"{"items":[],"next":null,"total":0}"#),
    )]);
    assert_field(&s, "p", "status", "\"ready\"");
}

#[test]
fn sentinel_ignored_token_remains_consumed_after_progress_returns() {
    let mut s = stream_fixture(PAGE, false, false, None);
    let r = pages(&load(&mut s))[0].0.clone();
    let out = s.batch(vec![page_answer(
        &r,
        None,
        Ok(r#"{"items":[1],"next":"a"}"#),
    )]);
    commit(&mut s, &out);
    let out = auto(&mut s, 1);
    let r = pages(&out)[0].0.clone();
    assert!(pages(&auto(&mut s, 2)).is_empty());
    let out = s.batch(vec![page_answer(
        &r,
        Some("a"),
        Ok(r#"{"items":[2],"next":"b"}"#),
    )]);
    commit(&mut s, &out);
    assert!(pages(&auto(&mut s, 2)).is_empty());
    assert_eq!(pages(&auto(&mut s, 3)).len(), 1);
}

#[test]
fn declared_watch_item_and_page_schemas_are_checked_before_accepting_values() {
    for (payload, expected) in [(r#"["ok"]"#, "\"live\""), ("[7]", "\"error\"")] {
        let mut s = stream_fixture(WATCH, true, false, None);
        let Operation::Watch { item, .. } = s.queries.reads.registry.0.get_mut("events").unwrap()
        else {
            panic!()
        };
        *item = serde_json::from_str(r#"{"type":"string"}"#).unwrap();
        let r = watches(&load(&mut s))[0].0.clone();
        s.batch(vec![ack(&r, 0), notice(&r, 1, 1, payload)]);
        if s.queries.apply_timer.is_some() {
            s.batch(vec![tick(&s, &r)]);
        }
        assert_field(&s, "w", "status", expected);
        if expected == "\"error\"" {
            assert_field(&s, "w", "value", "[]");
        }
    }
    for invalid_cursor in [false, true] {
        let mut s = stream_fixture(PAGE, false, false, None);
        let Operation::Paged { page, .. } = s.queries.reads.registry.0.get_mut("pages").unwrap()
        else {
            panic!()
        };
        *page = serde_json::from_str(r#"{"type":"object","properties":{"next":{"type":"string","nullable":true},"tag":{"enum":["ok"]}},"required":["tag"]}"#).unwrap();
        let r = pages(&load(&mut s))[0].0.clone();
        let payload = if invalid_cursor {
            r#"{"items":[],"next":7,"tag":"ok"}"#
        } else {
            r#"{"items":[],"next":null}"#
        };
        let out = s.batch(vec![page_answer(&r, None, Ok(payload))]);
        assert_field(&s, "p", "error", "\"invalid_page\"");
        commit(&mut s, &out);
        let retry = act(&mut s, 1, 1);
        let r = if invalid_cursor {
            assert!(pages(&retry).is_empty());
            pages(&act(&mut s, 0, 2))[0].0.clone()
        } else {
            pages(&retry)[0].0.clone()
        };
        s.batch(vec![page_answer(
            &r,
            None,
            Ok(r#"{"items":[],"next":null,"tag":"ok"}"#),
        )]);
        assert_field(&s, "p", "status", "\"ready\"");
    }
}
