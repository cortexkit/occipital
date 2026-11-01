//! Checking programs and evaluating tool templates: `occ_program_check` and
//! `occ_template_evaluate`.

use std::ffi::c_char;

use occipital_lang::{check_program, evaluate_template, Diagnostic, TrialOutcome};
use serde::Serialize;

use crate::boundary::{buffer, json_arg, string_entry};
use crate::envelope::CheckEnvelope;
use crate::output::{host_failure, to_json};
use crate::tree::TreeJson;

/// Checks the program source in `src` (`len` bytes, passed to the library
/// unvalidated) under `ctx_json`, `{"context", "host"}`, and runs its trial
/// evaluation. `*out` is the library's §12.2 check result JSON.
///
/// A check that completes is `OCC_OK` whatever it found: error diagnostics
/// are in the result, and invalid UTF-8 in the source is repaired and
/// reported there as `W_ENCODING`. A refused check is `OCC_REFUSED` with its
/// error JSON, and a host value that cannot become a language value (such
/// as an over-long zone) is `OCC_INVALID_ARGUMENT`.
///
/// # Safety
///
/// `src` must be valid for reading `len` bytes (or anything when `len` is
/// 0), `ctx_json` NULL or a NUL-terminated string, and `out` NULL or valid
/// for writing one pointer, all for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn occ_program_check(
    src: *const u8,
    len: usize,
    ctx_json: *const c_char,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `buffer` and
        // `json_arg`.
        let (source, envelope) =
            unsafe { (buffer(src, len)?, json_arg::<CheckEnvelope>(ctx_json)?) };
        let (context, host) = envelope.into_parts()?;
        let result = check_program(source, &context, &host).map_err(host_failure)?;
        to_json(&result)
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

/// One evaluation of a tool template, as `occ_template_evaluate` returns it.
#[derive(Serialize)]
struct TemplateJson<'a> {
    /// Tree JSON, or null when the evaluation aborted.
    tree: Option<TreeJson<'a>>,
    diagnostics: &'a [Diagnostic],
    /// `ok`, or the abort that stopped the evaluation, as a trial outcome.
    outcome: TrialOutcome,
}

/// Checks the tool template in `src` (`len` bytes) under `ctx_json`, as
/// [`occ_program_check`] does, and evaluates it once with the context's
/// `tool` input and view and the host's time and zone. `*out` is
/// `{"tree", "diagnostics", "outcome"}`.
///
/// A tool template declares no sites, so one evaluation is all an app needs
/// to draw a row or a page; it evaluates again when the view changes. A
/// profile other than `tool_row` or `tool_page` is `OCC_INTERNAL`, a template
/// with an error diagnostic is `OCC_REJECTED` with its diagnostics, and an
/// evaluation that aborts is `OCC_OK` with a null tree and the abort as the
/// outcome.
///
/// # Safety
///
/// As [`occ_program_check`].
#[no_mangle]
pub unsafe extern "C" fn occ_template_evaluate(
    src: *const u8,
    len: usize,
    ctx_json: *const c_char,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `buffer` and
        // `json_arg`.
        let (source, envelope) =
            unsafe { (buffer(src, len)?, json_arg::<CheckEnvelope>(ctx_json)?) };
        let (context, host) = envelope.into_parts()?;
        let evaluated = evaluate_template(source, &context, &host).map_err(host_failure)?;
        to_json(&TemplateJson {
            tree: evaluated.tree.as_ref().map(TreeJson),
            diagnostics: &evaluated.diagnostics,
            outcome: evaluated.outcome(),
        })
    };
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe { string_entry(out, body) }
}

#[cfg(test)]
mod tests;
