//! Format-hinted snapshot parsing (language specification §10): strict JSON
//! (§10.3) and CSV (§10.4) under the §10.5 caps, with the §10.6 rule that data
//! diagnostics carry positions and counts but never the data itself.
//!
//! Every parse runs in the data preparation account. An input within the
//! 2 MiB byte cap is charged 1 fuel per input byte before format parsing
//! starts, plus 2 per JSON value or CSV cell before that value or cell is
//! built (§9.2). An input over the cap fails uncharged. A failed charge stops the parse and is
//! returned to the caller, which owns the account and decides what an
//! exhausted account means.

mod csv;
mod json;

use std::collections::BTreeMap;

use crate::number::Decimal;
use crate::{
    DataPosition, Diagnostic, DiagnosticCode, DiagnosticPhase, SourceSpan, Value, ValueError,
};

/// The two `format` values of §10.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Format {
    Json,
    Csv,
}

/// A `columns` declaration: the type a named column converts to (§10.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ColumnType {
    Text,
    Number,
    Boolean,
}

/// The four CSV delimiters §10.2 allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Delimiter {
    Comma,
    Semicolon,
    Tab,
    Pipe,
}

impl Delimiter {
    fn byte(self) -> u8 {
        match self {
            Delimiter::Comma => b',',
            Delimiter::Semicolon => b';',
            Delimiter::Tab => b'\t',
            Delimiter::Pipe => b'|',
        }
    }
}

/// Parse options, already validated by the checker: they are literals, and an
/// unknown option or value is a check error (§10.2), so none reaches here.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Options {
    /// Declared conversions in the order the source literal lists them. Empty
    /// means no conversions.
    pub columns: Vec<(String, ColumnType)>,
    pub delimiter: Delimiter,
    pub header: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            columns: Vec::new(),
            delimiter: Delimiter::Comma,
            header: true,
        }
    }
}

/// The snapshot members a parse sets (§10.2). `text` is always kept, even when
/// the parse fails; then `value` and `rows` are both the parse's error value.
#[derive(Clone, Debug)]
pub(crate) struct Prepared {
    pub text: Value,
    pub value: Value,
    /// The root when it is a list; missing for any other JSON root.
    pub rows: Value,
    /// At most one error, which is then the only diagnostic; otherwise the
    /// warnings, ordered by their position in the snapshot.
    pub diagnostics: Vec<Diagnostic>,
}

const BYTES_CAP: usize = 2 * 1024 * 1024;
const DEPTH_CAP: u32 = 32;
const ROWS_CAP: usize = 10_000;
const COLUMNS_CAP: usize = 256;
const CELLS_CAP: u64 = 200_000;
const STRING_CAP: u64 = 65_536;
const SIZE_CAP: u64 = 16 * 1024 * 1024;

/// The caps of §10.5. Breaking one fails the parse with `E_DATA_LIMIT`, whose
/// subject and message name the cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cap {
    Bytes,
    Depth,
    Rows,
    Columns,
    Cells,
    String,
    Size,
}

impl Cap {
    fn name(self) -> &'static str {
        match self {
            Cap::Bytes => "bytes",
            Cap::Depth => "depth",
            Cap::Rows => "rows",
            Cap::Columns => "columns",
            Cap::Cells => "cells",
            Cap::String => "string",
            Cap::Size => "size",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Cap::Bytes => "Data exceeds the cap of 2 MiB parsed bytes.",
            Cap::Depth => "Data exceeds the cap of 32 nested JSON arrays and objects.",
            Cap::Rows => "Data exceeds the cap of 10000 rows.",
            Cap::Columns => "Data exceeds the cap of 256 columns.",
            Cap::Cells => "Data exceeds the cap of 200000 cells or values.",
            Cap::String => "A data string or cell exceeds the cap of 65536 scalars.",
            Cap::Size => "Data exceeds the cap of 16 MiB expanded logical size.",
        }
    }
}

/// A problem found while parsing. Its position stays a byte offset until the
/// parse ends, so that every position is resolved in one pass over the text.
#[derive(Debug)]
struct Finding {
    code: DiagnosticCode,
    offset: Option<usize>,
    record: Option<u64>,
    /// The broken cap of an `E_DATA_LIMIT`, named as the subject.
    cap: Option<Cap>,
    /// A column name from the source's `columns` literal, named as the
    /// subject. It is never text taken from the snapshot.
    column: Option<String>,
    message: Option<(&'static str, Vec<u64>)>,
}

impl Finding {
    fn at(code: DiagnosticCode, offset: usize) -> Self {
        Self {
            code,
            offset: Some(offset),
            record: None,
            cap: None,
            column: None,
            message: None,
        }
    }

    fn limit(cap: Cap, offset: Option<usize>) -> Self {
        Self {
            code: DiagnosticCode::DataLimit,
            offset,
            record: None,
            cap: Some(cap),
            column: None,
            message: None,
        }
    }

    fn counts(mut self, template: &'static str, counts: Vec<u64>) -> Self {
        self.message = Some((template, counts));
        self
    }
}

/// Why a parse stopped: the caller's fuel account refused a charge, or the
/// data failed with the given error.
enum Stop<E> {
    Fuel(E),
    Fail(Finding),
}

impl<E> Stop<E> {
    fn syntax(code: DiagnosticCode, offset: usize) -> Self {
        Stop::Fail(Finding::at(code, offset))
    }

    fn limit(cap: Cap, offset: usize) -> Self {
        Stop::Fail(Finding::limit(cap, Some(offset)))
    }

    /// Attach the CSV record number an error occurred in.
    fn in_record(self, record: u64) -> Self {
        match self {
            Stop::Fail(mut finding) => {
                finding.record = Some(record);
                Stop::Fail(finding)
            }
            fuel => fuel,
        }
    }
}

type Step<T, E> = Result<T, Stop<E>>;

/// Low-level parsers also accept test fuel callbacks. Runtime preparation uses
/// Unit so expansion and retained allocations reserve every enclosing scope.
pub(crate) trait Charge<E> {
    fn fuel(&mut self, amount: u64) -> Result<(), E>;
    fn expanded(&mut self, _amount: u64) -> Result<(), E> {
        Ok(())
    }
    fn allocation(&mut self, _requested: u64) -> Result<(), E> {
        Ok(())
    }
    fn shrink_allocation(&mut self, _requested: u64) -> Result<(), E> {
        Ok(())
    }
}

impl<E, F: FnMut(u64) -> Result<(), E>> Charge<E> for F {
    fn fuel(&mut self, amount: u64) -> Result<(), E> {
        self(amount)
    }
}

impl Charge<crate::accounts::Refused> for crate::accounts::Unit {
    fn shrink_allocation(&mut self, requested: u64) -> Result<(), crate::accounts::Refused> {
        self.shrink_last(crate::RetainedAllocation {
            requested,
            unused_capacity: 0,
        })
    }
    fn fuel(&mut self, amount: u64) -> Result<(), crate::accounts::Refused> {
        self.charge(amount)
    }
    fn expanded(&mut self, amount: u64) -> Result<(), crate::accounts::Refused> {
        self.expand(amount)
    }
    fn allocation(&mut self, requested: u64) -> Result<(), crate::accounts::Refused> {
        self.retain(crate::RetainedAllocation {
            requested,
            unused_capacity: 0,
        })
    }
}

/// Fuel, the value/cell count and the expanded logical size of one parse.
struct Meter<'a, E, F: Charge<E>> {
    charge: &'a mut F,
    values: u64,
    size: u64,
    error: std::marker::PhantomData<E>,
}

impl<E, F: Charge<E>> Meter<'_, E, F> {
    fn pay(&mut self, fuel: u64) -> Step<(), E> {
        self.charge.fuel(fuel).map_err(Stop::Fuel)
    }

    fn allocation(&mut self, requested: usize) -> Step<(), E> {
        self.charge.allocation(requested as u64).map_err(Stop::Fuel)
    }

    fn slice<T>(&mut self, len: usize) -> Step<(), E> {
        // Arc<[T]> requests the payload and two atomic reference counters.
        self.allocation(std::mem::size_of::<usize>() * 2 + len * std::mem::size_of::<T>())
    }

    fn scalar(&mut self, scalar: &Scalar) -> Step<(), E> {
        match scalar {
            Scalar::Text(s) => self.slice::<u8>(s.len()),
            Scalar::Error(code) => self.slice::<u8>(code.len()),
            _ => Ok(()),
        }
    }

    /// Count one JSON value or CSV cell against its cap, then charge for it.
    fn value(&mut self, offset: usize) -> Step<(), E> {
        if self.values == CELLS_CAP {
            return Err(Stop::limit(Cap::Cells, offset));
        }
        self.values += 1;
        self.pay(2)
    }

    /// Add a value's own §9.3 logical size before it is built.
    fn grow(&mut self, amount: u64, offset: usize) -> Step<(), E> {
        let size = self.size.saturating_add(amount);
        if size > SIZE_CAP {
            return Err(Stop::limit(Cap::Size, offset));
        }
        self.charge.expanded(amount).map_err(Stop::Fuel)?;
        self.size = size;
        Ok(())
    }
}

/// How often a once-per-parse or once-per-column warning applies, and where
/// it first applied.
#[derive(Clone, Copy, Debug, Default)]
struct Tally {
    count: u64,
    offset: usize,
    row: u64,
    record: Option<u64>,
}

impl Tally {
    fn note(&mut self, offset: usize, row: u64, record: Option<u64>) {
        if self.count == 0 {
            self.offset = offset;
            self.row = row;
            self.record = record;
        }
        self.count += 1;
    }

    fn finding(&self, code: DiagnosticCode) -> Option<Finding> {
        (self.count > 0).then(|| {
            let mut finding = Finding::at(code, self.offset);
            finding.record = self.record;
            finding
        })
    }
}

/// One `W_CELL_TYPE` per declared column with failed conversions, giving the
/// count and the first row (§10.3, §10.4), in declaration order.
fn cell_type_findings(columns: &[(String, ColumnType)], tallies: &[Tally]) -> Vec<Finding> {
    columns
        .iter()
        .zip(tallies)
        .filter_map(|((name, _), tally)| {
            let mut finding = tally.finding(DiagnosticCode::CellType)?;
            finding.column = Some(name.clone());
            Some(finding.counts(
                "{} values in this column could not be converted; the first is in row {}.",
                vec![tally.count, tally.row],
            ))
        })
        .collect()
}

/// A scalar about to be built, so its logical size can be checked first.
enum Scalar {
    Text(String),
    Number(Decimal),
    Boolean(bool),
    Null,
    Error(&'static str),
}

impl Scalar {
    /// The §9.3 logical size the built value will have.
    fn size(&self) -> u64 {
        match self {
            Scalar::Text(text) => 16 + 4 * text.chars().count() as u64,
            _ => 16,
        }
    }

    fn build(self, span: SourceSpan) -> Value {
        match self {
            // The size was checked against the 16 MiB cap, so it cannot overflow.
            Scalar::Text(text) => Value::text(text),
            Scalar::Number(number) => Value::number(number),
            Scalar::Boolean(value) => Value::boolean(value),
            Scalar::Null => Value::null(),
            Scalar::Error(code) => Value::error(ValueError::new(code, span)),
        }
    }
}

/// Convert a string cell to its declared type. The flag reports a failed
/// conversion, which counts toward `W_CELL_TYPE`.
///
/// JSON follows the §10.3 table: for `"number"` only the empty string is null,
/// and other text goes through the `Number(s)` rules, which trim white space.
/// CSV follows §10.4: a `"number"` cell has white space trimmed first, a
/// `"boolean"` cell must be exactly `true` or `false`, and an empty cell is
/// null for both.
fn convert_string(text: String, column: ColumnType, format: Format) -> (Scalar, bool) {
    match column {
        ColumnType::Text => (Scalar::Text(text), false),
        ColumnType::Number => {
            let empty = match format {
                Format::Json => text.is_empty(),
                Format::Csv => crate::unicode::trim(&text).is_empty(),
            };
            if empty {
                return (Scalar::Null, false);
            }
            match Decimal::parse_text(crate::unicode::trim(&text)) {
                Ok(number) => (Scalar::Number(number), false),
                Err(error) => (Scalar::Error(error.code()), true),
            }
        }
        ColumnType::Boolean => match text.as_str() {
            "true" => (Scalar::Boolean(true), false),
            "false" => (Scalar::Boolean(false), false),
            "" if format == Format::Csv => (Scalar::Null, false),
            _ => (Scalar::Error("E_TYPE"), true),
        },
    }
}

/// The snapshot's `text` (§10.1): a leading UTF-8 byte-order mark removed and
/// each invalid sequence replaced with U+FFFD; nothing else changes.
pub(crate) fn snapshot_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(strip_bom(bytes)).into_owned()
}

/// Runtime entry point: unlike the low-level parser's test callbacks, this
/// cannot omit per-unit, cumulative expansion or retained-memory admission.
pub(crate) fn prepare_accounted(
    accounts: &crate::accounts::Accounts,
    kind: crate::accounts::UnitKind,
    bytes: &[u8],
    format: Format,
    options: &Options,
    span: SourceSpan,
) -> Result<Prepared, crate::accounts::Refused> {
    let mut unit = accounts
        .begin(kind)
        .map_err(crate::accounts::Refused::Aggregate)?;
    let mut prepared = prepare(bytes, format, options, span, &mut unit)?;
    // Remove the list alias before attaching guards, so Arc::make_mut need
    // not copy the newly built value graph. Extracted descendants keep guards.
    prepared.rows = Value::missing();
    let retention = unit.finish()?;
    prepared.text.attach_retention(&retention);
    prepared.value.attach_retention(&retention);
    if prepared.value.value_type() == crate::ValueType::List {
        prepared.rows = prepared.value.clone();
    }
    Ok(prepared)
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

/// Parse a snapshot's bytes under a format hint (§10.2).
///
/// `span` is the source span of the `Parse` call or `data` hint; error values
/// built from the data (an out-of-range number, a failed cell conversion, a
/// failed parse) carry it. `charge` is the data preparation account. A refused
/// charge stops the parse and its error is returned as `Err`.
pub(crate) fn prepare<E>(
    bytes: &[u8],
    format: Format,
    options: &Options,
    span: SourceSpan,
    charge: &mut impl Charge<E>,
) -> Result<Prepared, E> {
    // The lossy UTF-8 text can occupy at most three bytes per input byte.
    // Reserve that bound before repair, separately from expanded parsed values.
    charge.allocation((bytes.len() as u64).saturating_mul(3).saturating_add(16))?;
    let text = snapshot_text(bytes);
    charge.shrink_allocation(text.len() as u64 + 16)?;
    let text = Value::text(text);
    let mut meter = Meter {
        charge,
        values: 0,
        size: 0,
        error: std::marker::PhantomData,
    };
    let body = strip_bom(bytes);
    // Offsets are resolved against the longest valid UTF-8 prefix; an
    // encoding error sits just after it.
    let valid = match std::str::from_utf8(body) {
        Ok(valid) => valid,
        // The prefix before the first invalid sequence is valid UTF-8.
        Err(error) => std::str::from_utf8(&body[..error.valid_up_to()]).unwrap_or_default(),
    };
    let outcome = parse(bytes, body, format, options, span, &mut meter);
    let failed = |finding: Finding| {
        let error = Value::error(ValueError::new(finding.code.as_str(), span));
        Prepared {
            text: text.clone(),
            value: error.clone(),
            rows: error,
            diagnostics: resolve(valid, vec![finding]),
        }
    };
    match outcome {
        Ok((value, findings)) => {
            let rows = match value.value_type() {
                crate::ValueType::List => value.clone(),
                _ => Value::missing(),
            };
            Ok(Prepared {
                text: text.clone(),
                value,
                rows,
                diagnostics: resolve(valid, findings),
            })
        }
        Err(Stop::Fail(finding)) => Ok(failed(finding)),
        Err(Stop::Fuel(error)) => Err(error),
    }
}

fn parse<E, F: Charge<E>>(
    bytes: &[u8],
    body: &[u8],
    format: Format,
    options: &Options,
    span: SourceSpan,
    meter: &mut Meter<'_, E, F>,
) -> Step<(Value, Vec<Finding>), E> {
    // The byte cap is checked before charging and before format parsing, so
    // an oversized input is not charged and its diagnostic has no position.
    if bytes.len() > BYTES_CAP {
        return Err(Stop::Fail(Finding::limit(Cap::Bytes, None)));
    }
    meter.pay(bytes.len() as u64)?;
    // Only UTF-8 is accepted. UTF-16 and UTF-32 byte-order marks are named
    // here, although each also starts with bytes that are invalid UTF-8.
    let wide_bom = [&b"\xFF\xFE"[..], b"\xFE\xFF", b"\x00\x00\xFE\xFF"]
        .iter()
        .any(|bom| bytes.starts_with(bom));
    if wide_bom {
        return Err(Stop::syntax(DiagnosticCode::DataEncoding, 0));
    }
    let text = std::str::from_utf8(body)
        .map_err(|error| Stop::syntax(DiagnosticCode::DataEncoding, error.valid_up_to()))?;
    match format {
        Format::Json => json::parse(text, &options.columns, span, meter),
        Format::Csv => csv::parse(text, options, span, meter),
    }
}

/// Turn findings into data-phase diagnostics, resolving every byte offset to
/// a one-based line and scalar column in a single pass over the text.
fn resolve(text: &str, mut findings: Vec<Finding>) -> Vec<Diagnostic> {
    findings.sort_by_key(|finding| (finding.offset.is_none(), finding.offset));
    let positions = positions(text, findings.iter().filter_map(|f| f.offset).collect());
    findings
        .into_iter()
        .map(|finding| {
            let mut diagnostic = Diagnostic::new(finding.code, DiagnosticPhase::Data);
            // `positions` holds an entry for every finding's offset, so the
            // lookup cannot miss; were it to, the diagnostic stays unpositioned.
            if let Some(&(line, column)) = finding.offset.and_then(|offset| positions.get(&offset))
            {
                // Positions and record numbers count from one, so each step
                // succeeds; `refine` keeps the diagnostic if one ever did not.
                diagnostic = diagnostic.refine(|diagnostic| {
                    let mut position = DataPosition::new(line, column)?;
                    if let Some(record) = finding.record {
                        position = position.with_record(record)?;
                    }
                    diagnostic.with_span(position)
                });
            }
            if let Some(cap) = finding.cap {
                // Cap messages are fixed and short.
                diagnostic = diagnostic
                    .with_subject(cap.name())
                    .refine(|diagnostic| diagnostic.with_message(cap.message()));
            }
            if let Some(column) = &finding.column {
                diagnostic = diagnostic.with_subject(column);
            }
            if let Some((template, counts)) = &finding.message {
                // Count templates are fixed and short, even with 20-digit counts.
                diagnostic =
                    diagnostic.refine(|diagnostic| diagnostic.with_counts(template, counts));
            }
            diagnostic
        })
        .collect()
}

/// One-based (line, scalar column) for each byte offset. A line ends at LF, at
/// CRLF, or at a lone CR. An offset past the text maps to the end.
fn positions(text: &str, mut offsets: Vec<usize>) -> BTreeMap<usize, (u32, u32)> {
    offsets.sort_unstable();
    offsets.dedup();
    let mut resolved = BTreeMap::new();
    let mut targets = offsets.into_iter().peekable();
    let (mut line, mut column) = (1_u32, 1_u32);
    let bytes = text.as_bytes();
    let ends = std::iter::once((text.len(), '\n'));
    for (index, c) in text.char_indices().chain(ends) {
        while let Some(&target) = targets.peek() {
            if target > index && index < text.len() {
                break;
            }
            resolved.insert(target, (line, column));
            targets.next();
        }
        if index == text.len() {
            break;
        }
        let breaks = c == '\n' || (c == '\r' && bytes.get(index + 1) != Some(&b'\n'));
        if breaks {
            line = line.saturating_add(1);
            column = 1;
        } else {
            column = column.saturating_add(1);
        }
    }
    resolved
}

#[cfg(test)]
mod tests;
