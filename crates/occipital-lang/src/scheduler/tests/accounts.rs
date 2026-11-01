use super::*;
use crate::accounts::{Accounts, Resources};
use crate::{AccountScope, SharedAccount};

#[test]
fn shared_exhausted_chat_prevents_requests_in_both_sessions_even_after_new_load() {
    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    for _ in 0..3 {
        let accounts = Accounts::new(AccountScope::Interface, chat.clone(), app.clone()).unwrap();
        let mut lease = accounts
            .reserve(Resources {
                fuel: 8_000_000,
                ..Resources::default()
            })
            .unwrap();
        lease.spend(8_000_000, 0).unwrap();
    }
    for _ in 0..2 {
        let mut session = fixture("export const q = Query(\"read\", {}, null)\n\n{q.value}")
            .with_accounts(chat.clone(), app.clone())
            .unwrap();
        assert!(issues(&load(&mut session)).is_empty());
        assert_field(&session, "q", "error", "\"too_large\"");
        let out = session.batch(vec![Event::Load {
            epoch: 43,
            inputs: Box::new(Inputs::new(Profile::ShowUi)),
            restored: BTreeMap::new(),
        }]);
        assert!(issues(&out).is_empty());
    }
    assert_eq!(chat.usage().spent_fuel, 24_000_000);
}

#[test]
fn cancelling_a_site_releases_its_pending_fetch_reservation() {
    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    let mut session = fixture("export const q = Query(\"read\", {}, null)\n\n{q.value}")
        .with_accounts(chat.clone(), app.clone())
        .unwrap();
    let out = load(&mut session);
    assert_eq!(issues(&out).len(), 1);
    let with_pending = chat.usage().retained;
    assert!(with_pending > 1024 * 1024 + 64);
    let mut effects = Vec::new();
    session.queries.cancel_all(&mut effects);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Cancel { .. }))
            .count(),
        1
    );
    assert_eq!(chat.usage().retained, with_pending - (1024 * 1024 + 64));
    assert_eq!(app.usage().retained, chat.usage().retained);
    drop((out, session));
    assert_eq!(chat.usage().retained, 0);
    assert_eq!(app.usage().retained, 0);
}

#[test]
fn query_conversion_charges_preparation_and_eviction_does_not_refill_it() {
    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    let mut session = fixture("export const q = Query(\"read\", {}, null)\n\n{q.value}")
        .with_accounts(chat.clone(), app)
        .unwrap();
    let request = issues(&load(&mut session)).remove(0);
    session.batch(vec![answer(&request, Ok("7"))]);
    assert_eq!(chat.usage().spent_fuel, 3); // One byte and one JSON value (2 fuel).
    assert_eq!(chat.usage().expanded, 16);
    let before = chat.usage();
    session.queries.cancel_all(&mut Vec::new());
    assert_eq!(chat.usage().spent_fuel, before.spent_fuel);
    assert_eq!(chat.usage().expanded, before.expanded);
}

#[test]
fn local_preparation_exhaustion_aborts_program_and_cancels_pending_sites() {
    let mut session = fixture("export const q = Query(\"read\", {}, null)\n\n{q.value}");
    let request = issues(&load(&mut session)).remove(0);
    let mut spent = session
        .queries
        .accounts
        .reserve(Resources {
            fuel: 8_000_000,
            ..Resources::default()
        })
        .unwrap();
    spent.spend(8_000_000, 0).unwrap();
    let out = session.batch(vec![answer(&request, Ok("7"))]);
    assert_eq!(out.abort, Some(eval::Abort::Fuel));
    assert!(out.candidate.is_some());
    assert!(session.queries.shared.is_empty());
    assert!(session.aborted);
    let candidate = out.candidate.unwrap();
    assert!(
        session
            .batch(vec![measure(&candidate)])
            .committed
            .unwrap()
            .fallback
    );
}
