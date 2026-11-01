//! Helpers shared by the reply unit tests and the reply golden traces: one
//! fixed layout context, a fresh session, a resolved reference record and
//! fixed text metrics, so measured heights are the same on every platform.

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

use super::*;
use crate::geometry::{Metrics, Network, Placement, TextStyle};
use crate::Decimal;

pub(super) fn layout() -> LayoutContext {
    LayoutContext {
        width: Decimal::from_integer(320).unwrap(),
        content_size_category: "normal".into(),
        network: Network::Yes,
        placement: Placement::Page,
    }
}
pub(super) fn session() -> Session {
    Session::new(
        Profile::Reply,
        "1.0".into(),
        Catalog::shared(),
        layout(),
        crate::SharedAccount::chat(),
        crate::SharedAccount::app(),
    )
    .unwrap()
}
pub(super) fn record() -> ReferenceRecord {
    ReferenceRecord {
        state: ReferenceState::Resolved,
        reason: None,
        tool: Some("bash".into()),
        bytes: Some(20),
        call_key: Some("call".into()),
    }
}
pub(super) fn finish(source: &[u8]) -> Revision {
    session().finish(source, BTreeMap::new(), vec![]).unwrap()
}
pub(super) fn same(a: &Revision, b: &Revision) {
    assert!(!a.provisional && !b.provisional);
    assert_eq!(a.tree, b.tree);
    assert_eq!(a.literals, b.literals);
    assert_eq!(a.diagnostics, b.diagnostics);
    assert_eq!(a.plain_text, b.plain_text);
}

pub(super) struct Fixed;
impl Metrics for Fixed {
    fn l(&self, _: TextStyle) -> Decimal {
        Decimal::from_integer(20).unwrap()
    }
    fn t(&self, _: &str, _: TextStyle, _: Decimal, _: Option<u32>) -> Decimal {
        Decimal::from_integer(20).unwrap()
    }
    fn tw(&self, s: &str, _: TextStyle) -> Decimal {
        Decimal::from_integer(s.len() as i128).unwrap()
    }
}
pub(super) fn measure(r: &Revision) -> Measurement {
    geometry::measure(&r.measure_revision(), &layout(), &Fixed).unwrap()
}
