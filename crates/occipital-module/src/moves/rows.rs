//! The rows a move carries for a set of sessions, copied whole from the four
//! files they live in.
//!
//! Every table here is keyed so a row means the same on any computer: by
//! session name, by the runner that minted a call and its call key, or by a
//! sender and its send id. So a row is exported as its column values, and
//! imported with `INSERT OR IGNORE`: a row the receiver already holds is the
//! same row (records and notices are immutable), and is kept.
//!
//! Tool records and details carry no session; they belong to the sessions
//! whose calls they answer, found through the call pins.

use std::collections::{BTreeMap, BTreeSet, HashSet};

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

    /// The calls the sessions' pins name, as `(runner principal JSON, call
    /// key)`, the form the tool record tables key them by.
    fn calls(&self, sessions: &BTreeSet<String>) -> Result<HashSet<(String, String)>, String> {
        let pins: Vec<(String, String, String)> = self
            .records
            .with_conn(|conn| {
                let mut statement =
                    conn.prepare("SELECT runner, call_key, session FROM display_call_pins")?;
                let rows = statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                Ok(rows)
            })
            .map_err(|error| error.to_string())?;
        let mut calls = HashSet::new();
        for (runner, call_key, session) in pins {
            if !sessions.contains(&session) {
                continue;
            }
            let principal = Principal::Reserved { module_id: runner };
            let runner = crate::display::store::principal_json(&principal)
                .map_err(|error| error.to_string())?;
            calls.insert((runner, call_key));
        }
        Ok(calls)
    }

    /// Each row of every table that belongs to `sessions`, handed to `each`
    /// table by table, in [`TABLES`] order. Returns how many rows each table
    /// gave.
    pub(crate) fn export(
        &self,
        sessions: &BTreeSet<String>,
        mut each: impl FnMut(Row) -> Result<(), String>,
    ) -> Result<BTreeMap<&'static str, u64>, String> {
        let calls = self.calls(sessions)?;
        let mut counts = BTreeMap::new();
        for table in TABLES {
            let rows = self.matching(table, sessions, &calls)?;
            counts.insert(table.name, rows.len() as u64);
            for row in rows {
                each((table, row))?;
            }
        }
        Ok(counts)
    }

    /// Delete the queued and unsent sends of `sessions`, once a move has
    /// carried them to the computer that follows them now: a send has one
    /// follower, or it would be asked after on two computers. Display rows are
    /// kept: they are immutable, so a copy here costs nothing, and a record a
    /// provider writes here after the cut is kept for a move back.
    pub(crate) fn delete_waiting_sends(&self, sessions: &BTreeSet<String>) -> Result<u64, String> {
        let calls = HashSet::new();
        let mut deleted = 0;
        for table in TABLES.iter().filter(|table| table.file == File::Store) {
            let rows = self.matching(table, sessions, &calls)?;
            if rows.is_empty() {
                continue;
            }
            let delete = format!(
                "DELETE FROM {} WHERE {}",
                table.sql_name,
                table
                    .key
                    .iter()
                    .enumerate()
                    .map(|(i, column)| format!("{column} = ?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(" AND ")
            );
            let positions: Vec<usize> = table.key.iter().map(|c| table.position(c)).collect();
            deleted += self
                .file(table.file)
                .with_conn_fenced(|tx| {
                    let mut statement = tx.prepare(&delete)?;
                    let mut n = 0;
                    for row in &rows {
                        let key = positions.iter().map(|&i| &row[i]);
                        n += statement.execute(rusqlite::params_from_iter(key))? as u64;
                    }
                    Ok(n)
                })
                .map_err(|error| error.to_string())?;
        }
        Ok(deleted)
    }

    /// Insert `rows`, keeping every row already held. Each file's rows go in
    /// one transaction, so a failed import leaves that file as it was.
    /// Returns how many rows were new.
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
                        let insert = format!(
                            "INSERT OR IGNORE INTO {} ({}) VALUES ({})",
                            table.sql_name,
                            table.columns.join(", "),
                            (1..=table.columns.len())
                                .map(|i| format!("?{i}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        let mut statement = tx.prepare_cached(&insert)?;
                        n += statement.execute(rusqlite::params_from_iter(values.iter()))? as u64;
                    }
                    Ok(n)
                })
                .map_err(|error| error.to_string())?;
        }
        Ok(inserted)
    }

    /// `table`'s rows that belong to `sessions`, whole. Tables here are read
    /// once each, rather than once per session, since a head can have thousands
    /// of sessions and most tables aren't indexed by session.
    fn matching(
        &self,
        table: &Table,
        sessions: &BTreeSet<String>,
        calls: &HashSet<(String, String)>,
    ) -> Result<Vec<Vec<Sql>>, String> {
        let select = format!(
            "SELECT {} FROM {}",
            table.columns.join(", "),
            table.sql_name
        );
        // The columns that decide whether a row belongs: the session, or the
        // call's runner and key.
        let (first, second) = match table.belongs {
            Belongs::Session => (table.position("session"), 0),
            Belongs::Call => (table.position("runner"), table.position("call_key")),
        };
        self.file(table.file)
            .with_conn(|conn| {
                let mut statement = conn.prepare(&select)?;
                let mut rows = statement.query([])?;
                let mut kept = Vec::new();
                while let Some(row) = rows.next()? {
                    let belongs = match table.belongs {
                        Belongs::Session => sessions.contains(&row.get::<_, String>(first)?),
                        Belongs::Call => calls.contains(&(
                            row.get::<_, String>(first)?,
                            row.get::<_, String>(second)?,
                        )),
                    };
                    if belongs {
                        kept.push(
                            (0..table.columns.len())
                                .map(|i| row.get::<_, Sql>(i))
                                .collect::<rusqlite::Result<_>>()?,
                        );
                    }
                }
                Ok(kept)
            })
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
pub(crate) mod tests;
