//! `occ_version`: the version descriptor (language spec §13) of what the
//! library implements, and the tool catalog its tool templates use.

use std::ffi::c_char;

use occipital_lang::{CatalogDescriptor, VersionDescriptor};
use serde::Serialize;

use crate::boundary::{string_entry, Failure};

/// What `occ_version` returns: the §13 descriptor's members, then the tool
/// catalog. The tool catalog is a member of its own, not a second entry
/// beside `catalog`, so an app that reads `catalog` as the reply catalog
/// reads exactly what it read before.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionJson {
    #[serde(flatten)]
    descriptor: VersionDescriptor,
    tool_catalog: CatalogDescriptor,
}

/// The version descriptor as JSON:
/// `{"executionDialect", "catalog": {"version", "digest", "layout", "aliases"},
/// "toolCatalog": {"version", "digest", "layout", "aliases"}}`. `catalog`
/// is the embedded catalog: a stored reply whose stamp names its `digest` or
/// any of its `aliases` draws with it. `toolCatalog` is the catalog tool
/// templates name: a template whose `catalog.digest` is `sha256:` followed by
/// its `digest` or one of its `aliases` is for this library, and a template
/// naming another digest is for a catalog it doesn't implement.
///
/// # Safety
///
/// `out` must be NULL or valid for writing one pointer. A string written to
/// `*out` must be freed with [`occ_string_free`](crate::occ_string_free).
#[no_mangle]
pub unsafe extern "C" fn occ_version(out: *mut *mut c_char) -> i32 {
    // SAFETY: the caller's guarantee about `out` is `string_entry`'s.
    unsafe {
        string_entry(out, || {
            serde_json::to_string(&VersionJson {
                descriptor: VersionDescriptor::current(),
                tool_catalog: CatalogDescriptor::tool_templates(),
            })
            .map_err(|_| Failure::internal())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{occ_string_free, OCC_INVALID_ARGUMENT, OCC_OK};
    use std::ffi::CStr;
    use std::ptr;

    #[test]
    fn returns_the_descriptor_json() {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a valid local.
        assert_eq!(unsafe { occ_version(&mut out) }, OCC_OK);
        // SAFETY: `out` is the NUL-terminated string `occ_version` returned.
        let text = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_owned();
        // SAFETY: freed exactly once.
        unsafe { occ_string_free(out) };

        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let current = VersionDescriptor::current();
        assert_eq!(json["executionDialect"], "1.0");
        assert_eq!(json["executionDialect"], current.execution_dialect.as_str());
        assert_eq!(json["catalog"]["version"], current.catalog.version.as_str());
        assert_eq!(json["catalog"]["layout"], current.catalog.layout.as_str());
        let digest = json["catalog"]["digest"].as_str().unwrap();
        assert_eq!(digest, current.catalog.digest);
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            json["catalog"]["aliases"],
            serde_json::json!(current.catalog.aliases)
        );
        assert_eq!(json.as_object().unwrap().len(), 3);
        assert_eq!(json["catalog"].as_object().unwrap().len(), 4);

        // The tool catalog: what a template's `catalog` must name, here
        // checked against the template checker's own expectation.
        assert_eq!(json["toolCatalog"]["version"], "1.1");
        assert_eq!(
            json["toolCatalog"]["layout"],
            current.catalog.layout.as_str()
        );
        let tool = json["toolCatalog"]["digest"].as_str().unwrap();
        assert_eq!(tool, occipital_lang::Catalog::tool_templates_digest());
        assert_ne!(tool, digest);
        // The digest the tool catalog had before the narrow Slot width.
        assert_eq!(
            json["toolCatalog"]["aliases"],
            serde_json::json!(["9a23139f4f1f48d6ed366a4a48b9221b1e5ac0ccd87473c0b241b8a37a876fbf"])
        );
        assert_eq!(json["toolCatalog"].as_object().unwrap().len(), 4);
    }

    #[test]
    fn a_null_out_is_an_invalid_argument() {
        // SAFETY: NULL is allowed for `out`.
        assert_eq!(
            unsafe { occ_version(ptr::null_mut()) },
            OCC_INVALID_ARGUMENT
        );
    }
}
