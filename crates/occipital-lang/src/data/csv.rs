//! RFC 4180 CSV with the §10.4 choices, building the row records directly so
//! every §10.5 cap is checked before the value it guards is built.

use std::collections::HashSet;

use super::{
    cell_type_findings, convert_string, Cap, ColumnType, Finding, Format, Meter, Options, Scalar,
    Step, Stop, Tally, COLUMNS_CAP, ROWS_CAP, STRING_CAP,
};
use crate::{DiagnosticCode, ScalarString, SourceSpan, Value};

/// One parsed field and the byte offset where it starts.
struct Cell {
    offset: usize,
    text: String,
}

/// A record, the byte offset where it starts, and its one-based number among
/// all records of the snapshot, the header included.
struct Record {
    offset: usize,
    number: u64,
    cells: Vec<Cell>,
}

struct Reader<'t> {
    text: &'t str,
    bytes: &'t [u8],
    at: usize,
    delimiter: u8,
    records: u64,
}

pub(super) fn parse<E, F: super::Charge<E>>(
    text: &str,
    options: &Options,
    span: SourceSpan,
    meter: &mut Meter<'_, E, F>,
) -> Step<(Value, Vec<Finding>), E> {
    let mut reader = Reader {
        text,
        bytes: text.as_bytes(),
        at: 0,
        delimiter: options.delimiter.byte(),
        records: 0,
    };

    let mut findings = Vec::new();
    meter.grow(16, 0)?;
    let first = match reader.next_record_start()? {
        Some(_) => Some(reader.record(meter, true)?),
        None => None,
    };
    let (names, header, pending) = match first {
        Some(first) if options.header => {
            let (names, renamed) = rename(&first, meter)?;
            findings.extend(renamed.finding(DiagnosticCode::CsvHeaderRenamed).map(|f| {
                f.counts(
                    "{} CSV header names were empty or repeated and were renamed; the first is column {}.",
                    vec![renamed.count, renamed.row],
                )
            }));
            (names, Some((first.offset, first.number)), None)
        }
        Some(first) => {
            let mut names = Vec::new();
            for position in 1..=first.cells.len() {
                let name = format!("column{position}");
                meter.slice::<u8>(name.len())?;
                names.push(ScalarString::new(name));
            }
            (names, Some((first.offset, first.number)), Some(first))
        }
        None => (Vec::new(), None, None),
    };

    // Each column's declared type, by its index in `options.columns`. A
    // declared column the header lacks is reported once, by name.
    let types: Vec<Option<(usize, ColumnType)>> = names
        .iter()
        .map(|name| {
            options
                .columns
                .iter()
                .position(|(declared, _)| declared == name.as_str())
                .map(|index| (index, options.columns[index].1))
        })
        .collect();
    for (index, (declared, _)) in options.columns.iter().enumerate() {
        if !types.iter().flatten().any(|(found, _)| *found == index) {
            findings.push(Finding {
                code: DiagnosticCode::CsvUnknownColumn,
                offset: header.map(|(offset, _)| offset),
                record: header.map(|(_, number)| number),
                cap: None,
                column: Some(declared.clone()),
                message: None,
            });
        }
    }

    let mut rows = Vec::new();
    let mut short = Tally::default();
    let mut long = Tally::default();
    let mut cell_types = vec![Tally::default(); options.columns.len()];
    let mut next = pending;
    loop {
        let record = match next.take() {
            Some(record) => record,
            None => match reader.next_record_start()? {
                None => break,
                Some(offset) => {
                    if rows.len() == ROWS_CAP {
                        return Err(Stop::limit(Cap::Rows, offset).in_record(reader.records + 1));
                    }
                    reader.record(meter, false)?
                }
            },
        };
        let row = rows.len() as u64 + 1;
        let number = record.number;
        let mut cells = record.cells.into_iter();
        meter
            .grow(16, record.offset)
            .map_err(|s| s.in_record(number))?;
        let mut fields = Vec::with_capacity(names.len());
        let mut missing = false;
        for (name, declared) in names.iter().zip(&types) {
            let (scalar, offset) = match cells.next() {
                // A missing cell is null (§10.4).
                None => {
                    missing = true;
                    (Scalar::Null, record.offset)
                }
                Some(cell) => match declared {
                    None => (Scalar::Text(cell.text), cell.offset),
                    Some((index, column)) => {
                        let (scalar, failed) = convert_string(cell.text, *column, Format::Csv);
                        if failed {
                            cell_types[*index].note(cell.offset, row, Some(number));
                        }
                        (scalar, cell.offset)
                    }
                },
            };
            meter
                .grow(16 + 4 * name.len() + scalar.size(), offset)
                .map_err(|s| s.in_record(number))?;
            meter.scalar(&scalar)?;
            fields.push((name.clone(), scalar.build(span)));
        }
        if missing {
            short.note(record.offset, row, Some(number));
        }
        // Cells beyond the header are dropped (§10.4).
        if cells.len() > 0 {
            long.note(record.offset, row, Some(number));
        }
        meter.slice::<(ScalarString, Value)>(fields.len())?;
        rows.push(Value::record(fields).map_err(|_| Stop::limit(Cap::Size, record.offset))?);
    }

    findings.extend(short.finding(DiagnosticCode::CsvShortRow).map(|f| {
        f.counts(
            "{} CSV records have missing cells; the first is record {}.",
            vec![short.count, short.record.unwrap_or_default()],
        )
    }));
    findings.extend(long.finding(DiagnosticCode::CsvLongRow).map(|f| {
        f.counts(
            "{} CSV records have extra cells, which were dropped; the first is record {}.",
            vec![long.count, long.record.unwrap_or_default()],
        )
    }));
    findings.extend(cell_type_findings(&options.columns, &cell_types));
    meter.slice::<Value>(rows.len())?;
    let value = Value::list(rows).map_err(|_| Stop::limit(Cap::Size, 0))?;
    Ok((value, findings))
}

/// Header names (§10.4): an empty name becomes `column` and its position from
/// 1; a repeated name keeps its first occurrence and later ones become
/// `name_2`, `name_3`, …, skipping names already taken. The tally counts the
/// renamed columns, with the first one's position in `row`.
fn rename<E, F: super::Charge<E>>(
    header: &Record,
    meter: &mut Meter<'_, E, F>,
) -> Step<(Vec<ScalarString>, Tally), E> {
    let mut taken = HashSet::new();
    let mut renamed = Tally::default();
    let mut names = Vec::with_capacity(header.cells.len());
    for (index, cell) in header.cells.iter().enumerate() {
        let position = index as u64 + 1;
        let mut name = if cell.text.is_empty() {
            format!("column{position}")
        } else {
            cell.text.clone()
        };
        let mut changed = cell.text.is_empty();
        if taken.contains(&name) {
            changed = true;
            let mut suffix = 2_u64;
            while taken.contains(&format!("{name}_{suffix}")) {
                suffix += 1;
            }
            name = format!("{name}_{suffix}");
        }
        if changed {
            renamed.note(cell.offset, position, Some(header.number));
        }
        taken.insert(name.clone());
        meter.slice::<u8>(name.len())?;
        names.push(ScalarString::new(name));
    }
    Ok((names, renamed))
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn syntax<T, E>(&self, offset: usize) -> Step<T, E> {
        Err(Stop::syntax(DiagnosticCode::CsvSyntax, offset).in_record(self.records))
    }

    /// Skip empty lines, which hold nothing between two breaks, and return
    /// where the next record starts, or `None` at the end. A lone CR outside
    /// quotes is a syntax error.
    fn next_record_start<E>(&mut self) -> Step<Option<usize>, E> {
        loop {
            match self.peek() {
                None => return Ok(None),
                Some(b'\n') => self.at += 1,
                Some(b'\r') if self.bytes.get(self.at + 1) == Some(&b'\n') => self.at += 2,
                Some(b'\r') => {
                    return Err(Stop::syntax(DiagnosticCode::CsvSyntax, self.at)
                        .in_record(self.records + 1))
                }
                Some(_) => return Ok(Some(self.at)),
            }
        }
    }

    /// Read one record starting at a non-empty line. The first record defines
    /// the columns, so the column cap applies to it.
    fn record<E, F: super::Charge<E>>(
        &mut self,
        meter: &mut Meter<'_, E, F>,
        first: bool,
    ) -> Step<Record, E> {
        self.records += 1;
        let number = self.records;
        let offset = self.at;
        let mut cells = Vec::new();
        loop {
            let start = self.at;
            if first && cells.len() == COLUMNS_CAP {
                return Err(Stop::limit(Cap::Columns, start).in_record(number));
            }
            meter.value(start).map_err(|s| s.in_record(number))?;
            let text = if self.peek() == Some(b'"') {
                self.quoted()?
            } else {
                self.unquoted()?
            };
            cells.push(Cell {
                offset: start,
                text,
            });
            match self.peek() {
                None => break,
                Some(byte) if byte == self.delimiter => self.at += 1,
                Some(b'\n') => {
                    self.at += 1;
                    break;
                }
                Some(b'\r') if self.bytes.get(self.at + 1) == Some(&b'\n') => {
                    self.at += 2;
                    break;
                }
                // A lone CR, or anything after a closing quote other than a
                // delimiter, a line break or the end.
                Some(_) => return self.syntax(self.at),
            }
        }
        Ok(Record {
            offset,
            number,
            cells,
        })
    }

    /// An unquoted field runs to the next delimiter or line break; spaces are
    /// kept, and a quote inside it is a syntax error.
    fn unquoted<E>(&mut self) -> Step<String, E> {
        let start = self.at;
        let mut scalars = 0_u64;
        while let Some(byte) = self.peek() {
            if byte == self.delimiter || byte == b'\n' || byte == b'\r' {
                break;
            }
            if byte == b'"' {
                return self.syntax(self.at);
            }
            // Count scalars by their leading bytes.
            if byte & 0xC0 != 0x80 {
                scalars += 1;
                if scalars > STRING_CAP {
                    return Err(Stop::limit(Cap::String, start).in_record(self.records));
                }
            }
            self.at += 1;
        }
        Ok(self.text[start..self.at].to_owned())
    }

    /// A quoted field ends at the next lone quote. Inside it `""` is a quote,
    /// and delimiters and line breaks are kept exactly. A quote still open at
    /// the end is a syntax error at the opening quote.
    fn quoted<E>(&mut self) -> Step<String, E> {
        let start = self.at;
        self.at += 1;
        let mut text = String::new();
        let mut scalars = 0_u64;
        loop {
            let c = match self.text[self.at..].chars().next() {
                None => return self.syntax(start),
                Some('"') if self.bytes.get(self.at + 1) == Some(&b'"') => {
                    self.at += 2;
                    '"'
                }
                Some('"') => {
                    self.at += 1;
                    return Ok(text);
                }
                Some(c) => {
                    self.at += c.len_utf8();
                    c
                }
            };
            scalars += 1;
            if scalars > STRING_CAP {
                return Err(Stop::limit(Cap::String, start).in_record(self.records));
            }
            text.push(c);
        }
    }
}
