//! What entry points hand back: revisions and measurements as JSON, and the
//! library's errors as statuses with, for REFUSED and REJECTED, error JSON.
//!
//! Revisions and measurements are encoded from their public fields under
//! the library's field names. Decimals are canonical decimal strings,
//! enumerations lower-case (snake_case) strings, and trees tree JSON (see
//! [`crate::tree`]).
//!
//! Error JSON is `{"error":"refused","refused":{"kind":…}}` for a refusal
//! and `{"error":"rejected","diagnostics":[…]}` for a rejection.

use occipital_lang::{
    AccountScope, Diagnostic, Height, HostError, LayoutKey, MeasuredBox, Measurement, Network,
    Placement, Refused, RenderState, ReplyError, ReplyRevision, ReservationError,
    ReservationOutcome, RowFrame, ScopeError, SourceSpan,
};
use serde::Serialize;
use serde_json::json;

use crate::boundary::Failure;
use crate::decimal;
use crate::status::{
    OCC_INTERNAL, OCC_INVALID_ARGUMENT, OCC_REFUSED, OCC_REJECTED, OCC_SESSION_STATE,
};
use crate::tree::TreeJson;

/// `value` as JSON text; an encoder failure is `OCC_INTERNAL`.
pub(crate) fn to_json(value: &impl Serialize) -> Result<String, Failure> {
    serde_json::to_string(value).map_err(|_| Failure::internal())
}

/// A reply revision's JSON.
#[derive(Serialize)]
pub(crate) struct RevisionJson<'a> {
    id: u64,
    provisional: bool,
    tree: TreeJson<'a>,
    literals: &'a [SourceSpan],
    diagnostics: &'a [Diagnostic],
    plain_text: &'a str,
    readable_text: &'a str,
}

impl<'a> RevisionJson<'a> {
    pub(crate) fn new(revision: &'a ReplyRevision) -> Self {
        Self {
            id: revision.id,
            provisional: revision.provisional,
            tree: TreeJson(&revision.tree),
            literals: &revision.literals,
            diagnostics: &revision.diagnostics,
            plain_text: revision.plain_text.as_str(),
            readable_text: revision.readable_text.as_str(),
        }
    }
}

/// A revision's JSON, or JSON `null` for no revision.
pub(crate) fn optional_revision(revision: Option<&ReplyRevision>) -> Result<String, Failure> {
    to_json(&revision.map(RevisionJson::new))
}

/// A measurement's JSON.
#[derive(Serialize)]
pub(crate) struct MeasurementJson {
    height: HeightJson,
    key: KeyJson,
    known_work_visits: u64,
    reservation: serde_json::Value,
    boxes: Vec<BoxJson>,
    row_frame: Option<RowFrameJson>,
}

/// `{"known":"180"}` or `"prose_deferred"`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum HeightJson {
    Known(String),
    ProseDeferred,
}

#[derive(Serialize)]
struct KeyJson {
    instance_epoch: u64,
    revision: u64,
    state_hash: String,
    width: String,
    content_size_category: String,
    catalog_digest: String,
    layout_version: String,
    network: &'static str,
    placement: &'static str,
}

#[derive(Serialize)]
struct BoxJson {
    path: String,
    height: String,
    width: String,
    state: serde_json::Value,
}

#[derive(Serialize)]
struct RowFrameJson {
    baseline: HeightJson,
    height: HeightJson,
    clipped: Option<bool>,
}

fn height(height: Height) -> HeightJson {
    match height {
        Height::Known(value) => HeightJson::Known(decimal::encode(value)),
        Height::ProseDeferred => HeightJson::ProseDeferred,
    }
}

fn key(key: &LayoutKey) -> KeyJson {
    KeyJson {
        instance_epoch: key.instance_epoch,
        revision: key.revision,
        state_hash: key.state_hash.clone(),
        width: decimal::encode(key.width),
        content_size_category: key.content_size_category.clone(),
        catalog_digest: key.catalog_digest.clone(),
        layout_version: key.layout_version.clone(),
        network: match key.network {
            Network::Yes => "yes",
            Network::No => "no",
            Network::Unknown => "unknown",
        },
        placement: match key.placement {
            Placement::Row => "row",
            Placement::Page => "page",
        },
    }
}

fn measured_box(measured: &MeasuredBox) -> BoxJson {
    BoxJson {
        path: measured.path.clone(),
        height: decimal::encode(measured.height),
        width: decimal::encode(measured.width),
        state: match &measured.state {
            RenderState::Ready => json!("ready"),
            RenderState::Loading => json!("loading"),
            RenderState::Held => json!("held"),
            RenderState::Unavailable => json!("unavailable"),
            RenderState::Error(code) => json!({ "error": code.as_str() }),
        },
    }
}

fn row_frame(frame: &RowFrame) -> RowFrameJson {
    RowFrameJson {
        baseline: height(frame.baseline),
        height: height(frame.height),
        clipped: frame.clipped,
    }
}

impl MeasurementJson {
    pub(crate) fn new(measurement: &Measurement) -> Self {
        Self {
            height: height(measurement.height),
            key: key(&measurement.key),
            known_work_visits: measurement.known_work_visits,
            reservation: match measurement.reservation {
                ReservationOutcome::Deferred => json!("deferred"),
                ReservationOutcome::Abort(abort) => json!({ "abort": abort_code(abort) }),
            },
            boxes: measurement.boxes.iter().map(measured_box).collect(),
            row_frame: measurement.row_frame.as_ref().map(row_frame),
        }
    }
}

/// The §12.4 code of an evaluation abort.
fn abort_code(abort: occipital_lang::Abort) -> &'static str {
    use occipital_lang::Abort;
    match abort {
        Abort::Fuel => "E_FUEL",
        Abort::LimitString => "E_LIMIT_STRING",
        Abort::LimitList => "E_LIMIT_LIST",
        Abort::LimitRecord => "E_LIMIT_RECORD",
        Abort::LimitDepth => "E_LIMIT_DEPTH",
        Abort::LimitValue => "E_LIMIT_VALUE",
        Abort::LimitAlloc => "E_LIMIT_ALLOC",
        Abort::LimitNodes => "E_LIMIT_NODES",
        Abort::LimitText => "E_LIMIT_TEXT",
        Abort::LimitLayout => "E_LIMIT_LAYOUT",
    }
}

fn scope(scope: AccountScope) -> &'static str {
    match scope {
        AccountScope::Interface => "interface",
        AccountScope::Reply => "reply",
        AccountScope::Chat => "chat",
        AccountScope::App => "app",
    }
}

/// A refusal as the `refused` member of its error JSON.
fn refused_json(refused: Refused) -> serde_json::Value {
    match refused {
        Refused::UnsupportedMode => json!({ "kind": "unsupported_mode" }),
        Refused::UnitFuel => json!({ "kind": "unit_fuel" }),
        Refused::Aggregate(exhausted) => {
            json!({ "kind": "aggregate", "scope": scope(exhausted.scope) })
        }
        Refused::Scope(error) => {
            let (handle, actual) = match error {
                ScopeError::Local(actual) => ("local", actual),
                ScopeError::Chat(actual) => ("chat", actual),
                ScopeError::App(actual) => ("app", actual),
            };
            json!({ "kind": "scope", "handle": handle, "actual": scope(actual) })
        }
        Refused::Reservation(error) => json!({
            "kind": "reservation",
            "reason": match error {
                ReservationError::RetainedIncrease => "retained_increase",
                ReservationError::FuelExceedsReserved => "fuel_exceeds_reserved",
                ReservationError::ExpansionExceedsReserved => "expansion_exceeds_reserved",
            },
        }),
        Refused::ArithmeticOverflow => json!({ "kind": "arithmetic_overflow" }),
        Refused::MissingRetention => json!({ "kind": "missing_retention" }),
    }
}

/// `OCC_REFUSED` with the refusal's error JSON.
pub(crate) fn refused(refused: Refused) -> Failure {
    Failure {
        status: OCC_REFUSED,
        payload: serde_json::to_string(&json!({
            "error": "refused",
            "refused": refused_json(refused),
        }))
        .ok(),
    }
}

/// `OCC_REJECTED` with the diagnostics' error JSON.
pub(crate) fn rejected(diagnostics: &[Diagnostic]) -> Failure {
    Failure {
        status: OCC_REJECTED,
        payload: serde_json::to_string(&json!({
            "error": "rejected",
            "diagnostics": diagnostics,
        }))
        .ok(),
    }
}

/// The status of a reply session error. Finishing twice and sending an
/// event before finishing are session state errors; an account refusal is
/// REFUSED. A wrong profile or catalog cannot happen, because the FFI always
/// passes the reply profile and the embedded catalog, so it is internal.
pub(crate) fn reply_failure(error: ReplyError) -> Failure {
    match error {
        ReplyError::Finished | ReplyError::NotFinished => Failure::bare(OCC_SESSION_STATE),
        ReplyError::Account(error) => refused(error),
        ReplyError::Profile | ReplyError::Catalog => Failure::internal(),
    }
}

/// The status of a host request error: REJECTED and REFUSED with their error
/// JSON, a host value that cannot become a language value as an invalid
/// argument, and a broken library invariant as `OCC_INTERNAL`.
pub(crate) fn host_failure(error: HostError) -> Failure {
    match error {
        HostError::Rejected(diagnostics) => rejected(&diagnostics),
        HostError::Refused(error) => refused(error),
        HostError::InvalidHostValue(_) => Failure::bare(OCC_INVALID_ARGUMENT),
        HostError::Internal(_) => Failure::bare(OCC_INTERNAL),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::string_entry;
    use occipital_lang::{AccountExhausted, DiagnosticCode, DiagnosticPhase};
    use std::ffi::{c_char, CStr};
    use std::ptr;

    /// Runs `failure` through the real `char **out` boundary and returns the
    /// status and whatever `*out` then holds.
    fn through_boundary(failure: Failure) -> (i32, Option<serde_json::Value>) {
        let mut out: *mut c_char = ptr::NonNull::dangling().as_ptr();
        // SAFETY: `out` is a valid local.
        let status = unsafe { string_entry(&mut out, || Err(failure)) };
        if out.is_null() {
            return (status, None);
        }
        // SAFETY: a non-NULL `out` is a string the boundary returned.
        let text = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_owned();
        // SAFETY: freed exactly once.
        unsafe { crate::occ_string_free(out) };
        (status, Some(serde_json::from_str(&text).unwrap()))
    }

    #[test]
    fn a_rejection_carries_its_diagnostics_in_out() {
        let diagnostic = Diagnostic::new(DiagnosticCode::UnknownName, DiagnosticPhase::Check);
        let (status, out) =
            through_boundary(host_failure(HostError::Rejected(vec![diagnostic.clone()])));
        assert_eq!(status, OCC_REJECTED);
        let out = out.unwrap();
        assert_eq!(out["error"], "rejected");
        assert_eq!(
            out["diagnostics"][0],
            serde_json::to_value(&diagnostic).unwrap()
        );
        assert_eq!(out["diagnostics"][0]["code"], "E_UNKNOWN_NAME");
    }

    #[test]
    fn a_refusal_carries_its_reason_in_out() {
        let exhausted = Refused::Aggregate(AccountExhausted {
            scope: AccountScope::Chat,
        });
        for failure in [
            host_failure(HostError::Refused(exhausted)),
            reply_failure(ReplyError::Account(exhausted)),
        ] {
            let (status, out) = through_boundary(failure);
            assert_eq!(status, OCC_REFUSED);
            assert_eq!(
                out,
                Some(json!({
                    "error": "refused",
                    "refused": {"kind": "aggregate", "scope": "chat"},
                }))
            );
        }
        let (_, out) =
            through_boundary(refused(Refused::Scope(ScopeError::Chat(AccountScope::App))));
        assert_eq!(
            out.unwrap()["refused"],
            json!({"kind": "scope", "handle": "chat", "actual": "app"})
        );
    }

    #[test]
    fn state_and_internal_errors_leave_out_null() {
        for (failure, expected) in [
            (reply_failure(ReplyError::Finished), OCC_SESSION_STATE),
            (reply_failure(ReplyError::NotFinished), OCC_SESSION_STATE),
            (reply_failure(ReplyError::Profile), OCC_INTERNAL),
            (reply_failure(ReplyError::Catalog), OCC_INTERNAL),
            (host_failure(HostError::Internal("broken")), OCC_INTERNAL),
            (
                host_failure(HostError::InvalidHostValue("zone")),
                OCC_INVALID_ARGUMENT,
            ),
        ] {
            assert_eq!(through_boundary(failure), (expected, None));
        }
    }
}
