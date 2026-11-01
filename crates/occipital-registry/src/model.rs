//! The closed typed model of `occipital.template/1`.
//!
//! The member set is final for version 1: any later change to it is a new
//! schema version. A document is read with [`Template::parse`], and a typed
//! template becomes canonical bytes with [`Template::to_canonical`]. Both
//! directions run the same rules in the same order, so every template the
//! crate serializes parses back to an equal template and re-serializes to
//! identical bytes.
//!
//! # Which fault is reported
//!
//! A document with several faults gets exactly one code. The document is
//! walked depth-first, pre-order: object members in canonical (UTF-16) order
//! and array elements in index order. An absent member sits where its name
//! would sort among its siblings. Tiers are judged in this order, and within
//! a tier the first fault in the walk wins (at one position, the code listed
//! first):
//!
//! 1. Canonical JSON ([`crate::canon::CanonError`]), including a canonical
//!    form over 1,048,576 bytes (`input_too_large`).
//! 2. The root and the top-level `schema`: a root that is not an object is
//!    `wrong_type`; then `missing_field`, `wrong_type`,
//!    `registry_schema_unsupported` and `invalid_value` for `schema`.
//! 3. Shape: `unknown_field`, `missing_field`, `wrong_type`.
//! 4. Values: `invalid_value`, `invalid_id`, `invalid_digest`,
//!    `template_too_large`.
//! 5. Cross-references: `duplicate_entry`, placed at a repeated read's
//!    second occurrence.
//!
//! The crate documents, and does not check, that a template uses only
//! operations in its `reads` (occipital-lang reports `E_UNKNOWN_OPERATION`).

use crate::canon::Integer;

/// The template schema id this crate implements.
pub const TEMPLATE_SCHEMA: &str = "occipital.template/1";

/// The largest template source, in bytes of UTF-8.
pub const MAX_TEMPLATE_SOURCE_BYTES: usize = 65_536;

/// `catalog{version, digest}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRef {
    /// `version`: the catalog version label.
    pub version: String,
    /// `digest`: `sha256:` followed by occipital-lang's bare-hex catalog
    /// digest.
    pub digest: String,
}

/// One `{id, version}` entry of a `reads` array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRef {
    /// `id`: the operation id.
    pub id: String,
    /// `version`: from 1.
    pub version: Integer,
}

/// An `occipital.template/1` document. Its `schema` member is always
/// [`TEMPLATE_SCHEMA`] and is not stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// `profile`: `tool_row` or `tool_page`.
    pub profile: String,
    /// `executionDialect`: the dialect the source is written in, `N.N`.
    pub execution_dialect: String,
    /// `catalog`: the component catalog the source was checked against.
    pub catalog: CatalogRef,
    /// `reads`: the operations the template may use. May be empty.
    pub reads: Vec<ReadRef>,
    /// `source`: the program, at most 65,536 bytes of UTF-8.
    pub source: String,
}
