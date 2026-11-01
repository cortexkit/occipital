use super::{strings, Accumulator, Context, Failure, Result};
use crate::{Decimal, Relation, Value, ValueKind};

/// The list built-ins.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Op {
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
}

pub(super) fn call<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    builtin: Op,
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    use Op::*;
    match builtin {
        Count => {
            cx.pay(1)?;
            let list = cx.list(&args[0])?;
            Ok(Value::number(Decimal::from_u64(list.len() as u64)))
        }
        First => {
            cx.pay(1)?;
            let list = cx.list(&args[0])?;
            Ok(list.first().cloned().unwrap_or_else(Value::missing))
        }
        Last => {
            cx.pay(1)?;
            let list = cx.list(&args[0])?;
            Ok(list.last().cloned().unwrap_or_else(Value::missing))
        }
        Sum | Avg => {
            let list = cx.list(&args[0])?;
            cx.pay(list.len() as u64)?;
            let mut sum = Accumulator::new();
            for value in list {
                match value.kind() {
                    ValueKind::Number(n) => sum.push(*n),
                    ValueKind::Null | ValueKind::Missing => (),
                    ValueKind::Error(e) => return Err(Failure::Value(e.clone())),
                    _ => return Err(cx.error("E_TYPE")),
                }
            }
            sum.finish(builtin == Avg)
                .map(|n| n.map(Value::number).unwrap_or_else(Value::null))
                .map_err(|e| cx.error(e.code()))
        }
        Min | Max => extrema(args, builtin == Max, cx),
        Range => range(args, cx),
        Slice => slice(args, cx),
        Contains => contains(&args[0], &args[1], cx).map(Value::boolean),
        Keys | Values => {
            let fields = match args[0].kind() {
                ValueKind::Record(r) => r.fields(),
                ValueKind::Null | ValueKind::Missing => &[],
                _ => return Err(cx.error("E_TYPE")),
            };
            cx.pay(fields.len() as u64)?;
            cx.list_limit(fields.len() as u64)?;
            if builtin == Keys {
                for (key, _) in fields {
                    cx.string_limit(key.len())?;
                }
            }
            let values = fields
                .iter()
                .map(|(key, value)| {
                    if builtin == Keys {
                        // Each key passed the string limit above.
                        Value::text(key.as_str())
                    } else {
                        value.clone()
                    }
                })
                .collect();
            cx.build_list(values)
        }
        Join => join(args, cx),
        Sort => sort(args, cx),
        Filter => filter(args, cx),
    }
}

fn extrema<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    maximum: bool,
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    let list = if args.len() == 1 {
        cx.list(&args[0])?
    } else {
        args
    };
    cx.pay(list.len() as u64)?;
    let mut best: Option<&Value> = None;
    for value in list {
        if value.is_absent() {
            continue;
        }
        if let ValueKind::Error(e) = value.kind() {
            return Err(Failure::Value(e.clone()));
        }
        if !matches!(value.kind(), ValueKind::String(_) | ValueKind::Number(_)) {
            return Err(cx.error("E_TYPE"));
        }
        if let Some(prior) = best {
            let relation = if maximum {
                Relation::Greater
            } else {
                Relation::Less
            };
            if value.relates(prior, relation, cx.span, cx.charge)? {
                best = Some(value);
            }
        } else {
            best = Some(value);
        }
    }
    Ok(best.cloned().unwrap_or_else(Value::null))
}

fn range<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    let start = cx.number(&args[0])?;
    let end = cx.number(&args[1])?;
    let step = args
        .get(2)
        .map(|v| cx.number(v))
        .transpose()?
        .unwrap_or(Decimal::ONE);
    if step == Decimal::ZERO {
        return Err(cx.error("E_ARG"));
    }
    let count = if (step > Decimal::ZERO && start < end) || (step < Decimal::ZERO && start > end) {
        start
            .coefficient()
            .abs_diff(end.coefficient())
            .div_ceil(step.coefficient().unsigned_abs())
    } else {
        0
    };
    if count > 10_000 {
        return Err(Failure::Limit("E_LIMIT_LIST"));
    }
    cx.pay(count as u64)?;
    let mut result = Vec::with_capacity(count as usize);
    let mut value = start;
    for index in 0..count {
        result.push(Value::number(value));
        if index + 1 < count {
            value = value.checked_add(step).map_err(|e| cx.error(e.code()))?;
        }
    }
    cx.build_list(result)
}

fn position<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    value: &Value,
    len: u64,
    cx: &Context<'_, E, F>,
) -> Result<u64, E> {
    let pos = cx
        .number(value)?
        .integer()
        .ok_or_else(|| cx.error("E_ARG"))?;
    let pos = if pos < 0 { i128::from(len) + pos } else { pos };
    Ok(pos.clamp(0, len.into()) as u64)
}

fn slice<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    if args[0].is_absent() {
        return Ok(Value::missing());
    }
    let (len, string) = match args[0].kind() {
        ValueKind::List(list) => (list.len() as u64, false),
        ValueKind::String(text) => (text.len(), true),
        _ => return Err(cx.error("E_TYPE")),
    };
    let start = position(&args[1], len, cx)?;
    let end = args
        .get(2)
        .map(|v| position(v, len, cx))
        .transpose()?
        .unwrap_or(len)
        .max(start);
    let length = end - start;
    if string {
        // UTF-8 scalar indexing must locate the start before copying the result.
        // Charge that prefix by index even if a future representation caches it.
        cx.pay(start.div_ceil(8))?;
        cx.pay(length.div_ceil(8))?;
        cx.string_limit(length)?;
        let text = cx.string(&args[0])?.as_str();
        cx.build_string(
            text.chars()
                .skip(start as usize)
                .take(length as usize)
                .collect(),
        )
    } else {
        cx.pay(length)?;
        cx.list_limit(length)?;
        cx.build_list(cx.list(&args[0])?[start as usize..end as usize].to_vec())
    }
}

pub(super) fn contains<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    x: &Value,
    y: &Value,
    cx: &mut Context<'_, E, F>,
) -> Result<bool, E> {
    match x.kind() {
        ValueKind::Null | ValueKind::Missing => Ok(false),
        ValueKind::String(text) => {
            let pattern = cx.string(y)?;
            cx.pay(text.len().saturating_add(pattern.len()).div_ceil(16))?;
            Ok(strings::Search::new(pattern.as_str())
                .matches(text.as_str())
                .next()
                .is_some())
        }
        ValueKind::List(list) => {
            cx.pay(list.len() as u64)?;
            for item in list.iter() {
                if item.equals(y, cx.span, cx.charge)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ValueKind::Error(e) => Err(Failure::Value(e.clone())),
        _ => Err(cx.error("E_TYPE")),
    }
}

fn join<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    let list = cx.list(&args[0])?;
    let separator = cx.string(&args[1])?;
    cx.pay(list.len() as u64)?;
    let mut length = separator
        .len()
        .saturating_mul(list.len().saturating_sub(1) as u64);
    for value in list {
        length = length.saturating_add(strings::text_len(value, cx)?);
    }
    cx.string_limit(length)?;
    cx.pay(length.div_ceil(8))?;
    let mut text = String::new();
    for (index, value) in list.iter().enumerate() {
        if index != 0 {
            text.push_str(separator.as_str());
        }
        strings::append_text(&mut text, value);
    }
    cx.build_string(text)
}

fn field<'a, E, F: FnMut(u64) -> std::result::Result<(), E>>(
    value: Option<&'a Value>,
    cx: &Context<'_, E, F>,
    filter: bool,
) -> Result<Option<&'a str>, E> {
    match value.map(Value::kind) {
        None | Some(ValueKind::Null) => Ok(None),
        Some(ValueKind::String(text)) => Ok(Some(text.as_str())),
        _ => Err(cx.error(if filter { "E_ARG" } else { "E_TYPE" })),
    }
}

fn key(value: &Value, field: Option<&str>) -> Value {
    if matches!(value.kind(), ValueKind::Error(_)) {
        return value.clone();
    }
    field
        .map(|name| value.record_field(name))
        .unwrap_or_else(|| value.clone())
}

fn sort<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    let list = cx.list(&args[0])?;
    let field = field(args.get(1), cx, false)?;
    let descending = match args.get(2) {
        None => false,
        Some(value) => match cx.string(value)?.as_str() {
            "asc" => false,
            "desc" => true,
            _ => return Err(cx.error("E_ARG")),
        },
    };
    let n = list.len() as u64;
    // The n term pays for discovering keys and their recorded scalar lengths.
    // The comparison/build term is reserved before allocating or sorting keys.
    cx.pay(n)?;
    let longest = list
        .iter()
        .map(|value| match key(value, field).kind() {
            ValueKind::String(text) => text.len(),
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    let log = u64::BITS - (n.max(2) - 1).leading_zeros();
    cx.pay(
        2u64.saturating_mul(n)
            .saturating_mul(log.into())
            .saturating_mul(1 + longest.div_ceil(64)),
    )?;
    cx.list_limit(n)?;
    let mut keyed: Vec<_> = list.iter().map(|v| (key(v, field), v.clone())).collect();
    keyed.sort_by(|(a, _), (b, _)| a.sort_order(b, descending));
    cx.build_list(keyed.into_iter().map(|(_, value)| value).collect())
}

fn filter<E, F: FnMut(u64) -> std::result::Result<(), E>>(
    args: &[Value],
    cx: &mut Context<'_, E, F>,
) -> Result<Value, E> {
    let list = cx.list(&args[0])?;
    let field = field(Some(&args[1]), cx, true)?;
    let op = cx.string(&args[2])?.as_str();
    // The test each element faces, decided once from the operator text.
    enum Test {
        Equal(bool),
        Contains,
        Order(Relation),
    }
    let test = match op {
        "==" => Test::Equal(true),
        "!=" => Test::Equal(false),
        "contains" => Test::Contains,
        "<" => Test::Order(Relation::Less),
        "<=" => Test::Order(Relation::LessEqual),
        ">" => Test::Order(Relation::Greater),
        ">=" => Test::Order(Relation::GreaterEqual),
        _ => return Err(cx.error("E_ARG")),
    };
    cx.pay(list.len() as u64)?;
    let mut output = Vec::new();
    for value in list {
        let key = key(value, field);
        let comparison = match test {
            Test::Equal(wanted) => key
                .equals(&args[3], cx.span, cx.charge)
                .map(|equal| equal == wanted)
                .map_err(Failure::from),
            Test::Contains => contains(&key, &args[3], cx),
            Test::Order(relation) => key
                .relates(&args[3], relation, cx.span, cx.charge)
                .map_err(Failure::from),
        };
        match comparison {
            Ok(true) => {
                cx.list_limit(output.len() as u64 + 1)?;
                output.push(value.clone());
            }
            Ok(false) | Err(Failure::Value(_)) => (),
            Err(error) => return Err(error),
        }
    }
    cx.build_list(output)
}
