//! Expressions: literals, names, operators (§5.1) and lookups (§5.2), in the
//! fixed left-to-right order of §5.3.

use cortexkit_rich_ui_syntax::{Expression as E, Kind, Node};

use super::{Abort, Evaluator};
use crate::number::Decimal;
use crate::{Relation, ScalarString, Value, ValueError, ValueFailure, ValueKind};

impl<'a> Evaluator<'a> {
    pub(super) fn expr(&mut self, node: &'a Node) -> Result<Value, Abort> {
        match &node.kind {
            Kind::Expression(expression) => self.expression(expression, node),
            Kind::Tag { name, props, .. } => self.tag(node, name, props),
            Kind::Reference { n } => {
                self.charge(1)?;
                Ok(self.reference(n))
            }
            Kind::Handle { kind, .. } => {
                self.charge(1)?;
                Ok(self.handle(node, kind))
            }
            // Body constructs never stand where a value is expected.
            _ => self.error("E_TYPE", node),
        }
    }

    fn expression(&mut self, expression: &'a E, node: &'a Node) -> Result<Value, Abort> {
        match expression {
            E::Number(token) => {
                self.charge(1)?;
                match Decimal::from_literal(token) {
                    Ok(number) => self.atom(Value::number(number)),
                    Err(error) => self.error(error.code(), node),
                }
            }
            E::String(text) => {
                // Visiting a string literal costs one unit; it is not charged the
                // per-scalar production cost of a newly produced string.
                self.charge(1)?;
                self.string(text.clone(), text.chars().count() as u64)
            }
            E::Boolean(value) => {
                self.charge(1)?;
                self.atom(Value::boolean(*value))
            }
            E::Null => {
                self.charge(1)?;
                self.atom(Value::null())
            }
            E::Name(name) => {
                self.charge(1)?;
                self.name(name, node)
            }
            E::State(name) => {
                self.charge(1)?;
                Ok(self.state(name))
            }
            E::Group => self.expr(&node.children[0]),
            E::List => {
                self.charge(1)?;
                let mut values = Vec::with_capacity(node.children.len());
                for child in &node.children {
                    values.push(self.expr(child)?);
                }
                // Building a list: one unit per element (§9.2).
                self.charge(values.len() as u64)?;
                self.list(values)
            }
            E::Record(keys) => {
                self.charge(1)?;
                let mut fields = Vec::with_capacity(keys.len());
                for (key, child) in keys.iter().zip(&node.children) {
                    fields.push((ScalarString::new(key.as_str()), self.expr(child)?));
                }
                self.charge(fields.len() as u64)?;
                self.record(fields)
            }
            E::Unary(op) => {
                self.charge(1)?;
                let operand = self.expr(&node.children[0])?;
                if op == "-" {
                    match operand.kind() {
                        ValueKind::Number(number) => self.atom(Value::number(-*number)),
                        ValueKind::Error(_) => Ok(operand),
                        _ => self.error("E_TYPE", node),
                    }
                } else {
                    let span = self.span(node);
                    match operand.condition(span) {
                        Ok(value) => self.atom(Value::boolean(!value)),
                        Err(error) => self.failed(error, &operand),
                    }
                }
            }
            E::Binary(op) => self.binary(op, node),
            E::Conditional => {
                self.charge(1)?;
                let condition = self.expr(&node.children[0])?;
                let span = self.span(&node.children[0]);
                match condition.condition(span) {
                    Ok(true) => self.expr(&node.children[1]),
                    Ok(false) => self.expr(&node.children[2]),
                    Err(error) => self.failed(error, &condition),
                }
            }
            E::Member(name) => {
                self.charge(1)?;
                let target = self.expr(&node.children[0])?;
                self.member(&target, name)
            }
            E::Index => {
                self.charge(1)?;
                let target = self.expr(&node.children[0])?;
                if matches!(target.kind(), ValueKind::Error(_)) {
                    return Ok(target);
                }
                let index = self.expr(&node.children[1])?;
                Ok(index_value(&target, &index))
            }
            E::Call(name) => self.call(name, node),
            _ => self.unsupported(node),
        }
    }

    pub(super) fn unsupported(&mut self, node: &'a Node) -> Result<Value, Abort> {
        self.charge(1)?;
        match self.program.parsed.unsupported(node) {
            Ok(text) => self.string(text.to_owned(), text.chars().count() as u64),
            Err(code) => self.error(code.as_str(), node),
        }
    }

    /// An operand's own error propagates unchanged; a new one is created.
    fn failed(&mut self, error: ValueError, operand: &Value) -> Result<Value, Abort> {
        if matches!(operand.kind(), ValueKind::Error(existing) if *existing == error) {
            Ok(operand.clone())
        } else {
            self.atom(Value::error(error))
        }
    }

    fn name(&mut self, name: &str, node: &'a Node) -> Result<Value, Abort> {
        if let Some((_, value)) = self
            .binders
            .iter()
            .rev()
            .find(|(binder, _)| *binder == name)
        {
            return Ok(value.clone());
        }
        let host = match name {
            "data" => Some(self.inputs.data.clone()),
            "input" => Some(self.inputs.input.clone()),
            "view" => Some(self.inputs.view.clone()),
            "now" => Some(Value::number(self.inputs.now)),
            "zone" => Some(self.inputs.zone.clone()),
            _ => None,
        };
        if let Some(value) = host {
            return Ok(value);
        }
        match self.names.get(name).copied() {
            Some(index) => self.named(index, node),
            None => self.error("E_UNKNOWN_NAME", node),
        }
    }

    /// The current state value, or the declared initial value (§4.10).
    fn state(&self, name: &str) -> Value {
        self.inputs.state.get(name).cloned().unwrap_or_else(|| {
            self.program
                .declarations
                .iter()
                .find(|declaration| declaration.state && declaration.name == name)
                .map(|declaration| super::literal(&declaration.expression))
                .unwrap_or_else(Value::null)
        })
    }

    fn binary(&mut self, op: &str, node: &'a Node) -> Result<Value, Abort> {
        self.charge(1)?;
        let span = self.span(node);
        match op {
            "&&" | "||" => {
                let left = self.expr(&node.children[0])?;
                let left_value = match left.condition(span) {
                    Ok(value) => value,
                    Err(error) => return self.failed(error, &left),
                };
                // Only the needed right side is evaluated (§5.1 levels 8, 9).
                if left_value == (op == "||") {
                    return self.atom(Value::boolean(left_value));
                }
                let right = self.expr(&node.children[1])?;
                match right.condition(span) {
                    Ok(value) => self.atom(Value::boolean(value)),
                    Err(error) => self.failed(error, &right),
                }
            }
            "??" => {
                let left = self.expr(&node.children[0])?;
                if left.is_absent() {
                    self.expr(&node.children[1])
                } else {
                    Ok(left)
                }
            }
            _ => {
                let left = self.expr(&node.children[0])?;
                // An error operand is returned without evaluating the other
                // side, so the first error in evaluation order wins (§4.6).
                if matches!(left.kind(), ValueKind::Error(_)) {
                    return Ok(left);
                }
                let right = self.expr(&node.children[1])?;
                if matches!(right.kind(), ValueKind::Error(_)) {
                    return Ok(right);
                }
                self.strict(op, &left, &right, node)
            }
        }
    }

    fn strict(
        &mut self,
        op: &str,
        left: &Value,
        right: &Value,
        node: &'a Node,
    ) -> Result<Value, Abort> {
        let span = self.span(node);
        let meter = &mut self.meter;
        let compared = match op {
            "==" | "!=" => Some(
                left.equals(right, span, &mut |fuel| meter.charge(fuel))
                    .map(|equal| equal == (op == "==")),
            ),
            "<" | "<=" | ">" | ">=" => {
                let relation = match op {
                    "<" => Relation::Less,
                    "<=" => Relation::LessEqual,
                    ">" => Relation::Greater,
                    _ => Relation::GreaterEqual,
                };
                Some(left.relates(right, relation, span, &mut |fuel| meter.charge(fuel)))
            }
            _ => None,
        };
        if let Some(compared) = compared {
            return match compared {
                Ok(value) => self.atom(Value::boolean(value)),
                Err(ValueFailure::Value(error)) => self.atom(Value::error(error)),
                Err(ValueFailure::Fuel(abort)) => Err(abort),
            };
        }
        if op == "+" && !(is_number(left) && is_number(right)) {
            return self.concatenate(left, right, node);
        }
        let (ValueKind::Number(a), ValueKind::Number(b)) = (left.kind(), right.kind()) else {
            // Arithmetic on absent values is a type error (§5.1).
            return self.error("E_TYPE", node);
        };
        let (a, b) = (*a, *b);
        let result = match op {
            "+" => a.checked_add(b),
            "-" => a.checked_sub(b),
            "*" => a.checked_mul(b),
            "/" => a.checked_div(b),
            "%" => a.checked_rem(b),
            "**" => {
                // One unit per multiplication of the §4.2 power algorithm.
                let meter = &mut self.meter;
                a.pow_with_fuel(b, || meter.charge(1))?
            }
            _ => return self.error("E_TYPE", node),
        };
        match result {
            Ok(number) => self.atom(Value::number(number)),
            Err(error) => self.error(error.code(), node),
        }
    }

    /// `+` with a string side: text conversion of both sides, then one
    /// produced string whose length is known before anything is copied.
    fn concatenate(&mut self, left: &Value, right: &Value, node: &'a Node) -> Result<Value, Abort> {
        if !(is_string(left) || is_string(right)) {
            return self.error("E_TYPE", node);
        }
        let (Some(a), Some(b)) = (text_length(left), text_length(right)) else {
            return self.error("E_TYPE", node);
        };
        let scalars = a.saturating_add(b);
        self.produce(scalars)?;
        let span = self.span(node);
        let mut text = left.to_text(span).unwrap_or_default();
        text.push_str(&right.to_text(span).unwrap_or_default());
        self.string(text, scalars)
    }

    /// `.name` (§5.2). On a list it plucks, at one unit per element (§9.2).
    pub(super) fn member(&mut self, target: &Value, name: &str) -> Result<Value, Abort> {
        match target.kind() {
            ValueKind::Record(record) => Ok(record.get(name)),
            ValueKind::List(items) => {
                self.charge(items.len() as u64)?;
                let mut values = Vec::with_capacity(items.len());
                for item in items.iter() {
                    values.push(self.member(item, name)?);
                }
                self.list(values)
            }
            ValueKind::Snapshot(snapshot) => Ok(snapshot.members.get(name)),
            ValueKind::SiteEnvelope(envelope) => Ok(envelope.members.get(name)),
            ValueKind::Error(_) => Ok(target.clone()),
            _ => Ok(Value::missing()),
        }
    }

    /// Whether a name is the binder of an enclosing `Each`.
    pub(super) fn is_binder(&self, name: &str) -> bool {
        self.binders.iter().any(|(binder, _)| *binder == name)
    }

    /// Whether a named value has already been evaluated in this evaluation.
    #[cfg(test)]
    pub(super) fn evaluated(&self, name: &str) -> bool {
        self.names
            .get(name)
            .is_some_and(|index| matches!(self.memo[*index], super::Memo::Done(..)))
    }
}

/// `[index]` (§5.2): never fails on shape, and an error index propagates.
fn index_value(target: &Value, index: &Value) -> Value {
    if matches!(index.kind(), ValueKind::Error(_)) {
        return index.clone();
    }
    match (target.kind(), index.kind()) {
        (ValueKind::List(items), ValueKind::Number(position)) => position
            .integer()
            .and_then(|position| usize::try_from(position).ok())
            .and_then(|position| items.get(position).cloned())
            .unwrap_or_else(Value::missing),
        (ValueKind::Record(record), ValueKind::String(key)) => record.get(key.as_str()),
        (ValueKind::Snapshot(snapshot), ValueKind::String(key)) => {
            snapshot.members.get(key.as_str())
        }
        (ValueKind::SiteEnvelope(envelope), ValueKind::String(key)) => {
            envelope.members.get(key.as_str())
        }
        _ => Value::missing(),
    }
}

fn is_number(value: &Value) -> bool {
    matches!(value.kind(), ValueKind::Number(_))
}

fn is_string(value: &Value) -> bool {
    matches!(value.kind(), ValueKind::String(_))
}

/// The scalar length of a value's text conversion (§4.11), computed without
/// building it; `None` for kinds that have no text conversion.
fn text_length(value: &Value) -> Option<u64> {
    match value.kind() {
        ValueKind::String(text) => Some(text.len()),
        ValueKind::Number(number) => Some(number.text_len()),
        ValueKind::Boolean(true) => Some(4),
        ValueKind::Boolean(false) => Some(5),
        ValueKind::Null | ValueKind::Missing => Some(0),
        _ => None,
    }
}
