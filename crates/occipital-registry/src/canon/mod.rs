//! Canonical JSON as RFC 8785 (JCS) defines it, restricted to the registry's
//! JSON profile: numbers must be integers within ±(2^53 − 1), written without
//! a fraction or exponent, and `-0` is refused.
//!
//! The crate has its own byte-level parser rather than `serde_json`, for two
//! reasons. `serde_json::Value` silently keeps the last of two duplicate keys,
//! and the workspace's language library enables serde_json's
//! `arbitrary_precision` feature, which Cargo unifies across every crate that
//! links serde_json and which changes how numbers decode.
//!
//! Each refusal is a [`CanonError`] with a stable code and the input byte
//! offset of the fault. When an input has several faults, an input over
//! [`MAX_INPUT_BYTES`] is refused first, and otherwise the fault at the
//! lowest offset is reported.

mod parse;
mod value;
mod write;

use std::fmt;

pub use value::{compare_keys, Integer, Object, Value};

/// The largest input accepted, in bytes. It equals the cap on a canonical
/// document: canonicalization never lengthens a conforming input, so a raw
/// input within this bound always yields canonical bytes within it.
pub const MAX_INPUT_BYTES: usize = 1_048_576;

/// The deepest nesting accepted. A top-level array or object is depth 1 and
/// each array or object inside it adds 1.
pub const MAX_DEPTH: usize = 64;

/// Why an input was refused, each with a stable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanonErrorKind {
    /// `input_too_large`: the input is longer than [`MAX_INPUT_BYTES`].
    /// Checked before parsing.
    InputTooLarge,
    /// `depth_exceeded`: an array or object opens deeper than
    /// [`MAX_DEPTH`]. Reported at its opening bracket.
    DepthExceeded,
    /// `invalid_utf8`: an ill-formed UTF-8 sequence. Reported at the
    /// sequence's first byte.
    InvalidUtf8,
    /// `lone_surrogate`: a `\u` escape naming a surrogate that is not part
    /// of a high-then-low pair. Reported at the escape's backslash.
    LoneSurrogate,
    /// `duplicate_key`: two keys of one object are equal once their escapes
    /// are decoded. Reported at the second key's opening quote.
    DuplicateKey,
    /// `number_not_integer`: a valid number with a fraction or exponent, or
    /// `-0`. Reported at the token's first byte.
    NumberNotInteger,
    /// `number_out_of_range`: an integer outside ±(2^53 − 1). Reported at
    /// the token's first byte.
    NumberOutOfRange,
    /// `json_syntax`: any other grammar error, including a number token that
    /// is not a valid RFC 8259 number. Reported at the first byte the
    /// grammar cannot accept, which is the input length when the input ends
    /// too early.
    JsonSyntax,
}

impl CanonErrorKind {
    /// The stable code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InputTooLarge => "input_too_large",
            Self::DepthExceeded => "depth_exceeded",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::LoneSurrogate => "lone_surrogate",
            Self::DuplicateKey => "duplicate_key",
            Self::NumberNotInteger => "number_not_integer",
            Self::NumberOutOfRange => "number_out_of_range",
            Self::JsonSyntax => "json_syntax",
        }
    }
}

/// A refused input: what is wrong and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanonError {
    kind: CanonErrorKind,
    offset: usize,
}

impl CanonError {
    pub(crate) const fn new(kind: CanonErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// What is wrong.
    pub const fn kind(&self) -> CanonErrorKind {
        self.kind
    }

    /// The stable code, for example `duplicate_key`.
    pub const fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// The input byte offset of the fault. For `input_too_large` it is
    /// [`MAX_INPUT_BYTES`], the first byte past the limit.
    pub const fn offset(&self) -> usize {
        self.offset
    }
}

impl fmt::Display for CanonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.code(), self.offset)
    }
}

impl std::error::Error for CanonError {}

/// Parses `input` as one JSON document in the registry's profile and returns
/// it decoded, object members in canonical order.
pub fn parse(input: &[u8]) -> Result<Value, CanonError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(CanonError::new(
            CanonErrorKind::InputTooLarge,
            MAX_INPUT_BYTES,
        ));
    }
    parse::parse_document(input)
}

/// Returns the RFC 8785 canonical bytes of `input`, or the refusal for its
/// first fault.
pub fn canonicalize(input: &[u8]) -> Result<Vec<u8>, CanonError> {
    parse(input).map(|value| value.to_canonical())
}

impl Value {
    /// The RFC 8785 canonical bytes of this value.
    pub fn to_canonical(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write::write_value(self, &mut out);
        out
    }
}
