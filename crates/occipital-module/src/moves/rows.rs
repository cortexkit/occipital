//! The rows a move carries for a set of sessions, copied whole from the four
//! files they live in.
//!
//! Every table here is keyed so a row means the same on any computer: by
//! session name, by the runner that minted a call and its call key, or by a
//! sender and its send id. So a row is exported as its column values, and
//! the receiver keeps any existing payload. Pins and riding notices take the
//! incoming session because their call can move between sessions.
//!
//! Tool records and details carry no session; they belong to the sessions
//! whose calls they answer, found through the call pins.

use std::collections::{BTreeMap, BTreeSet};

use cortexkit_store::SqliteStore;
use rusqlite::types::Value as Sql;
use subc_protocol::Principal;

/// The four files, as a move reads and writes them.
pub(crate) struct Files<'a> {
    pub records: &'a SqliteStore,
    pub details: &'a SqliteStore,
    pub notices: &'a SqliteStore,
    pub store: &'a SqliteStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum File {
    Records,
    Details,
    Notices,
    Store,
}

/// How a table's rows are chosen for a set of sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Belongs {
    /// Its `session` column names one of them.
    Session,
    /// It answers a call the sessions' call pins name: its `runner` column is
    /// the minting runner's principal, its `call_key` the call's key.
    Call,
}

pub(crate) struct Table {
    /// The name a row carries in the stream.
    pub name: &'static str,
    file: File,
    sql_name: &'static str,
    /// Every column, in the order a row carries them. A test checks this
    /// against the table, so a column added later is never left behind.
    columns: &'static [&'static str],
    /// The primary key's columns, which delete a row.
    key: &'static [&'static str],
    belongs: Belongs,
}

const WRITE_COLUMNS: &[&str] = &[
    "key",
    "writer",
    "runner",
    "call_key",
    "kind",
    "schema_pin",
    "body",
    "stored_at_ms",
];

/// Call pins come first: the tool records and details that follow are chosen
/// by them, on export and on delete.
pub(crate) const TABLES: &[Table] = &[
    Table {
        name: "call_pins",
        file: File::Records,
        sql_name: "display_call_pins",
        columns: &["runner", "call_key", "session", "pin", "seen_at_ms"],
        key: &["runner", "call_key"],
        belongs: Belongs::Session,
    },
    Table {
        name: "records",
        file: File::Records,
        sql_name: "display_writes",
        columns: WRITE_COLUMNS,
        key: &["key"],
        belongs: Belongs::Call,
    },
    Table {
        name: "details",
        file: File::Details,
        sql_name: "display_writes",
        columns: WRITE_COLUMNS,
        key: &["key"],
        belongs: Belongs::Call,
    },
    Table {
        name: "notices",
        file: File::Notices,
        sql_name: "notices",
        columns: &[
            "runner",
            "call_key",
            "sender",
            "send_id",
            "session",
            "text",
            "delivered_as",
            "speaks_for",
            "position",
            "seen_at_ms",
        ],
        key: &["runner", "call_key", "sender", "send_id"],
        belongs: Belongs::Session,
    },
    Table {
        name: "wake_records",
        file: File::Notices,
        sql_name: "wake_records",
        columns: &[
            "session",
            "sender",
            "send_id",
            "runner",
            "body",
            "written_at_ms",
        ],
        key: &["session", "sender", "send_id"],
        belongs: Belongs::Session,
    },
    Table {
        name: "sends",
        file: File::Notices,
        sql_name: "sends",
        columns: &[
            "runner",
            "session",
            "mid",
            "sender",
            "send_id",
            "seen_at_ms",
        ],
        key: &["runner", "session", "mid"],
        belongs: Belongs::Session,
    },
    Table {
        name: "queued_sends",
        file: File::Store,
        sql_name: "harness_app_send_queue",
        columns: &[
            "session",
            "client_message_id",
            "harness",
            "send_id",
            "accepted_at",
            "steer_until",
            "state",
            "reason",
            "ended_at",
        ],
        key: &["session", "client_message_id"],
        belongs: Belongs::Session,
    },
    Table {
        name: "send_waits",
        file: File::Store,
        sql_name: "harness_app_send_wait",
        columns: &[
            "session",
            "client_message_id",
            "runner_code",
            "retry_after_ms",
            "cause",
        ],
        key: &["session", "client_message_id"],
        belongs: Belongs::Session,
    },
];

pub(crate) fn table(name: &str) -> Option<&'static Table> {
    TABLES.iter().find(|table| table.name == name)
}

impl Table {
    pub(crate) fn columns(&self) -> usize {
        self.columns.len()
    }

    /// Where a row of this table names its session, if it does.
    pub(crate) fn session_column(&self) -> Option<usize> {
        self.columns.iter().position(|name| *name == "session")
    }

    fn position(&self, column: &str) -> usize {
        // Every name looked up here is one of the table's own columns, which
        // the schema test checks.
        self.columns
            .iter()
            .position(|name| *name == column)
            .unwrap_or(0)
    }
}

/// One row: its table and its column values.
pub(crate) type Row = (&'static Table, Vec<Sql>);

impl Files<'_> {
    fn file(&self, file: File) -> &SqliteStore {
        match file {
            File::Records => self.records,
            File::Details => self.details,
            File::Notices => self.notices,
            File::Store => self.store,
        }
    }

    /// Write matching rows directly from SQLite's cursor. Only pin identities
    /// cross database boundaries: a detail lives in a different file from
    /// the pin that places it in a session.
    pub(crate) fn export(
        &self,
        sessions: &BTreeSet<String>,
        mut each: impl FnMut(Row) -> Result<(), String>,
    ) -> Result<BTreeMap<&'static str, u64>, String> {
        let sessions = serde_json::to_string(sessions).map_err(|e| e.to_string())?;
        let mut counts = BTreeMap::new();
        for table in TABLES {
            let mut count = 0;
            let mut emit = |row: &rusqlite::Row<'_>| -> rusqlite::Result<()> {
                let values = (0..table.columns())
                    .map(|i| row.get(i))
                    .collect::<rusqlite::Result<_>>()?;
                each((table, values)).map_err(|e| {
                    rusqlite::Error::ToSqlConversionFailure(std::io::Error::other(e).into())
                })?;
                count += 1;
                Ok(())
            };
            let select = format!("SELECT {} FROM {}", table.columns.join(","), table.sql_name);
            if table.belongs == Belongs::Call {
                let by_call = format!("{select} WHERE runner=?1 AND call_key=?2");
                self.records.with_conn(|conn| {
                    let mut pins = conn.prepare("SELECT runner, call_key FROM display_call_pins WHERE session IN (SELECT value FROM json_each(?1))")?;
                    let mut pins = pins.query([&sessions])?;
                    while let Some(row) = pins.next()? {
                        let runner = crate::display::store::principal_json(&Principal::Reserved { module_id: row.get(0)? }).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
                        let call: String = row.get(1)?;
                        let mut take = |conn: &rusqlite::Connection| {
                            let mut stmt = conn.prepare_cached(&by_call)?;
                            let mut rows = stmt.query([&runner,&call])?;
                            while let Some(row) = rows.next()? { emit(row)?; }
                            Ok(())
                        };
                        if table.file == File::Records { take(conn)?; } else {
                            self.details.with_conn(take).map_err(|e|rusqlite::Error::ToSqlConversionFailure(e.into()))?;
                        }
                    }
                    Ok(())
                }).map_err(|e| e.to_string())?;
            } else {
                let mut select =
                    format!("{select} WHERE session IN (SELECT value FROM json_each(?1))");
                if table.name == "wake_records" {
                    // An exact-session wake is selected above. A row carried
                    // from another session resolves only a unique sender/send.
                    select.push_str(" OR ((sender,send_id) IN (SELECT sender,send_id FROM sends WHERE session IN (SELECT value FROM json_each(?1)) UNION SELECT sender,send_id FROM notices WHERE session IN (SELECT value FROM json_each(?1))) AND NOT EXISTS (SELECT 1 FROM wake_records other WHERE other.sender=wake_records.sender AND other.send_id=wake_records.send_id AND other.session!=wake_records.session))");
                }
                self.file(table.file)
                    .with_conn(|conn| {
                        let mut stmt = conn.prepare(&select)?;
                        let mut rows = stmt.query([&sessions])?;
                        while let Some(row) = rows.next()? {
                            emit(row)?;
                        }
                        Ok(())
                    })
                    .map_err(|e| e.to_string())?;
            }
            counts.insert(table.name, count);
        }
        Ok(counts)
    }

    /// Delete only waiting-send keys recorded in the prepared stream.
    pub(crate) fn delete_waiting_sends(&self, rows: &[Row]) -> Result<u64, String> {
        self.store
            .with_conn_fenced(|tx| {
                let mut n = 0;
                for (table, row) in rows.iter().filter(|(table, _)| table.file == File::Store) {
                    let predicate = table
                        .key
                        .iter()
                        .map(|col| format!("{col} = ?"))
                        .collect::<Vec<_>>()
                        .join(" AND ");
                    let mut stmt = tx.prepare_cached(&format!(
                        "DELETE FROM {} WHERE {predicate}",
                        table.sql_name
                    ))?;
                    n += stmt.execute(rusqlite::params_from_iter(
                        table.key.iter().map(|col| &row[table.position(col)]),
                    ))? as u64;
                }
                Ok(n)
            })
            .map_err(|e| e.to_string())
    }

    /// Insert missing payloads and follow the stream's call associations.
    /// Each file's share of this batch is committed in one transaction.
    /// Returns how many rows were inserted or reassociated.
    pub(crate) fn import(&self, rows: &[Row]) -> Result<u64, String> {
        let mut by_file: BTreeMap<File, Vec<&Row>> = BTreeMap::new();
        for row in rows {
            by_file.entry(row.0.file).or_default().push(row);
        }
        let mut inserted = 0;
        for (file, rows) in by_file {
            inserted += self
                .file(file)
                .with_conn_fenced(|tx| {
                    let mut n = 0;
                    for (table, values) in rows {
                        let mut insert = format!(
                            "INSERT OR IGNORE INTO {} ({}) VALUES ({})",
                            table.sql_name,
                            table.columns.join(", "),
                            (1..=table.columns.len())
                                .map(|i| format!("?{i}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        if matches!(table.name,"call_pins"|"notices") {
                            insert.push_str(&format!(" ON CONFLICT ({}) DO UPDATE SET session=excluded.session WHERE session!=excluded.session",table.key.join(",")));
                        }
                        let mut statement = tx.prepare_cached(&insert)?;
                        n += statement.execute(rusqlite::params_from_iter(values.iter()))? as u64;
                    }
                    Ok(n)
                })
                .map_err(|error| error.to_string())?;
        }
        Ok(inserted)
    }
}

#[cfg(test)]
pub(crate) mod tests;
