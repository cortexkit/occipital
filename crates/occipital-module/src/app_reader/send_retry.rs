//! The apps' sends this module keeps and resends because the session's runner
//! couldn't take them: it was restarting for an update, or a call to it failed
//! with no answer. Each is resent under its own send id, in the order the app
//! sent them to that session, until the runner takes it or [`RESEND_LIMIT_MS`]
//! passes, when it is listed unsent with [`RUNNER_UNAVAILABLE`]. A runner
//! answers a repeat of a send id with the send it already has, so a resend
//! never delivers twice. The app's request is kept here, so a restart of this
//! module resumes it. While a send is kept, the apps' list shows its queue row
//! with `held`.
//!
//! It is its own namespace, so a build without it can still open the store
//! after a rollback. That build follows a kept send's queue row as any other,
//! and the runner, which never got it, ends it unsent.

use cortexkit_store::Migration;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

/// The chain `--migration-chains` reports, so the deploy check sees this table.
pub(crate) const NAMESPACE: &str = "harness_app_send_retry";
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    statements: "
CREATE TABLE harness_app_send_retry (
    session TEXT NOT NULL,
    client_message_id TEXT NOT NULL,
    device TEXT,
    params TEXT NOT NULL,
    kept_at INTEGER NOT NULL,
    give_up_at INTEGER NOT NULL,
    PRIMARY KEY (session, client_message_id)
);",
}];

/// How long a send is kept and resent, from when it was kept. A runner's
/// restart takes seconds; this bounds a runner that doesn't come back.
pub const RESEND_LIMIT_MS: i64 = 10 * 60 * 1000;

/// Why a kept send the runner never took within [`RESEND_LIMIT_MS`] is listed
/// as unsent.
pub const RUNNER_UNAVAILABLE: &str = "runner_unavailable";

/// One kept send: what the app asked, from which device (whose uploads its
/// files are), and when it was kept and is given up.
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    pub session: String,
    pub client_message_id: String,
    pub device: Option<String>,
    pub params: Value,
    pub kept_at: i64,
    pub give_up_at: i64,
}

pub(super) fn insert(tx: &rusqlite::Transaction<'_>, kept: &Kept) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO harness_app_send_retry(session,client_message_id,device,params,kept_at,give_up_at) \
         VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session,client_message_id) DO NOTHING",
        params![kept.session, kept.client_message_id, kept.device, kept.params.to_string(), kept.kept_at, kept.give_up_at],
    )?;
    Ok(())
}

pub(super) fn remove(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    id: &str,
) -> rusqlite::Result<usize> {
    tx.execute(
        "DELETE FROM harness_app_send_retry WHERE session=?1 AND client_message_id=?2",
        params![session, id],
    )
}

/// The session's first kept send, in the order the app sent them.
pub(super) fn first(conn: &rusqlite::Connection, session: &str) -> rusqlite::Result<Option<Kept>> {
    conn.query_row(
        "SELECT client_message_id,device,params,kept_at,give_up_at FROM harness_app_send_retry \
         WHERE session=?1 ORDER BY kept_at, client_message_id LIMIT 1",
        params![session],
        |row| {
            let params: String = row.get(2)?;
            Ok(Kept {
                session: session.to_owned(),
                client_message_id: row.get(0)?,
                device: row.get(1)?,
                params: serde_json::from_str(&params).unwrap_or(Value::Null),
                kept_at: row.get(3)?,
                give_up_at: row.get(4)?,
            })
        },
    )
    .optional()
}

/// Every session with a kept send.
pub(super) fn sessions(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT DISTINCT session FROM harness_app_send_retry ORDER BY session")?
        .query_map([], |row| row.get(0))?
        .collect()
}

/// Whether this message is kept.
pub(super) fn kept(conn: &rusqlite::Connection, session: &str, id: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM harness_app_send_retry WHERE session=?1 AND client_message_id=?2",
            params![session, id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
