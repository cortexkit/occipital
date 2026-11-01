//! The decoded JSON document the canonical writer serializes.
//!
//! Registry JSON carries only integers within ±(2^53 − 1), never fractions,
//! so numbers are kept as `i64` behind [`Integer`], which refuses anything
//! outside that range. Objects keep their members in canonical order (UTF-16
//! code units) with unique keys, so a value built in code and a value parsed
//! from bytes serialize the same way.

use std::cmp::Ordering;

/// A JSON integer within ±(2^53 − 1), the range every JSON implementation
/// represents exactly, so RFC 8785's number rule gives the same digits
/// everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Integer(i64);

impl Integer {
    /// The largest accepted value, 2^53 − 1.
    pub const MAX: i64 = 9_007_199_254_740_991;
    /// The smallest accepted value, −(2^53 − 1).
    pub const MIN: i64 = -9_007_199_254_740_991;

    /// Wraps `value`, or returns `None` when it lies outside ±(2^53 − 1).
    pub fn new(value: i64) -> Option<Self> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// The wrapped value.
    pub fn get(self) -> i64 {
        self.0
    }
}

// Every `i32` and `u32` lies well inside ±(2^53 − 1), so these conversions
// cannot fail and let callers build typed registry objects without unwrapping.
impl From<i32> for Integer {
    fn from(value: i32) -> Self {
        Self(i64::from(value))
    }
}

impl From<u32> for Integer {
    fn from(value: u32) -> Self {
        Self(i64::from(value))
    }
}

/// A JSON value as the registry accepts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// An integer within ±(2^53 − 1).
    Integer(Integer),
    /// A string, already decoded from its escapes.
    String(String),
    /// An array, in its original element order.
    Array(Vec<Value>),
    /// An object, its members in canonical order.
    Object(Object),
}

/// A JSON object whose members are always sorted by the UTF-16 code units of
/// their keys (RFC 8785 §3.2.3) and whose keys are unique.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Object {
    members: Vec<(String, Value)>,
}

impl Object {
    /// An empty object.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an object from members whose keys are already known to be
    /// unique, sorting them into canonical order.
    pub(crate) fn from_unique_members(mut members: Vec<(String, Value)>) -> Self {
        members.sort_by(|a, b| compare_keys(&a.0, &b.0));
        Self { members }
    }

    /// Inserts a member, keeping canonical order. When the key is already
    /// present its value is replaced and the old value returned.
    pub fn insert(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        let key = key.into();
        match self
            .members
            .binary_search_by(|(existing, _)| compare_keys(existing, &key))
        {
            Ok(index) => self
                .members
                .get_mut(index)
                .map(|member| std::mem::replace(&mut member.1, value)),
            Err(index) => {
                self.members.insert(index, (key, value));
                None
            }
        }
    }

    /// The value of the member named `key`, if present.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.members
            .binary_search_by(|(existing, _)| compare_keys(existing, key))
            .ok()
            .and_then(|index| self.members.get(index))
            .map(|(_, value)| value)
    }

    /// A mutable reference to the value of the member named `key`, if
    /// present.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.members
            .binary_search_by(|(existing, _)| compare_keys(existing, key))
            .ok()
            .and_then(|index| self.members.get_mut(index))
            .map(|(_, value)| value)
    }

    /// Removes the member named `key` and returns its value, if present.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.members
            .binary_search_by(|(existing, _)| compare_keys(existing, key))
            .ok()
            .map(|index| self.members.remove(index).1)
    }

    /// The members in canonical order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.members
            .iter()
            .map(|(key, value)| (key.as_str(), value))
    }

    /// The number of members.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether the object has no members.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

/// Orders two keys by their UTF-16 code units, the order RFC 8785 §3.2.3
/// requires. It differs from byte or code point order for characters beyond
/// the Basic Multilingual Plane: U+10000 is the surrogate pair D800 DC00, so
/// it sorts before U+E000.
pub fn compare_keys(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}
