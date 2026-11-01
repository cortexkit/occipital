//! Metered §6.1–§6.4 primitives. Dispatch owns the single call-base charge;
//! callers must not add expression-visit or duplicate scan/build charges.

mod format;
mod lists;
mod strings;

use crate::number::Accumulator;
use crate::{Decimal, SourceSpan, Value, ValueError, ValueFailure, ValueKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Builtin {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    Filter,
    Sort,
    First,
    Last,
    Slice,
    Range,
    Contains,
    Keys,
    Values,
    Join,
    IsError,
    Round,
    Floor,
    Ceil,
    Abs,
    Sqrt,
    Clamp,
    Number,
    Div,
    Text,
    Len,
    Upper,
    Lower,
    Trim,
    StartsWith,
    EndsWith,
    Replace,
    Split,
    Lines,
    Truncate,
    FormatNumber,
    FormatBytes,
    FormatDuration,
    FormatDate,
    FormatRelative,
    ParseDate,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure<E> {
    Value(ValueError),
    Fuel(E),
    Limit(&'static str),
}

impl<E> From<ValueFailure<E>> for Failure<E> {
    fn from(error: ValueFailure<E>) -> Self {
        match error {
            ValueFailure::Value(error) => Self::Value(error),
            ValueFailure::Fuel(error) => Self::Fuel(error),
        }
    }
}

type Result<T, E> = std::result::Result<T, Failure<E>>;

/// How a call to a built-in is dispatched. `IsError` alone sees error
/// arguments; every other built-in belongs to exactly one family, so each
/// family matches only its own operations, exhaustively.
enum Dispatch {
    IsError,
    Strict(Family),
}

enum Family {
    List(lists::Op),
    String(strings::Op),
    Format(format::Op),
    ParseDate,
    ParseNumber,
    Number(NumberOp),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NumberOp {
    Round,
    Floor,
    Ceil,
    Abs,
    Sqrt,
    Div,
    Clamp,
}

pub(super) struct Context<'a, E, F: FnMut(u64) -> std::result::Result<(), E>> {
    span: SourceSpan,
    charge: &'a mut F,
}

impl<E, F: FnMut(u64) -> std::result::Result<(), E>> Context<'_, E, F> {
    fn pay(&mut self, fuel: u64) -> Result<(), E> {
        (self.charge)(fuel).map_err(Failure::Fuel)
    }

    fn error(&self, code: &'static str) -> Failure<E> {
        Failure::Value(ValueError::new(code, self.span))
    }

    fn number(&self, value: &Value) -> Result<Decimal, E> {
        match value.kind() {
            ValueKind::Number(n) => Ok(*n),
            _ => Err(self.error("E_TYPE")),
        }
    }

    fn numeric(
        &self,
        result: std::result::Result<Decimal, crate::NumberError>,
    ) -> Result<Value, E> {
        result.map(Value::number).map_err(|e| self.error(e.code()))
    }

    fn list<'a>(&self, value: &'a Value) -> Result<&'a [Value], E> {
        match value.kind() {
            ValueKind::List(list) => Ok(list),
            ValueKind::Null | ValueKind::Missing => Ok(&[]),
            _ => Err(self.error("E_TYPE")),
        }
    }

    fn string<'a>(&self, value: &'a Value) -> Result<&'a crate::ScalarString, E> {
        match value.kind() {
            ValueKind::String(text) => Ok(text),
            _ => Err(self.error("E_TYPE")),
        }
    }

    fn string_limit(&self, length: u64) -> Result<(), E> {
        if length > 65_536 {
            Err(Failure::Limit("E_LIMIT_STRING"))
        } else {
            Ok(())
        }
    }

    fn list_limit(&self, length: u64) -> Result<(), E> {
        if length > 10_000 {
            Err(Failure::Limit("E_LIMIT_LIST"))
        } else {
            Ok(())
        }
    }

    fn build_string(&self, text: String) -> Result<Value, E> {
        Value::string(text).map_err(|_| Failure::Limit("E_LIMIT_VALUE"))
    }

    fn build_list(&self, list: Vec<Value>) -> Result<Value, E> {
        let mut logical = 16;
        for value in &list {
            logical = self.list_item_size(logical, value)?;
        }
        Value::list(list).map_err(|_| Failure::Limit("E_LIMIT_VALUE"))
    }

    fn list_item_size(&self, logical: u64, value: &Value) -> Result<u64, E> {
        let size = logical.saturating_add(value.logical_size());
        if size > 4 * 1024 * 1024 {
            Err(Failure::Limit("E_LIMIT_VALUE"))
        } else {
            Ok(size)
        }
    }
}

impl Builtin {
    fn dispatch(self) -> Dispatch {
        use Builtin::*;
        let family = match self {
            IsError => return Dispatch::IsError,
            Count => Family::List(lists::Op::Count),
            Sum => Family::List(lists::Op::Sum),
            Avg => Family::List(lists::Op::Avg),
            Min => Family::List(lists::Op::Min),
            Max => Family::List(lists::Op::Max),
            Filter => Family::List(lists::Op::Filter),
            Sort => Family::List(lists::Op::Sort),
            First => Family::List(lists::Op::First),
            Last => Family::List(lists::Op::Last),
            Slice => Family::List(lists::Op::Slice),
            Range => Family::List(lists::Op::Range),
            Contains => Family::List(lists::Op::Contains),
            Keys => Family::List(lists::Op::Keys),
            Values => Family::List(lists::Op::Values),
            Join => Family::List(lists::Op::Join),
            Text => Family::String(strings::Op::Text),
            Len => Family::String(strings::Op::Len),
            Upper => Family::String(strings::Op::Upper),
            Lower => Family::String(strings::Op::Lower),
            Trim => Family::String(strings::Op::Trim),
            StartsWith => Family::String(strings::Op::StartsWith),
            EndsWith => Family::String(strings::Op::EndsWith),
            Replace => Family::String(strings::Op::Replace),
            Split => Family::String(strings::Op::Split),
            Lines => Family::String(strings::Op::Lines),
            Truncate => Family::String(strings::Op::Truncate),
            FormatNumber => Family::Format(format::Op::Number),
            FormatBytes => Family::Format(format::Op::Bytes),
            FormatDuration => Family::Format(format::Op::Duration),
            FormatDate => Family::Format(format::Op::Date),
            FormatRelative => Family::Format(format::Op::Relative),
            ParseDate => Family::ParseDate,
            Number => Family::ParseNumber,
            Round => Family::Number(NumberOp::Round),
            Floor => Family::Number(NumberOp::Floor),
            Ceil => Family::Number(NumberOp::Ceil),
            Abs => Family::Number(NumberOp::Abs),
            Sqrt => Family::Number(NumberOp::Sqrt),
            Div => Family::Number(NumberOp::Div),
            Clamp => Family::Number(NumberOp::Clamp),
        };
        Dispatch::Strict(family)
    }

    fn accepts(self, n: usize) -> bool {
        use Builtin::*;
        match self {
            Min | Max => n >= 1,
            Filter => n == 4,
            Sort => (1..=3).contains(&n),
            Range | Slice => (2..=3).contains(&n),
            Round | FormatBytes | FormatDuration => (1..=2).contains(&n),
            FormatNumber | FormatDate => (1..=3).contains(&n),
            Clamp | Replace => n == 3,
            Contains | Join | Div | StartsWith | EndsWith | Split | Truncate | FormatRelative => {
                n == 2
            }
            _ => n == 1,
        }
    }

    /// Arguments are already evaluated left to right. Each uses a separate lazy
    /// template entry point so templates are never eagerly evaluated as arguments.
    pub(crate) fn call<E>(
        self,
        args: &[Value],
        span: SourceSpan,
        charge: &mut impl FnMut(u64) -> std::result::Result<(), E>,
    ) -> Result<Value, E> {
        let mut cx = Context { span, charge };
        cx.pay(1)?;
        if !self.accepts(args.len()) {
            return Err(cx.error("E_ARITY"));
        }
        let family = match self.dispatch() {
            Dispatch::IsError => {
                cx.pay(1)?;
                return Ok(Value::boolean(matches!(
                    args[0].kind(),
                    ValueKind::Error(_)
                )));
            }
            Dispatch::Strict(family) => family,
        };
        for arg in args {
            if let ValueKind::Error(error) = arg.kind() {
                return Err(Failure::Value(error.clone()));
            }
        }
        match family {
            Family::List(op) => lists::call(op, args, &mut cx),
            Family::String(op) => strings::call(op, args, &mut cx),
            Family::Format(op) => format::call(op, args, &mut cx),
            Family::ParseDate => format::parse_date(args, &mut cx),
            Family::ParseNumber => parse_number(args, &mut cx),
            Family::Number(op) => numbers(op, args, &mut cx),
        }
    }
}

/// `Number(x)`: a number, absent, or text to parse.
fn parse_number<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    match args[0].kind() {
        ValueKind::Null | ValueKind::Missing => Ok(Value::null()),
        ValueKind::Number(_) => Ok(args[0].clone()),
        ValueKind::String(text) => {
            cx.pay(text.len().div_ceil(8))?;
            cx.numeric(Decimal::parse_text(crate::unicode::trim(text.as_str())))
        }
        _ => Err(cx.error("E_TYPE")),
    }
}

fn numbers<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    builtin: NumberOp,
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    use NumberOp::*;
    cx.pay(if builtin == Sqrt { 20 } else { 1 })?;
    let x = cx.number(&args[0])?;
    match builtin {
        Round => {
            let digits = args
                .get(1)
                .map(|v| cx.number(v))
                .transpose()?
                .unwrap_or(Decimal::ZERO);
            cx.numeric(x.round(digits))
        }
        Floor => cx.numeric(x.floor()),
        Ceil => cx.numeric(x.ceil()),
        Abs => Ok(Value::number(x.abs())),
        Sqrt => cx.numeric(x.sqrt()),
        Div => cx.numeric(x.div_integer(cx.number(&args[1])?)),
        Clamp => {
            let low = cx.number(&args[1])?;
            let high = cx.number(&args[2])?;
            if low > high {
                return Err(cx.error("E_ARG"));
            }
            Ok(Value::number(x.clamp(low, high)))
        }
    }
}

/// The evaluator supplies one template evaluation per element. Binder validation
/// and the four-level nesting check belong to the checker (§5.5).
pub(crate) fn each<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    input: &Value,
    span: SourceSpan,
    charge: &mut F,
    mut template: impl FnMut(&Value, &mut F) -> Result<Value, E>,
) -> Result<Value, E> {
    let mut cx = Context { span, charge };
    cx.pay(1)?;
    if let ValueKind::Error(error) = input.kind() {
        return Err(Failure::Value(error.clone()));
    }
    let list = cx.list(input)?;
    cx.pay(list.len() as u64)?;
    cx.list_limit(list.len() as u64)?;
    let mut output = Vec::with_capacity(list.len());
    let mut logical = 16;
    for value in list {
        let result = template(value, cx.charge)?;
        logical = cx.list_item_size(logical, &result)?;
        output.push(result);
    }
    cx.build_list(output)
}

#[cfg(test)]
mod tests;
