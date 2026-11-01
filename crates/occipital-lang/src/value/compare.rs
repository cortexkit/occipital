use std::cmp::Ordering;

use super::{HandleIdentity, Record, SourceSpan, Value, ValueError, ValueKind, ValueType};

/// A local error propagates as a value; a rejected fuel charge aborts evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueFailure<E> {
    /// A local value error, such as comparing incompatible kinds.
    Value(ValueError),
    /// The fuel charge was refused.
    Fuel(E),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// An ordering comparison operator (§4.12).
pub enum Relation {
    /// `<`.
    Less,
    /// `<=`.
    LessEqual,
    /// `>`.
    Greater,
    /// `>=`.
    GreaterEqual,
}

/// Charge each block before examining it. The terminal position is the common
/// prefix plus one in §9.2, including when the two strings are identical.
fn string_order<E>(
    left: &str,
    right: &str,
    charge: &mut impl FnMut(u64) -> Result<(), E>,
) -> Result<Ordering, E> {
    let mut left = left.chars();
    let mut right = right.chars();
    let mut position = 0u64;
    loop {
        if position.is_multiple_of(16) {
            charge(1)?;
        }
        match (left.next(), right.next()) {
            (Some(a), Some(b)) if a == b => position += 1,
            (a, b) => return Ok(a.cmp(&b)),
        }
    }
}

fn sort_fuel(record: &Record) -> u64 {
    let n = record.fields.len() as u64;
    let ceiling_log = u64::BITS - (n.max(2) - 1).leading_zeros();
    n.saturating_add(
        2u64.saturating_mul(n)
            .saturating_mul(ceiling_log.into())
            .saturating_mul(1u64.saturating_add(record.longest_key.div_ceil(64))),
    )
}

impl Value {
    /// Language equality, not Rust structural equality (§4.12). Expression-node
    /// fuel is the caller's charge; this method charges primitive comparisons
    /// and all values visited by a deep comparison, including its root.
    pub fn equals<E>(
        &self,
        other: &Self,
        span: SourceSpan,
        charge: &mut impl FnMut(u64) -> Result<(), E>,
    ) -> Result<bool, ValueFailure<E>> {
        let deep = matches!(self.kind, ValueKind::List(_) | ValueKind::Record(_))
            || matches!(other.kind, ValueKind::List(_) | ValueKind::Record(_));
        self.equal_inner(other, span, charge, deep)
    }

    fn equal_inner<E>(
        &self,
        other: &Self,
        span: SourceSpan,
        charge: &mut impl FnMut(u64) -> Result<(), E>,
        visit: bool,
    ) -> Result<bool, ValueFailure<E>> {
        if visit {
            charge(1).map_err(ValueFailure::Fuel)?;
        }
        if let ValueKind::Error(error) = &self.kind {
            return Err(ValueFailure::Value(error.clone()));
        }
        if let ValueKind::Error(error) = &other.kind {
            return Err(ValueFailure::Value(error.clone()));
        }
        let opaque = |value: &Self| {
            matches!(
                value.kind,
                ValueKind::Component(_)
                    | ValueKind::Action(_)
                    | ValueKind::Snapshot(_)
                    | ValueKind::SiteEnvelope(_)
            )
        };
        if opaque(self) || opaque(other) {
            return Err(ValueFailure::Value(ValueError::type_error(span)));
        }
        if self.is_absent() && other.is_absent() {
            return Ok(true);
        }
        match (&self.kind, &other.kind) {
            (ValueKind::Number(a), ValueKind::Number(b)) => Ok(a == b),
            (ValueKind::Boolean(a), ValueKind::Boolean(b)) => Ok(a == b),
            (ValueKind::String(a), ValueKind::String(b)) => {
                Ok(
                    string_order(a.as_str(), b.as_str(), charge).map_err(ValueFailure::Fuel)?
                        == Ordering::Equal,
                )
            }
            (ValueKind::Handle(a), ValueKind::Handle(b)) => {
                let equal_string = |a: &str, b: &str, charge: &mut _| {
                    string_order(a, b, charge)
                        .map(|order| order.is_eq())
                        .map_err(ValueFailure::Fuel)
                };
                match (&a.identity, &b.identity) {
                    (
                        HandleIdentity::Resolved {
                            kind: ak,
                            scoped_resource_id: ar,
                            version: av,
                        },
                        HandleIdentity::Resolved {
                            kind: bk,
                            scoped_resource_id: br,
                            version: bv,
                        },
                    ) => Ok(equal_string(ak, bk, charge)?
                        && equal_string(ar, br, charge)?
                        && equal_string(av, bv, charge)?),
                    (
                        HandleIdentity::Unresolved {
                            scoped_record_id: a,
                        },
                        HandleIdentity::Unresolved {
                            scoped_record_id: b,
                        },
                    ) => equal_string(a, b, charge),
                    _ => Ok(false),
                }
            }
            (ValueKind::List(a), ValueKind::List(b)) => {
                if a.len() != b.len() {
                    return Ok(false);
                }
                for (a, b) in a.iter().zip(b.iter()) {
                    if !a.equal_inner(b, span, charge, true)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (ValueKind::Record(a), ValueKind::Record(b)) => {
                if a.fields.len() != b.fields.len() {
                    return Ok(false);
                }
                // Record-key sorting uses the §9.2 Sort tariff once per record:
                // n + 2n ceil(log2(max(n,2))) (1 + ceil(K/64)), K=max key scalars.
                // Charge both sorts together before either runs. Different key
                // counts need only the root visit, since no sort is necessary.
                charge(sort_fuel(a).saturating_add(sort_fuel(b))).map_err(ValueFailure::Fuel)?;
                let mut a: Vec<_> = a.fields.iter().collect();
                let mut b: Vec<_> = b.fields.iter().collect();
                a.sort_unstable_by_key(|(key, _)| key.as_str());
                b.sort_unstable_by_key(|(key, _)| key.as_str());
                // Establish the whole key set before visiting any values. An
                // unequal key therefore wins over errors in unvisited fields.
                for ((a, _), (b, _)) in a.iter().zip(&b) {
                    if string_order(a.as_str(), b.as_str(), charge).map_err(ValueFailure::Fuel)?
                        != Ordering::Equal
                    {
                        return Ok(false);
                    }
                }
                for ((_, a), (_, b)) in a.into_iter().zip(b) {
                    if !a.equal_inner(b, span, charge, true)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// The four relational operators (§4.12). Errors propagate first; otherwise
    /// any absent operand makes the result false, without a less/greater order.
    pub fn relates<E>(
        &self,
        other: &Self,
        relation: Relation,
        span: SourceSpan,
        charge: &mut impl FnMut(u64) -> Result<(), E>,
    ) -> Result<bool, ValueFailure<E>> {
        if let ValueKind::Error(error) = &self.kind {
            return Err(ValueFailure::Value(error.clone()));
        }
        if let ValueKind::Error(error) = &other.kind {
            return Err(ValueFailure::Value(error.clone()));
        }
        if self.is_absent() || other.is_absent() {
            return Ok(false);
        }
        let ordering = match (&self.kind, &other.kind) {
            (ValueKind::Number(a), ValueKind::Number(b)) => a.cmp(b),
            (ValueKind::String(a), ValueKind::String(b)) => {
                string_order(a.as_str(), b.as_str(), charge).map_err(ValueFailure::Fuel)?
            }
            _ => return Err(ValueFailure::Value(ValueError::type_error(span))),
        };
        Ok(match relation {
            Relation::Less => ordering.is_lt(),
            Relation::LessEqual => ordering.is_le(),
            Relation::Greater => ordering.is_gt(),
            Relation::GreaterEqual => ordering.is_ge(),
        })
    }

    /// Sort key preorder (§4.12); the caller must charge the §9.2 Sort tariff
    /// before sorting. A stable sort retains source order on Equal. Absent
    /// values and errors stay last even for descending order.
    pub fn sort_order(&self, other: &Self, descending: bool) -> Ordering {
        fn group(value: &Value) -> u8 {
            match value.value_type() {
                ValueType::Number => 0,
                ValueType::String => 1,
                ValueType::Boolean => 2,
                ValueType::Null | ValueType::Missing => 4,
                ValueType::Error => 5,
                _ => 3,
            }
        }
        let a = group(self);
        let b = group(other);
        let order = a.cmp(&b).then_with(|| match (&self.kind, &other.kind) {
            (ValueKind::Number(a), ValueKind::Number(b)) => a.cmp(b),
            (ValueKind::String(a), ValueKind::String(b)) => a.as_str().cmp(b.as_str()),
            (ValueKind::Boolean(a), ValueKind::Boolean(b)) => a.cmp(b),
            _ => Ordering::Equal,
        });
        if descending && a < 4 && b < 4 {
            order.reverse()
        } else {
            order
        }
    }
}
