//! The one error type every registry refusal uses: a canonical JSON fault, or
//! a broken template rule with the place it was found.

use std::fmt;

use crate::canon::CanonError;

/// A template rule, each with a stable code. [`crate::model`] describes the
/// order in which the rules are judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleCode {
    /// `wrong_type`: a member of the wrong JSON type, including `null`, or a
    /// document root that is not an object.
    WrongType,
    /// `missing_field`: a required member is absent.
    MissingField,
    /// `registry_schema_unsupported`: the top-level `schema` names another
    /// version of the template schema.
    RegistrySchemaUnsupported,
    /// `unknown_field`: a member name the object doesn't have.
    UnknownField,
    /// `invalid_value`: a value outside its set, a read version below 1, or
    /// a malformed dialect.
    InvalidValue,
    /// `invalid_id`: a read id that breaks `^[a-z][a-z0-9_]{0,47}$`.
    InvalidId,
    /// `invalid_digest`: a catalog digest that is not `sha256:` and 64
    /// lowercase hex.
    InvalidDigest,
    /// `template_too_large`: a template source over 65,536 bytes.
    TemplateTooLarge,
    /// `duplicate_entry`: a repeated read.
    DuplicateEntry,
}

impl RuleCode {
    /// The stable code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongType => "wrong_type",
            Self::MissingField => "missing_field",
            Self::RegistrySchemaUnsupported => "registry_schema_unsupported",
            Self::UnknownField => "unknown_field",
            Self::InvalidValue => "invalid_value",
            Self::InvalidId => "invalid_id",
            Self::InvalidDigest => "invalid_digest",
            Self::TemplateTooLarge => "template_too_large",
            Self::DuplicateEntry => "duplicate_entry",
        }
    }
}

/// Why a registry document or typed object was refused. A document with
/// several faults gets exactly one error, chosen as [`crate::model`]
/// describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// The bytes are not canonical-profile JSON, or the canonical form is
    /// over the size or depth bound.
    Canon(CanonError),
    /// The document breaks a manifest or template rule.
    Rule {
        /// Which rule.
        code: RuleCode,
        /// Where, as a JSON Pointer (RFC 6901) into the document: `""` for
        /// the root, `/operations/0/limits/argsBytes` for a member.
        path: String,
    },
}

impl RegistryError {
    /// The stable code, for example `unknown_field` or `duplicate_key`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Canon(error) => error.code(),
            Self::Rule { code, .. } => code.code(),
        }
    }

    /// The JSON Pointer of the fault, for a rule fault.
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Canon(_) => None,
            Self::Rule { path, .. } => Some(path),
        }
    }
}

impl From<CanonError> for RegistryError {
    fn from(error: CanonError) -> Self {
        Self::Canon(error)
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Canon(error) => error.fmt(f),
            Self::Rule { code, path } => write!(f, "{} at {:?}", code.code(), path),
        }
    }
}

impl std::error::Error for RegistryError {}
