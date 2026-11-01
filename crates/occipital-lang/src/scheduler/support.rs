//! Helpers shared by the scheduler's unit tests and its golden traces: a
//! fixed read authority, scripted answers and a fixed-height measurement.
//! None of these values authorize a read; they only make identities concrete.

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

use super::identity::{Authority, ReadVersion};
use super::*;
use crate::{Operation, OperationRegistry, Schema, ValueKind};

pub(super) fn number(n: i128) -> Value {
    Value::number(Decimal::from_integer(n).unwrap())
}
pub(super) fn text(s: &str) -> Value {
    Value::string(s).unwrap()
}

pub(super) fn reads() -> Reads {
    let operation = Operation::Query {
        arguments: Schema::default(),
        result: Schema::default(),
    };
    Reads {
        authority: Authority {
            computer: "computer".into(),
            session: "session".into(),
            scope_epoch: "scope-1".into(),
            provider: "provider".into(),
            entity: "validated-command".into(),
            serving_computer: "computer".into(),
            verified_holder: "computer".into(),
        },
        versions: [(
            "read".into(),
            ReadVersion {
                operation: "operation-1".into(),
                schema: "schema-1".into(),
            },
        )]
        .into(),
        registry: OperationRegistry([("read".into(), operation)].into()),
    }
}

pub(super) fn issues(out: &Output) -> Vec<(String, Identity, Epoch)> {
    out.effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Issue {
                site,
                identity,
                epoch,
            } => Some((site.clone(), identity.clone(), *epoch)),
            _ => None,
        })
        .collect()
}

pub(super) fn answer(request: &(String, Identity, Epoch), answer: Result<&str, &str>) -> Event {
    Event::Answer {
        site: request.0.clone(),
        identity: Box::new(request.1.clone()),
        epoch: request.2,
        answer: answer.map(|s| s.as_bytes().to_vec()).map_err(str::to_owned),
    }
}

pub(super) fn measure(candidate: &Candidate) -> Event {
    Event::MeasureComplete(Measurement {
        key: candidate.layout_key(&candidate.layout),
        height: Height::Known(Decimal::from_integer(60).unwrap()),
        known_work_visits: 0,
        reservation: ReservationOutcome::Deferred,
        boxes: vec![],
        tables: vec![],
        row_frame: None,
    })
}

pub(super) fn layout(width: i128) -> LayoutContext {
    LayoutContext {
        width: Decimal::from_integer(width).unwrap(),
        content_size_category: "normal".into(),
        placement: Placement::Row,
        network: geometry::Network::Yes,
    }
}

pub(super) fn field(session: &Session, site: &str, key: &str) -> Value {
    let site_state = &session.queries.sites[site];
    let query = site_state
        .identity
        .as_ref()
        .and_then(|i| session.queries.shared.get(i))
        .unwrap_or(&site_state.local);
    let ValueKind::SiteEnvelope(envelope) = query.envelope(site).kind().clone() else {
        panic!()
    };
    envelope.members.get(key)
}

pub(super) fn assert_field(session: &Session, site: &str, key: &str, expected: &str) {
    assert_eq!(
        field(session, site, key).canonical_json().unwrap(),
        expected
    );
}
