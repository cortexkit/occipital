//! The stream a move carries: an agent's chat display rows, one JSON object
//! per line.
//!
//! - The first line names the format and the agent, so a receiver decides
//!   whether it can read the stream from that line alone.
//! - Each row line is `{"table": <name>, "row": [<column values>]}`, with
//!   values as SQL stores them: null, an integer, text, or a blob as
//!   `{"hex": …}`.
//! - The last line, `{"end": {"sessions": […], "rows": N}}`, names the
//!   sessions the rows belong to and counts them, so a stream cut short reads
//!   as broken, not as fewer rows.

use std::io::{BufRead, Read as _, Write};

use rusqlite::types::Value as Sql;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::rows::{self, Row};

/// The stream's format, which `prepare` reports and `import` checks first.
const LINE_LIMIT: usize = 16 << 20;

pub(crate) const FORMAT: &str = "occipital-chat-display/v1";

/// One session the stream carries rows for, as its runner keys it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Session {
    pub runner: String,
    pub harness: String,
    pub session: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    format: String,
    agent: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct End {
    sessions: Vec<Session>,
    rows: u64,
}

/// Why a stream couldn't be read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unreadable {
    /// Its first line names another format, or none this build reads.
    Format(String),
    /// It breaks this format's rules.
    Broken(String),
    Apply(String),
}

/// Writes rows to `out` as the stream, hashing as it goes.
pub(crate) struct Writer<W: Write> {
    out: W,
    hash: Sha256,
    length: u64,
    rows: u64,
}

impl<W: Write> Writer<W> {
    pub(crate) fn start(out: W, agent: &str) -> std::io::Result<Self> {
        let mut writer = Self {
            out,
            hash: Sha256::new(),
            length: 0,
            rows: 0,
        };
        writer.line(&json!(Header {
            format: FORMAT.into(),
            agent: agent.into(),
        }))?;
        Ok(writer)
    }

    pub(crate) fn row(&mut self, (table, values): &Row) -> std::io::Result<()> {
        let values: Vec<Value> = values.iter().map(encode).collect::<Result<_, _>>()?;
        self.rows += 1;
        self.line(&json!({"table": table.name, "row": values}))
    }

    /// Write the last line and return the stream's length and SHA-256.
    pub(crate) fn finish(mut self, sessions: Vec<Session>) -> std::io::Result<(u64, String)> {
        let rows = self.rows;
        self.line(&json!({"end": End { sessions, rows }}))?;
        self.out.flush()?;
        Ok((self.length, hex(&self.hash.finalize())))
    }

    fn line(&mut self, value: &Value) -> std::io::Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        if bytes.len() > LINE_LIMIT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "move stream line exceeds limit",
            ));
        }
        bytes.push(b'\n');
        self.out.write_all(&bytes)?;
        self.hash.update(&bytes);
        self.length += bytes.len() as u64;
        Ok(())
    }
}

/// The identity and session list from a validated stream.
pub(crate) struct Read {
    pub agent: String,
    pub sessions: Vec<Session>,
}

/// Read a whole stream. The first line decides the format before anything
/// else is read.
pub(crate) fn read(
    mut input: impl BufRead,
    mut each: impl FnMut(Row) -> Result<(), String>,
) -> Result<Read, Unreadable> {
    let first = next_line(&mut input)?.ok_or_else(|| broken("the stream is empty"))?;
    let header: Value = serde_json::from_str(&first).map_err(broken)?;
    let format = header.get("format").and_then(Value::as_str).unwrap_or("");
    if format != FORMAT {
        return Err(Unreadable::Format(format.into()));
    }
    let header: Header = serde_json::from_value(header).map_err(broken)?;
    let mut rows = 0u64;
    let mut end = None;
    while let Some(line) = next_line(&mut input)? {
        if end.is_some() {
            return Err(broken("a line follows the end"));
        }
        let value: Value = serde_json::from_str(&line).map_err(broken)?;
        if let Some(found) = value.get("end") {
            end = Some(serde_json::from_value::<End>(found.clone()).map_err(broken)?);
            continue;
        }
        each(decode_row(&value)?).map_err(Unreadable::Apply)?;
        rows += 1;
    }
    let end = end.ok_or_else(|| broken("the stream has no end"))?;
    if end.rows != rows {
        return Err(broken(format!(
            "the end counts {} rows, the stream holds {}",
            end.rows, rows
        )));
    }
    Ok(Read {
        agent: header.agent,
        sessions: end.sessions,
    })
}

fn next_line(input: &mut impl BufRead) -> Result<Option<String>, Unreadable> {
    let mut bytes = Vec::new();
    let n = input
        .take(LINE_LIMIT as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(broken)?;
    if n == 0 {
        return Ok(None);
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.len() > LINE_LIMIT {
        return Err(broken("move stream line exceeds limit"));
    }
    String::from_utf8(bytes).map(Some).map_err(broken)
}

fn decode_row(value: &Value) -> Result<Row, Unreadable> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Line {
        table: String,
        row: Vec<Value>,
    }
    let line: Line = serde_json::from_value(value.clone()).map_err(broken)?;
    let table =
        rows::table(&line.table).ok_or_else(|| broken(format!("unknown table {}", line.table)))?;
    if line.row.len() != table.columns() {
        return Err(broken(format!(
            "a {} row has {} values, not {}",
            table.name,
            line.row.len(),
            table.columns()
        )));
    }
    let values = line.row.iter().map(decode).collect::<Result<_, _>>()?;
    Ok((table, values))
}

fn encode(value: &Sql) -> std::io::Result<Value> {
    Ok(match value {
        Sql::Null => Value::Null,
        Sql::Integer(n) => json!(n),
        Sql::Text(text) => json!(text),
        Sql::Blob(bytes) => json!({ "hex": hex(bytes) }),
        // No table a move carries has a real column; one would change this
        // format.
        Sql::Real(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "a real value has no form in this stream",
            ))
        }
    })
}

fn decode(value: &Value) -> Result<Sql, Unreadable> {
    match value {
        Value::Null => Ok(Sql::Null),
        Value::String(text) => Ok(Sql::Text(text.clone())),
        Value::Number(n) => n
            .as_i64()
            .map(Sql::Integer)
            .ok_or_else(|| broken(format!("{n} is not an integer"))),
        Value::Object(object) if object.len() == 1 => object
            .get("hex")
            .and_then(Value::as_str)
            .and_then(unhex)
            .map(Sql::Blob)
            .ok_or_else(|| broken("a blob is not hex")),
        other => Err(broken(format!("{other} is not a column value"))),
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn broken(why: impl std::fmt::Display) -> Unreadable {
    Unreadable::Broken(why.to_string())
}

#[cfg(test)]
mod tests;
