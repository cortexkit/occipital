//! Walk positions and fault selection shared by the shape decoder and the
//! value rules.
//!
//! A document with several faults reports exactly one. Faults are grouped in
//! tiers; a lower tier always wins, and within a tier the first fault met in
//! the walk wins. Both walks visit positions in the registry's walk order and,
//! at one position, try codes in their listed order, so "keep the first fault
//! of the lowest tier" is all the selection needs.

use crate::error::{RegistryError, RuleCode};

/// The tiers below canonical JSON, in the order they are judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tier {
    /// The root and the top-level `schema`.
    Root,
    /// Member names and JSON types.
    Shape,
    /// Values, ids, digests and sizes.
    Value,
    /// Duplicates and references between parts of the document.
    Cross,
}

/// One step of a JSON Pointer.
#[derive(Clone, Copy)]
enum Step<'a> {
    Root,
    Key(&'a str),
    Index(usize),
}

/// A position in the document, kept as a chain of borrowed steps on the
/// stack so the walk allocates only when it reports a fault.
#[derive(Clone, Copy)]
pub(crate) struct Path<'a> {
    parent: Option<&'a Path<'a>>,
    step: Step<'a>,
}

impl<'a> Path<'a> {
    /// The document root.
    pub(crate) const ROOT: Path<'static> = Path {
        parent: None,
        step: Step::Root,
    };

    /// The member `key` of the object here.
    pub(crate) fn key<'b>(&'b self, key: &'b str) -> Path<'b> {
        Path {
            parent: Some(self),
            step: Step::Key(key),
        }
    }

    /// The element `index` of the array here.
    pub(crate) fn index(&self, index: usize) -> Path<'_> {
        Path {
            parent: Some(self),
            step: Step::Index(index),
        }
    }

    /// The RFC 6901 JSON Pointer of this position.
    pub(crate) fn render(&self) -> String {
        let mut steps = Vec::new();
        let mut current = Some(self);
        while let Some(path) = current {
            steps.push(path.step);
            current = path.parent;
        }
        let mut out = String::new();
        for step in steps.iter().rev() {
            match step {
                Step::Root => {}
                Step::Key(key) => {
                    out.push('/');
                    for character in key.chars() {
                        match character {
                            '~' => out.push_str("~0"),
                            '/' => out.push_str("~1"),
                            other => out.push(other),
                        }
                    }
                }
                Step::Index(index) => {
                    out.push('/');
                    out.push_str(&index.to_string());
                }
            }
        }
        out
    }
}

/// Collects the one fault a document reports.
#[derive(Default)]
pub(crate) struct Faults {
    first: Option<(Tier, RuleCode, String)>,
}

impl Faults {
    /// Records a fault at `path`. It replaces the kept fault only when its
    /// tier is lower, because the walk meets faults of one tier in order.
    pub(crate) fn report(&mut self, tier: Tier, code: RuleCode, path: &Path<'_>) {
        let replaces = match &self.first {
            None => true,
            Some((kept, _, _)) => tier < *kept,
        };
        if replaces {
            self.first = Some((tier, code, path.render()));
        }
    }

    /// Whether any fault was recorded.
    pub(crate) fn any(&self) -> bool {
        self.first.is_some()
    }

    /// The fault to report, if any.
    pub(crate) fn into_error(self) -> Option<RegistryError> {
        self.first
            .map(|(_, code, path)| RegistryError::Rule { code, path })
    }
}
