//! `occ_format_fact`: the wording and tone of one fact a module reports, so
//! both apps draw a command preview's facts alike, and `occ_format_schedule`:
//! the wording of a module's recurring schedule. The rules are
//! cortexkit-occipital-lang's `format_fact` and `format_schedule`; this file
//! only decodes and encodes.

use std::ffi::c_char;

use occipital_lang::{format_fact, format_schedule, Tone};
use serde::Serialize;

use crate::boundary::{json_arg, string_entry};
use crate::output::to_json;

#[derive(Serialize)]
#[serde(untagged)]
enum FactJson {
    Formatted {
        label: String,
        text: String,
        tone: Tone,
    },
    Refused {
        error: &'static str,
    },
}

/// Words `fact_json`, one fact as a module sent it, for example
/// `{"label": "Removes", "value": 18200, "unit": "tokens", "approx": true}`.
/// `*out` is `{"label", "text", "tone"}`, such as
/// `{"label": "Removes", "text": "about 18k tokens", "tone": "neutral"}`,
/// or `{"error": "<code>"}` for a fact the rules refuse (`fact_malformed`,
/// `fact_conflict` or `fact_range`), which the app shows as missing. Text
/// that is not JSON is `OCC_INVALID_ARGUMENT`.
///
/// # Safety
///
/// `fact_json` must be NULL or a NUL-terminated string, and `out` NULL or
/// valid for writing one pointer, both for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn occ_format_fact(fact_json: *const c_char, out: *mut *mut c_char) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantee about `fact_json` is `json_arg`'s.
        let fact = unsafe { json_arg::<serde_json::Value>(fact_json)? };
        to_json(&match format_fact(&fact) {
            Ok(formatted) => FactJson::Formatted {
                label: formatted.label,
                text: formatted.text,
                tone: formatted.tone,
            },
            Err(error) => FactJson::Refused {
                error: error.code(),
            },
        })
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

#[derive(Serialize)]
#[serde(untagged)]
enum ScheduleJson {
    Worded { text: String },
    Refused { error: &'static str },
}

/// Words `schedule_json`, a module's schedule as it sent it, in the
/// person's clock: `shift_minutes` is the device's UTC offset minus the
/// offset of the schedule's `time_zone`, both now, which the app computes
/// from the zone name. `*out` is `{"text"}`, such as
/// `{"text": "Mondays and Thursdays at 09:30"}`, or `{"error": "<code>"}`:
/// `schedule_unworded` for a cron-only schedule, `schedule_malformed` for
/// any other shape. The app then shows its own fallback. Text that is not
/// JSON is `OCC_INVALID_ARGUMENT`.
///
/// # Safety
///
/// `schedule_json` must be NULL or a NUL-terminated string, and `out` NULL
/// or valid for writing one pointer, both for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn occ_format_schedule(
    schedule_json: *const c_char,
    shift_minutes: i32,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantee about `schedule_json` is `json_arg`'s.
        let schedule = unsafe { json_arg::<serde_json::Value>(schedule_json)? };
        to_json(&match format_schedule(&schedule, shift_minutes) {
            Ok(text) => ScheduleJson::Worded { text },
            Err(error) => ScheduleJson::Refused {
                error: error.code(),
            },
        })
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{occ_string_free, OCC_INVALID_ARGUMENT, OCC_OK};
    use std::ffi::{CStr, CString};
    use std::ptr;

    fn call(fact: Option<&str>) -> (i32, Option<String>) {
        let fact = fact.map(|json| CString::new(json).unwrap());
        let mut out = ptr::null_mut();
        // SAFETY: every pointer is a valid local for the call.
        let status = unsafe {
            occ_format_fact(
                fact.as_ref().map_or(ptr::null(), |json| json.as_ptr()),
                &mut out,
            )
        };
        let json = (!out.is_null()).then(|| {
            // SAFETY: a non-NULL `out` is a string this library returned.
            let text = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_owned();
            // SAFETY: as above, freed once.
            unsafe { occ_string_free(out) };
            text
        });
        (status, json)
    }

    #[test]
    fn words_a_number_and_a_state() {
        assert_eq!(
            call(Some(
                r#"{"label": "Removes", "value": 18200, "unit": "tokens", "approx": true}"#
            )),
            (
                OCC_OK,
                Some(r#"{"label":"Removes","text":"about 18k tokens","tone":"neutral"}"#.into())
            )
        );
        assert_eq!(
            call(Some(
                r#"{"label": "Code map", "state": "building", "progress": 0.4}"#
            )),
            (
                OCC_OK,
                Some(r#"{"label":"Code map","text":"Building, 40%","tone":"info"}"#.into())
            )
        );
    }

    /// A fact the rules refuse is an answer, with its code, so the app can
    /// show it as missing and log why; only text that isn't JSON is an
    /// invalid argument.
    #[test]
    fn a_refused_fact_answers_its_code() {
        assert_eq!(
            call(Some(r#"{"label": "x", "value": "ready"}"#)),
            (OCC_OK, Some(r#"{"error":"fact_malformed"}"#.into()))
        );
        assert_eq!(
            call(Some(
                r#"{"label": "x", "state": "ready", "tone": "danger"}"#
            )),
            (OCC_OK, Some(r#"{"error":"fact_conflict"}"#.into()))
        );
        assert_eq!(call(Some("{")), (OCC_INVALID_ARGUMENT, None));
        assert_eq!(call(None), (OCC_INVALID_ARGUMENT, None));
    }
}
