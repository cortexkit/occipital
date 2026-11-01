//! Tiers 4 and 5: value rules and cross-references, judged on the typed model
//! so documents read from bytes and templates built in code get the same
//! codes.
//!
//! Each walk below visits members in canonical (UTF-16) order, as the shape
//! decoder does, and at one position reports codes in their listed order.

use crate::error::RuleCode;
use crate::grammar::{is_dialect, is_operation_id, is_sha256_digest};
use crate::model::{CatalogRef, ReadRef, Template, MAX_TEMPLATE_SOURCE_BYTES};
use crate::walk::{Faults, Path, Tier};

const PROFILE_TOOL_ROW: &str = "tool_row";
const PROFILE_TOOL_PAGE: &str = "tool_page";

fn value(faults: &mut Faults, code: RuleCode, path: &Path<'_>) {
    faults.report(Tier::Value, code, path);
}

/// Checks a whole template document.
pub(crate) fn check_template(template: &Template, faults: &mut Faults) {
    let root = Path::ROOT;
    catalog(&template.catalog, &root.key("catalog"), faults);
    if !is_dialect(&template.execution_dialect) {
        value(
            faults,
            RuleCode::InvalidValue,
            &root.key("executionDialect"),
        );
    }
    if template.profile != PROFILE_TOOL_ROW && template.profile != PROFILE_TOOL_PAGE {
        value(faults, RuleCode::InvalidValue, &root.key("profile"));
    }
    reads(&template.reads, &root.key("reads"), faults);
    if template.source.len() > MAX_TEMPLATE_SOURCE_BYTES {
        value(faults, RuleCode::TemplateTooLarge, &root.key("source"));
    }
}

fn catalog(catalog: &CatalogRef, path: &Path<'_>, faults: &mut Faults) {
    if !is_sha256_digest(&catalog.digest) {
        value(faults, RuleCode::InvalidDigest, &path.key("digest"));
    }
}

/// Checks a `reads` array: each entry's id and version, and repeats.
fn reads(reads: &[ReadRef], path: &Path<'_>, faults: &mut Faults) {
    for (index, read) in reads.iter().enumerate() {
        let entry = path.index(index);
        if reads[..index].contains(read) {
            faults.report(Tier::Cross, RuleCode::DuplicateEntry, &entry);
        }
        if !is_operation_id(&read.id) {
            value(faults, RuleCode::InvalidId, &entry.key("id"));
        }
        if read.version.get() < 1 {
            value(faults, RuleCode::InvalidValue, &entry.key("version"));
        }
    }
}
