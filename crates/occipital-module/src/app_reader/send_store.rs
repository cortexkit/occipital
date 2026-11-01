//! Durable receipts for the apps' sends, so a repeat is answered and never sent twice.
//!
//! Ported from prefrontal-core's phone-message table (prefrontal `6b36afc14`,
//! `crates/prefrontal-core-store/src/phone_message.rs` and
//! `migrations/280_phone_message.sql`): one row per session and `clientMessageId`,
//! with the digest of the content it was accepted for, the receipt it was answered
//! with, and its age. Core also kept a host message id and attachment digests on the
//! row; the runner now keeps the app's id on the user entry itself, and attachments
//! arrive in a later change, so neither column is carried over.

use std::sync::Arc;

use cortexkit_store::{Migration, SqliteStore};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

/// The chain `--migration-chains` reports, so the deploy check sees this table.
pub(crate) const NAMESPACE: &str = "harness_app_send";
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    statements: "
CREATE TABLE harness_app_send (
    session TEXT NOT NULL,
    client_message_id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    mode TEXT NOT NULL,
    send_id TEXT NOT NULL,
    receipt_json TEXT NOT NULL,
    follow_until INTEGER,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (session, client_message_id)
);
CREATE INDEX harness_app_send_age ON harness_app_send(created_at);
CREATE INDEX harness_app_send_followed ON harness_app_send(follow_until) WHERE follow_until IS NOT NULL;",
}];

/// Kept separate so a build without run-bound steers can still open the store.
pub(crate) const RUN_NAMESPACE: &str = "harness_app_send_run";
pub(crate) const RUN_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    statements: "
CREATE TABLE harness_app_send_run (
    session TEXT NOT NULL,
    client_message_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    PRIMARY KEY (session, client_message_id)
);",
}];

/// The apps retry a send for up to 47 hours, so a receipt answers repeats for 48.
/// A repeat after that is treated as a new send; the runner's own `send_id`
/// check still keeps it from being delivered twice.
pub const RETENTION_MS: i64 = 48 * 60 * 60 * 1000;

/// Rows removed per prune statement, as core's retention pass removes them: a
/// bounded statement never holds the store's writer for long.
pub const PRUNE_CHUNK: usize = 128;

/// One stored send, as a repeat finds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub digest: String,
    pub mode: String,
    pub send_id: String,
    pub receipt: Value,
    /// Set while a worker steer is held by the runner and not yet delivered or
    /// expired: the time after which the runner must have settled it.
    pub follow_until: Option<i64>,
}

/// A stored worker steer the runner held and no follow-up has settled yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Followed {
    pub session: String,
    pub client_message_id: String,
    pub send_id: String,
    pub follow_until: i64,
    /// Absent for steers accepted by a build that only tracked expiry.
    pub run_id: Option<String>,
}

pub struct Receipts(Arc<SqliteStore>, Arc<super::send_queue::SendQueue>);

impl Receipts {
    /// Opens the receipts and, beside them in the same store, the list of sends
    /// not yet delivered.
    pub fn open(store: Arc<SqliteStore>) -> Result<Arc<Self>, crate::Error> {
        if store.migrate(NAMESPACE, MIGRATIONS)?.store_ahead() {
            return Err("harness_app_send_schema_ahead".into());
        }
        if store.migrate(RUN_NAMESPACE, RUN_MIGRATIONS)?.store_ahead() {
            return Err("harness_app_send_run_schema_ahead".into());
        }
        let queue = super::send_queue::SendQueue::open(store.clone())?;
        Ok(Arc::new(Self(store, queue)))
    }

    /// The sends accepted and not yet delivered, and those that ended unsent.
    pub fn queue(&self) -> &Arc<super::send_queue::SendQueue> {
        &self.1
    }

    /// The receipt for this message, unless it is older than [`RETENTION_MS`].
    pub fn get(&self, session: &str, id: &str, now: i64) -> Result<Option<Stored>, crate::Error> {
        let row = self.0.with_read(|conn| {
            conn.query_row(
                "SELECT content_digest,mode,send_id,receipt_json,follow_until FROM harness_app_send \
                 WHERE session=?1 AND client_message_id=?2 AND created_at>=?3",
                params![session, id, now.saturating_sub(RETENTION_MS)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()
        })?;
        row.map(|(digest, mode, send_id, receipt, follow_until)| {
            Ok(Stored {
                digest,
                mode,
                send_id,
                receipt: serde_json::from_str(&receipt)?,
                follow_until,
            })
        })
        .transpose()
    }

    /// Record the first receipt. An expired row for the same message is replaced:
    /// past retention the id names a new send. A held steer's deadline and target
    /// run are committed together so recovery never loses the run binding.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        session: &str,
        id: &str,
        digest: &str,
        mode: &str,
        send_id: &str,
        receipt: &Value,
        follow: Option<(i64, &str)>,
        now: i64,
    ) -> Result<(), crate::Error> {
        self.0.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let changed = tx.execute(
                "INSERT INTO harness_app_send(session,client_message_id,content_digest,mode,send_id,receipt_json,follow_until,created_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8) \
                 ON CONFLICT(session,client_message_id) DO UPDATE SET content_digest=excluded.content_digest,mode=excluded.mode,\
                 send_id=excluded.send_id,receipt_json=excluded.receipt_json,follow_until=excluded.follow_until,created_at=excluded.created_at \
                 WHERE harness_app_send.created_at < ?9",
                params![session, id, digest, mode, send_id, receipt.to_string(), follow.map(|(until, _)| until), now, now.saturating_sub(RETENTION_MS)],
            )?;
            if changed > 0 {
                tx.execute(
                    "DELETE FROM harness_app_send_run WHERE session=?1 AND client_message_id=?2",
                    params![session, id],
                )?;
                if let Some((_, run_id)) = follow {
                    tx.execute(
                        "INSERT INTO harness_app_send_run(session,client_message_id,run_id) VALUES(?1,?2,?3)",
                        params![session, id, run_id],
                    )?;
                }
            }
            tx.commit()
        })?;
        Ok(())
    }

    /// Settle a followed send: a changed final receipt replaces the first one, and
    /// the send is no longer followed either way.
    pub fn settle(
        &self,
        session: &str,
        id: &str,
        receipt: Option<&Value>,
    ) -> Result<(), crate::Error> {
        self.0.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE harness_app_send SET receipt_json=COALESCE(?3,receipt_json), follow_until=NULL WHERE session=?1 AND client_message_id=?2",
                params![session, id, receipt.map(Value::to_string)],
            )?;
            tx.execute(
                "DELETE FROM harness_app_send_run WHERE session=?1 AND client_message_id=?2",
                params![session, id],
            )?;
            tx.commit()
        })?;
        Ok(())
    }

    /// Up to `limit` sends still followed, the earliest deadline first.
    pub fn followed(&self, limit: usize) -> Result<Vec<Followed>, crate::Error> {
        Ok(self.0.with_read(|conn| {
            conn.prepare(
                "SELECT s.session,s.client_message_id,s.send_id,s.follow_until,r.run_id FROM harness_app_send s \
                 LEFT JOIN harness_app_send_run r ON s.session=r.session AND s.client_message_id=r.client_message_id \
                 WHERE s.follow_until IS NOT NULL ORDER BY s.follow_until LIMIT ?1",
            )?
            .query_map([limit as i64], |row| {
                Ok(Followed {
                    session: row.get(0)?,
                    client_message_id: row.get(1)?,
                    send_id: row.get(2)?,
                    follow_until: row.get(3)?,
                    run_id: row.get(4)?,
                })
            })?
            .collect()
        })?)
    }

    /// Remove at most [`PRUNE_CHUNK`] rows older than [`RETENTION_MS`], oldest first.
    pub fn prune(&self, now: i64) -> Result<usize, crate::Error> {
        Ok(self.0.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM harness_app_send_run WHERE (session,client_message_id) IN \
                 (SELECT session,client_message_id FROM harness_app_send WHERE created_at < ?1 ORDER BY created_at LIMIT ?2)",
                params![now.saturating_sub(RETENTION_MS), PRUNE_CHUNK as i64],
            )?;
            let removed = tx.execute(
                "DELETE FROM harness_app_send WHERE rowid IN (SELECT rowid FROM harness_app_send WHERE created_at < ?1 ORDER BY created_at LIMIT ?2)",
                params![now.saturating_sub(RETENTION_MS), PRUNE_CHUNK as i64],
            )?;
            tx.commit()?;
            Ok(removed)
        })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_bindings_follow_receipt_replacement_settlement_and_pruning() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::shared(&dir.path().join("store.db"));
        let receipts = Receipts::open(store.clone()).unwrap();
        let record = |id: &str, run: &str, now| {
            receipts
                .record(
                    "s",
                    id,
                    "digest",
                    "queue",
                    "send",
                    &json!({"ok": true}),
                    Some((now + 1_800_000, run)),
                    now,
                )
                .unwrap()
        };
        let bindings = || {
            store
                .with_read(|conn| {
                    conn.query_row("SELECT count(*) FROM harness_app_send_run", [], |row| {
                        row.get::<_, i64>(0)
                    })
                })
                .unwrap()
        };
        record("w1", "r1", 100);
        record("w1", "r2", 101);
        assert_eq!(
            receipts.followed(1).unwrap()[0].run_id.as_deref(),
            Some("r1")
        );
        record("w1", "r2", 101 + RETENTION_MS);
        assert_eq!(
            receipts.followed(1).unwrap()[0].run_id.as_deref(),
            Some("r2")
        );
        receipts.settle("s", "w1", None).unwrap();
        assert_eq!(bindings(), 0);
        record("w2", "r3", 100);
        assert_eq!(bindings(), 1);
        assert_eq!(receipts.prune(101 + RETENTION_MS).unwrap(), 1);
        assert_eq!(bindings(), 0);
    }

    #[test]
    fn legacy_followed_receipts_have_no_invented_run_binding() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::shared(&dir.path().join("store.db"));
        let receipts = Receipts::open(store.clone()).unwrap();
        receipts
            .record(
                "s",
                "w1",
                "d",
                "queue",
                "send",
                &json!({"ok": true}),
                None,
                100,
            )
            .unwrap();
        store
            .with_conn(|conn| {
                conn.execute(
                    "UPDATE harness_app_send SET follow_until=60100 WHERE session='s'",
                    [],
                )
            })
            .unwrap();
        let rows = receipts.followed(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].run_id, None);
        // The original chain is still version 1, even after the new namespace opens.
        assert!(!store.migrate(NAMESPACE, MIGRATIONS).unwrap().store_ahead());
    }
}
