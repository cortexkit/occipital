//! The apps' sends that a runner accepted but whose message isn't stored yet, those
//! that ended without being delivered, and those the runner can't confirm, so the
//! apps can show "waiting to reach <agent>", "didn't reach <agent>" or "may not
//! have arrived" instead of a delivered tick.
//!
//! A row is written when a send is accepted and not already delivered. A follower
//! asks the runner (`session.send_status`) until the send settles: delivered removes the row,
//! any undelivered end keeps it as `unsent` with the runner's reason, and a send the
//! runner handed on but can't confirm (`delivered.as: unknown`) keeps it as
//! `unknown`, each for [`UNSENT_RETENTION_MS`]. A queued row is never pruned by age:
//! the runner holds the send, and the follower settles it however long that takes.
//!
//! It is its own namespace, not a change to `harness_app_send`, so a build without
//! it can still open the store after a rollback. For the same reason the code a
//! queued send's run ended with lives in a namespace of its own,
//! `harness_app_send_wait`, beside the row it explains.

use std::sync::Arc;

use cortexkit_store::{Migration, SqliteStore};
use rusqlite::params;
use serde_json::{json, Value};

/// The chain `--migration-chains` reports, so the deploy check sees this table.
pub(crate) const NAMESPACE: &str = "harness_app_send_queue";
pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    statements: "
CREATE TABLE harness_app_send_queue (
    session TEXT NOT NULL,
    client_message_id TEXT NOT NULL,
    harness TEXT,
    send_id TEXT NOT NULL,
    accepted_at INTEGER NOT NULL,
    steer_until INTEGER,
    state TEXT NOT NULL,
    reason TEXT,
    ended_at INTEGER,
    PRIMARY KEY (session, client_message_id)
);
CREATE INDEX harness_app_send_queue_ended ON harness_app_send_queue(ended_at) WHERE ended_at IS NOT NULL;",
}];

/// The chain for the code an unsent send's run ended with ([`EndedRun`]).
pub(crate) const WAIT_NAMESPACE: &str = "harness_app_send_wait";
pub(crate) const WAIT_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        statements: "
CREATE TABLE harness_app_send_wait (
    session TEXT NOT NULL,
    client_message_id TEXT NOT NULL,
    runner_code TEXT NOT NULL,
    retry_after_ms INTEGER,
    PRIMARY KEY (session, client_message_id)
);",
    },
    // The named cause of a host's refusal (runner contract 0.8.58).
    Migration {
        version: 2,
        statements: "ALTER TABLE harness_app_send_wait ADD COLUMN cause TEXT;",
    },
];

/// An unsent send stays listed this long after it ended, the same 48 hours a
/// repeat of the send is answered from its receipt.
pub const UNSENT_RETENTION_MS: i64 = 48 * 60 * 60 * 1000;

/// At most this many entries ride on one reply, oldest first; the reply says when
/// more exist. The phone shows each as its own pending bubble, and a watch notice must
/// fit its 192 KiB bound beside the page it carries, so the list stays small.
pub const LIST_CAP: usize = 20;

pub const QUEUED: &str = "queued";
pub const UNSENT: &str = "unsent";
/// The runner can't confirm the send ran, and never sends it again; sending the
/// words again could run them twice, so the apps offer no resend.
pub const UNKNOWN: &str = "unknown";

/// A send the follower still watches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub session: String,
    pub client_message_id: String,
    pub harness: Option<String>,
    pub send_id: String,
    pub accepted_at: i64,
    /// A worker steer's expiry, kept with the queue row when the session moves.
    pub steer_until: Option<i64>,
}

/// The run that would have carried a queued send ended in an error the runner
/// contract names, such as an account wait. The apps show it on the unsent entry
/// as `runnerCode`, and for an account wait `retryAfterMs`: how long after the
/// entry's `endedAtMs` the same words may go again, as a new send under a new
/// `clientMessageId` (the runner answers the old send id with its record forever).
/// When the host refused the send for good (`host_refused`), `cause` names why,
/// so the apps can say what to change before sending the words again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedRun {
    pub runner_code: String,
    pub retry_after_ms: Option<i64>,
    pub cause: Option<String>,
}

/// How a followed send ended, for its entry on the apps' list.
pub(super) enum End {
    /// Its message is in a step or a turn: the entry leaves the list.
    Delivered,
    /// The runner ended it undelivered: listed `unsent` with this reason.
    Unsent(String),
    /// It may or may not have run: listed `unknown`, so the apps can say it may
    /// not have arrived. Sending the words again could run them twice, so only
    /// the person decides that.
    Unconfirmed,
}

pub struct SendQueue(Arc<SqliteStore>);

impl SendQueue {
    pub fn open(store: Arc<SqliteStore>) -> Result<Arc<Self>, crate::Error> {
        if store.migrate(NAMESPACE, MIGRATIONS)?.store_ahead() {
            return Err("harness_app_send_queue_schema_ahead".into());
        }
        if store
            .migrate(WAIT_NAMESPACE, WAIT_MIGRATIONS)?
            .store_ahead()
        {
            return Err("harness_app_send_wait_schema_ahead".into());
        }
        Ok(Arc::new(Self(store)))
    }

    pub(super) fn insert(
        tx: &rusqlite::Transaction<'_>,
        entry: &Queued,
        unknown: bool,
    ) -> rusqlite::Result<()> {
        tx.execute(
            "INSERT INTO harness_app_send_queue(session,client_message_id,harness,send_id,accepted_at,steer_until,state,ended_at) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(session,client_message_id) DO NOTHING",
            params![entry.session, entry.client_message_id, entry.harness, entry.send_id, entry.accepted_at, entry.steer_until, if unknown { UNKNOWN } else { QUEUED }, unknown.then_some(entry.accepted_at)],
        )?;
        Ok(())
    }

    pub(super) fn update(
        tx: &rusqlite::Transaction<'_>,
        session: &str,
        id: &str,
        end: &End,
        ended: Option<&EndedRun>,
        now: i64,
    ) -> rusqlite::Result<bool> {
        let changed = match end {
            End::Delivered => {
                tx.execute("DELETE FROM harness_app_send_wait WHERE session=?1 AND client_message_id=?2", params![session,id])?;
                tx.execute("DELETE FROM harness_app_send_queue WHERE session=?1 AND client_message_id=?2", params![session,id])?
            }
            End::Unsent(reason) => {
                let changed = tx.execute("UPDATE harness_app_send_queue SET state='unsent', reason=?3, ended_at=?4 WHERE session=?1 AND client_message_id=?2 AND state='queued'", params![session,id,reason,now])?;
                if let (true, Some(ended)) = (changed > 0, ended) {
                    tx.execute("INSERT OR REPLACE INTO harness_app_send_wait(session,client_message_id,runner_code,retry_after_ms,cause) VALUES(?1,?2,?3,?4,?5)", params![session,id,ended.runner_code,ended.retry_after_ms,ended.cause])?;
                }
                changed
            }
            End::Unconfirmed => tx.execute("UPDATE harness_app_send_queue SET state=?3, ended_at=?4 WHERE session=?1 AND client_message_id=?2 AND state='queued'", params![session,id,UNKNOWN,now])?,
        };
        Ok(changed > 0)
    }

    #[cfg(test)]
    pub fn queue(&self, entry: &Queued) -> Result<(), crate::Error> {
        Ok(self.0.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            Self::insert(&tx, entry, false)?;
            tx.commit()
        })?)
    }

    #[cfg(test)]
    fn update_alone(
        &self,
        session: &str,
        id: &str,
        end: End,
        ended: Option<&EndedRun>,
        now: i64,
    ) -> Result<bool, crate::Error> {
        Ok(self.0.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let changed = Self::update(&tx, session, id, &end, ended, now)?;
            tx.commit()?;
            Ok(changed)
        })?)
    }

    #[cfg(test)]
    pub fn delivered(&self, session: &str, id: &str) -> Result<bool, crate::Error> {
        self.update_alone(session, id, End::Delivered, None, 0)
    }
    #[cfg(test)]
    pub fn unsent(
        &self,
        session: &str,
        id: &str,
        reason: &str,
        ended: Option<&EndedRun>,
        now: i64,
    ) -> Result<bool, crate::Error> {
        self.update_alone(session, id, End::Unsent(reason.into()), ended, now)
    }
    #[cfg(test)]
    pub fn unconfirmed(&self, session: &str, id: &str, now: i64) -> Result<bool, crate::Error> {
        self.update_alone(session, id, End::Unconfirmed, None, now)
    }

    #[cfg(test)]
    pub fn queued_heads(&self, limit: usize) -> Result<Vec<Queued>, crate::Error> {
        self.queued_after(limit, &Default::default())
    }

    /// Queued sends after `after`, in key order. A worker send that came here
    /// when its agent moved from another computer has only its queue row, since
    /// a move carries no receipts; a worker send made here is followed through the
    /// run its receipt names, even before its recovery batch is read.
    pub(super) fn queued_after(
        &self,
        limit: usize,
        after: &(String, String),
    ) -> Result<Vec<Queued>, crate::Error> {
        Ok(self.0.with_read(|conn| {
            conn.prepare(
                "SELECT session,client_message_id,harness,send_id,accepted_at,steer_until FROM harness_app_send_queue q \
                 WHERE state='queued' AND (session,client_message_id)>(?2,?3) AND (steer_until IS NULL OR NOT EXISTS \
                 (SELECT 1 FROM harness_app_send s WHERE s.session=q.session AND s.client_message_id=q.client_message_id AND s.follow_until IS NOT NULL)) \
                 ORDER BY session,client_message_id LIMIT ?1",
            )?
            .query_map(params![limit as i64, after.0, after.1], |row| {
                Ok(Queued {
                    session: row.get(0)?,
                    client_message_id: row.get(1)?,
                    harness: row.get(2)?,
                    send_id: row.get(3)?,
                    accepted_at: row.get(4)?,
                    steer_until: row.get(5)?,
                })
            })?
            .collect()
        })?)
    }

    pub(super) fn pending_id(
        &self,
        session: &str,
        id: &str,
    ) -> Result<Option<String>, crate::Error> {
        use rusqlite::OptionalExtension;
        Ok(self.0.with_read(|conn| conn.query_row("SELECT send_id FROM harness_app_send_queue WHERE session=?1 AND client_message_id=?2 AND state='queued'", params![session,id], |r| r.get(0)).optional())?)
    }

    /// The session's list as the apps read it: queued sends and those that ended
    /// unsent or unknown within [`UNSENT_RETENTION_MS`], oldest first, at most
    /// [`LIST_CAP`].
    /// `None` when there are none; the flag says more than the cap exist.
    pub fn list(&self, session: &str, now: i64) -> Result<Option<(Value, bool)>, crate::Error> {
        let rows =
            self.0.with_read(|conn| {
                conn.prepare(
                "SELECT q.client_message_id,q.send_id,q.accepted_at,q.state,q.reason,q.ended_at,\
                 w.runner_code,w.retry_after_ms,w.cause FROM harness_app_send_queue q \
                 LEFT JOIN harness_app_send_wait w \
                 ON w.session=q.session AND w.client_message_id=q.client_message_id \
                 WHERE q.session=?1 AND (q.state='queued' OR q.ended_at>=?2) \
                 ORDER BY q.accepted_at, q.client_message_id LIMIT ?3",
            )?
            .query_map(
                params![session, now.saturating_sub(UNSENT_RETENTION_MS), LIST_CAP as i64 + 1],
                |row| {
                    let mut entry = json!({
                        "clientMessageId": row.get::<_, String>(0)?,
                        "effectId": row.get::<_, String>(1)?,
                        "acceptedAtMs": row.get::<_, i64>(2)?,
                        "state": row.get::<_, String>(3)?,
                    });
                    if let Some(reason) = row.get::<_, Option<String>>(4)? {
                        entry["reason"] = json!(reason);
                    }
                    if let Some(ended) = row.get::<_, Option<i64>>(5)? {
                        entry["endedAtMs"] = json!(ended);
                    }
                    if let Some(code) = row.get::<_, Option<String>>(6)? {
                        entry["runnerCode"] = json!(code);
                    }
                    if let Some(after) = row.get::<_, Option<i64>>(7)? {
                        entry["retryAfterMs"] = json!(after);
                    }
                    if let Some(cause) = row.get::<_, Option<String>>(8)? {
                        entry["cause"] = json!(cause);
                    }
                    Ok(entry)
                },
            )?
            .collect::<Result<Vec<_>, _>>()
            })?;
        if rows.is_empty() {
            return Ok(None);
        }
        let truncated = rows.len() > LIST_CAP;
        let shown: Vec<Value> = rows.into_iter().take(LIST_CAP).collect();
        Ok(Some((Value::Array(shown), truncated)))
    }

    /// Remove unsent and unknown rows whose listing window has passed, at most
    /// `chunk` at once.
    pub fn prune(&self, now: i64, chunk: usize) -> Result<usize, crate::Error> {
        Ok(self.0.with_conn(|conn| {
            let pruned = conn.execute(
                "DELETE FROM harness_app_send_queue WHERE rowid IN (SELECT rowid FROM harness_app_send_queue \
                 WHERE ended_at < ?1 ORDER BY ended_at LIMIT ?2)",
                params![now.saturating_sub(UNSENT_RETENTION_MS), chunk as i64],
            )?;
            // A wait row lives only as long as the entry it explains.
            conn.execute(
                "DELETE FROM harness_app_send_wait WHERE NOT EXISTS (SELECT 1 FROM harness_app_send_queue q \
                 WHERE q.session=harness_app_send_wait.session \
                 AND q.client_message_id=harness_app_send_wait.client_message_id)",
                [],
            )?;
            Ok(pruned)
        })?)
    }
}

/// Add the session's list to an app reply: `queuedSends`, and
/// `queuedSendsTruncated: true` when more than [`LIST_CAP`] exist. Absent when empty.
pub fn attach(reply: &mut Value, list: Option<(Value, bool)>) {
    let Some(object) = reply.as_object_mut() else {
        return;
    };
    object.remove("queuedSends");
    object.remove("queuedSendsTruncated");
    if let Some((entries, truncated)) = list {
        object.insert("queuedSends".into(), entries);
        if truncated {
            object.insert("queuedSendsTruncated".into(), json!(true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> Arc<SendQueue> {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::shared(&dir.path().join("store.db"));
        // The store outlives the test's directory handle.
        std::mem::forget(dir);
        super::super::send_store::Receipts::open(store)
            .unwrap()
            .queue()
            .clone()
    }

    fn entry(id: &str, at: i64) -> Queued {
        Queued {
            session: "ses".into(),
            client_message_id: id.into(),
            harness: Some("opencode2".into()),
            send_id: format!("send-{id}"),
            accepted_at: at,
            steer_until: None,
        }
    }

    #[test]
    fn a_send_is_listed_until_delivered_and_an_unsent_one_for_48_hours() {
        let q = queue();
        assert_eq!(q.list("ses", 0).unwrap(), None);
        q.queue(&entry("a", 10)).unwrap();
        q.queue(&entry("b", 20)).unwrap();
        // A repeat changes nothing.
        q.queue(&entry("a", 99)).unwrap();
        let (list, truncated) = q.list("ses", 30).unwrap().unwrap();
        assert!(!truncated);
        assert_eq!(
            list,
            json!([
                {"clientMessageId":"a","effectId":"send-a","acceptedAtMs":10,"state":"queued"},
                {"clientMessageId":"b","effectId":"send-b","acceptedAtMs":20,"state":"queued"},
            ])
        );
        assert!(q.delivered("ses", "a").unwrap());
        assert!(q.unsent("ses", "b", "superseded", None, 40).unwrap());
        // An unsent send can't end twice.
        assert!(!q.unsent("ses", "b", "not_sent", None, 50).unwrap());
        assert_eq!(
            q.list("ses", 41).unwrap().unwrap().0,
            json!([{"clientMessageId":"b","effectId":"send-b","acceptedAtMs":20,
                "state":"unsent","reason":"superseded","endedAtMs":40}])
        );
        assert!(q.queued_heads(10).unwrap().is_empty());
        // Past its window it is no longer listed, then pruned.
        let later = 40 + UNSENT_RETENTION_MS + 1;
        assert_eq!(q.list("ses", later).unwrap(), None);
        assert_eq!(q.prune(later, 128).unwrap(), 1);
    }

    /// An unsent send whose run ended on a contract code lists it, with the wait
    /// for an account wait, and the wait row goes with its entry.
    #[test]
    fn an_unsent_send_lists_the_code_its_run_ended_with() {
        let q = queue();
        q.queue(&entry("a", 10)).unwrap();
        q.queue(&entry("b", 20)).unwrap();
        let wait = EndedRun {
            runner_code: "account_limit_exhausted".into(),
            retry_after_ms: Some(120_000),
            cause: None,
        };
        assert!(q.unsent("ses", "a", "error", Some(&wait), 40).unwrap());
        let final_code = EndedRun {
            runner_code: "account_selection_final".into(),
            retry_after_ms: None,
            cause: None,
        };
        assert!(q
            .unsent("ses", "b", "error", Some(&final_code), 50)
            .unwrap());
        assert_eq!(
            q.list("ses", 60).unwrap().unwrap().0,
            json!([
                {"clientMessageId":"a","effectId":"send-a","acceptedAtMs":10,"state":"unsent",
                    "reason":"error","endedAtMs":40,"runnerCode":"account_limit_exhausted",
                    "retryAfterMs":120000},
                {"clientMessageId":"b","effectId":"send-b","acceptedAtMs":20,"state":"unsent",
                    "reason":"error","endedAtMs":50,"runnerCode":"account_selection_final"},
            ])
        );
        let waits = |q: &SendQueue| {
            q.0.with_read(|conn| {
                conn.query_row("SELECT count(*) FROM harness_app_send_wait", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .unwrap()
        };
        assert_eq!(waits(&q), 2);
        assert_eq!(q.prune(40 + UNSENT_RETENTION_MS + 1, 128).unwrap(), 1);
        assert_eq!(waits(&q), 1);
        // A later send under the same id starts clean.
        assert!(q.delivered("ses", "b").unwrap());
        assert_eq!(waits(&q), 0);
    }

    #[test]
    fn the_list_is_capped_oldest_first_and_says_so() {
        let q = queue();
        for n in 0..=LIST_CAP as i64 {
            q.queue(&entry(&format!("{n:02}"), n)).unwrap();
        }
        let (list, truncated) = q.list("ses", 0).unwrap().unwrap();
        assert!(truncated);
        assert_eq!(list.as_array().unwrap().len(), LIST_CAP);
        assert_eq!(list[0]["clientMessageId"], "00");
        let mut reply = json!({"messages": []});
        attach(&mut reply, Some((list, truncated)));
        assert_eq!(reply["queuedSendsTruncated"], true);
        attach(&mut reply, None);
        assert!(reply.get("queuedSends").is_none() && reply.get("queuedSendsTruncated").is_none());
    }

    /// The published sample set's synthetic `unsent` file is exactly what this list
    /// produces for the same three sends, so the phone pins the module's own shape.
    #[test]
    fn the_published_unsent_sample_is_what_the_list_produces() {
        let q = queue();
        let send = |id: &str, effect: &str, at: i64| Queued {
            session: "ses".into(),
            client_message_id: id.into(),
            harness: Some("opencode2".into()),
            send_id: effect.into(),
            accepted_at: at,
            steer_until: None,
        };
        let (a, b, c) = (
            "00000000-0000-4000-8000-00000000000a",
            "00000000-0000-4000-8000-00000000000b",
            "00000000-0000-4000-8000-00000000000c",
        );
        q.queue(&send(a, "app-synthetic-a", 1_792_522_200_000))
            .unwrap();
        q.queue(&send(b, "app-synthetic-b", 1_793_213_401_000))
            .unwrap();
        q.queue(&send(c, "app-synthetic-c", 1_793_213_402_000))
            .unwrap();
        assert!(q
            .unsent("ses", a, "hold_expired", None, 1_793_213_400_000)
            .unwrap());
        assert!(q
            .unsent("ses", b, "not_sent", None, 1_793_213_460_000)
            .unwrap());
        let mut notice = json!({});
        attach(&mut notice, q.list("ses", 1_793_213_500_000).unwrap());
        let published: Value = serde_json::from_slice(
            &std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../occipital-app-reader/test-vectors/phone-reader-queued-sends-v1/10-unsent-entries.synthetic.json"
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(notice, published);
    }
}
