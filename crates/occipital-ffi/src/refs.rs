//! `occ_text_references`: the issue and pull request mentions in a text, for
//! the app to draw as links. The rules are cortexkit-occipital-refs'; this
//! file only decodes the project and encodes the answer.

use std::ffi::c_char;

use occipital_refs::{references, Project, Repository, RULES_VERSION};
use serde::{Deserialize, Serialize};

use crate::boundary::{buffer, json_arg, string_entry, Failure};
use crate::output::to_json;

/// The project record core serves beside an agent. Only `repository` matters
/// to the rules; every other member, today's or a later one, is ignored, so
/// an app can pass the record exactly as it arrived.
#[derive(Deserialize)]
struct ProjectJson {
    #[serde(default)]
    repository: Option<RepositoryJson>,
}

#[derive(Deserialize)]
struct RepositoryJson {
    host: String,
    owner: String,
    name: String,
}

#[derive(Serialize)]
struct ReferencesJson<'a> {
    /// The version of the rules that found these references.
    rules: u32,
    references: Vec<ReferenceJson<'a>>,
}

#[derive(Serialize)]
struct ReferenceJson<'a> {
    start: usize,
    end: usize,
    kind: &'a str,
    target: String,
}

/// The mentions in `text` (`len` bytes of UTF-8, the Markdown source the app
/// draws) under `project_json`, the project record core serves beside the
/// text's agent, or JSON `null` for text with no project. `*out` is
/// `{"rules", "references": [{"start", "end", "kind", "target"}]}`, with
/// `start` and `end` as UTF-8 byte offsets into `text`.
///
/// Text that is not UTF-8 is `OCC_INVALID_ARGUMENT`: byte offsets into
/// repaired text would point at the wrong characters of the app's own copy.
///
/// # Safety
///
/// `text` must be valid for reading `len` bytes (or anything when `len` is
/// 0), `project_json` NULL or a NUL-terminated string, and `out` NULL or
/// valid for writing one pointer, all for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn occ_text_references(
    text: *const u8,
    len: usize,
    project_json: *const c_char,
    out: *mut *mut c_char,
) -> i32 {
    let body = || {
        // SAFETY: the caller's guarantees are those of `buffer` and
        // `json_arg`.
        let (bytes, project) = unsafe {
            (
                buffer(text, len)?,
                json_arg::<Option<ProjectJson>>(project_json)?,
            )
        };
        let text = std::str::from_utf8(bytes).map_err(|_| Failure::invalid())?;
        let repository = project
            .as_ref()
            .and_then(|project| project.repository.as_ref());
        let project = Project {
            repository: repository.map(|repository| Repository {
                host: &repository.host,
                owner: &repository.owner,
                name: &repository.name,
            }),
        };
        let found = references(text, &project);
        to_json(&ReferencesJson {
            rules: RULES_VERSION,
            references: found
                .into_iter()
                .map(|reference| ReferenceJson {
                    start: reference.start,
                    end: reference.end,
                    kind: reference.kind,
                    target: reference.target,
                })
                .collect(),
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

    fn call(text: &[u8], project: Option<&str>) -> (i32, Option<String>) {
        let project = project.map(|json| CString::new(json).unwrap());
        let mut out = ptr::null_mut();
        // SAFETY: every pointer is a valid local for the call.
        let status = unsafe {
            occ_text_references(
                text.as_ptr(),
                text.len(),
                project.as_ref().map_or(ptr::null(), |json| json.as_ptr()),
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

    /// The project record passes as core serves it, other members and all,
    /// and the answer names the rules version.
    #[test]
    fn finds_mentions_under_a_served_project() {
        let project = r#"{"id": "pj-1", "name": "occipital", "roots": ["/x"], "later": 1,
            "repository": {"host": "github.com", "owner": "cortexkit", "name": "occipital", "more": true}}"#;
        let (status, json) = call("Fixed by PR #499, not `PR 1`.".as_bytes(), Some(project));
        assert_eq!(status, OCC_OK);
        assert_eq!(
            json.unwrap(),
            r#"{"rules":1,"references":[{"start":9,"end":16,"kind":"issue","target":"https://github.com/cortexkit/occipital/issues/499"}]}"#
        );
    }

    /// JSON `null` is text with no project: only `owner/repo#N` links.
    #[test]
    fn null_is_text_with_no_project() {
        let (status, json) = call(b"PR 4 and cortexkit/aft#12", Some("null"));
        assert_eq!(status, OCC_OK);
        assert_eq!(
            json.unwrap(),
            r#"{"rules":1,"references":[{"start":9,"end":25,"kind":"repository_issue","target":"https://github.com/cortexkit/aft/issues/12"}]}"#
        );
        let (status, json) = call(b"PR 4", Some("{}"));
        assert_eq!(
            (status, json.as_deref()),
            (OCC_OK, Some(r#"{"rules":1,"references":[]}"#))
        );
    }

    #[test]
    fn refuses_text_that_is_not_utf8_and_a_missing_project() {
        assert_eq!(
            call(b"PR 1 \xff", Some("null")),
            (OCC_INVALID_ARGUMENT, None)
        );
        assert_eq!(call(b"PR 1", None), (OCC_INVALID_ARGUMENT, None));
        assert_eq!(
            call(b"PR 1", Some(r#"{"repository": {"host": "github.com"}}"#)),
            (OCC_INVALID_ARGUMENT, None)
        );
    }
}
