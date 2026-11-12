//! The apps' sends that a runner accepted but whose message isn't stored yet, and
//! those that ended without being delivered, so the phone can show "waiting to reach
//! <agent>" or "didn't reach <agent>" instead of a delivered tick.
//!
//! A row is written when a send is accepted and not already delivered. A follower
//! asks the runner (`session.send_status`) until the send settles: delivered removes the row,
//! and any undelivered end keeps it as `unsent` with the runner's reason for
//! [`UNSENT_RETENTION_MS`]. A queued row is never pruned by age: the runner holds the
//! send, and the follower settles it however long that takes.
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
pub(crate) const WAIT_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    statements: "
CREATE TABLE harness_app_send_wait (
    session TEXT NOT NULL,
    client_message_id TEXT NOT NULL,
    runner_code TEXT NOT NULL,
    retry_after_ms INTEGER,
    PRIMARY KEY (session, client_message_id)
);",
}];

/// An unsent send stays listed this long after it ended, the same 48 hours a
/// repeat of the send is answered from its receipt.
pub const UNSENT_RETENTION_MS: i64 = 48 * 60 * 60 * 1000;

/// At most this many entries ride on one reply, oldest first; the reply says when
/// more exist. The phone shows each as its own pending bubble, and a watch notice must
/// fit its 192 KiB bound beside the page it carries, so the list stays small.
pub const LIST_CAP: usize = 20;

pub const QUEUED: &str = "queued";
pub const UNSENT: &str = "unsent";

/// A send the follower still watches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub session: String,
    pub client_message_id: String,
    pub harness: Option<String>,
    pub send_id: String,
    pub accepted_at: i64,
    /// A worker steer's expiry. Such a send is followed from its receipt, which
    /// carries the same deadline; only head sends are resumed from this table.
    pub steer_until: Option<i64>,
}

/// The run that would have carried a queued send ended in an error the runner
/// contract names, such as an account wait. The apps show it on the unsent entry
/// as `runnerCode`, and for an account wait `retryAfterMs`: how long after the
/// entry's `endedAtMs` the same words may go again, as a new send under a new
/// `clientMessageId` (the runner answers the old send id with its record forever).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedRun {
    pub runner_code: String,
    pub retry_after_ms: Option<i64>,
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

    /// Record an accepted, undelivered send as queued. A repeat of a send already
    /// recorded changes nothing.
    pub fn queue(&self, entry: &Queued) -> Result<(), crate::Error> {
        self.0.with_conn(|conn| {
            conn.execute(
                "INSERT INTO harness_app_send_queue(session,client_message_id,harness,send_id,accepted_at,steer_until,state) \
                 VALUES(?1,?2,?3,?4,?5,?6,'queued') ON CONFLICT(session,client_message_id) DO NOTHING",
                params![entry.session, entry.client_message_id, entry.harness, entry.send_id, entry.accepted_at, entry.steer_until],
            )
        })?;
        Ok(())
    }

    /// The send's message is stored: it leaves the list.
    pub fn delivered(&self, session: &str, client_message_id: &str) -> Result<bool, crate::Error> {
        Ok(self.0.with_conn(|conn| {
            conn.execute(
                "DELETE FROM harness_app_send_wait WHERE session=?1 AND client_message_id=?2",
                params![session, client_message_id],
            )?;
            conn.execute(
                "DELETE FROM harness_app_send_queue WHERE session=?1 AND client_message_id=?2",
                params![session, client_message_id],
            )
        })? > 0)
    }

    /// The send ended undelivered: it stays listed as unsent, with `reason`, and
    /// with the code its run ended with when there is one.
    pub fn unsent(
        &self,
        session: &str,
        client_message_id: &str,
        reason: &str,
        ended: Option<&EndedRun>,
        now: i64,
    ) -> Result<bool, crate::Error> {
        Ok(self.0.with_conn(|conn| {
            let changed = conn.execute(
                "UPDATE harness_app_send_queue SET state='unsent', reason=?3, ended_at=?4 \
                 WHERE session=?1 AND client_message_id=?2 AND state='queued'",
                params![session, client_message_id, reason, now],
            )?;
            if let (true, Some(ended)) = (changed > 0, ended) {
                conn.execute(
                    "INSERT OR REPLACE INTO harness_app_send_wait(session,client_message_id,runner_code,retry_after_ms) \
                     VALUES(?1,?2,?3,?4)",
                    params![session, client_message_id, ended.runner_code, ended.retry_after_ms],
                )?;
            }
            Ok(changed)
        })? > 0)
    }

    /// Up to `limit` queued head sends, the oldest first, for the follower to resume.
    /// A worker steer is resumed from its receipt instead.
    pub fn queued_heads(&self, limit: usize) -> Result<Vec<Queued>, crate::Error> {
        Ok(self.0.with_read(|conn| {
            conn.prepare(
                "SELECT session,client_message_id,harness,send_id,accepted_at FROM harness_app_send_queue \
                 WHERE state='queued' AND steer_until IS NULL ORDER BY accepted_at LIMIT ?1",
            )?
            .query_map([limit as i64], |row| {
                Ok(Queued {
                    session: row.get(0)?,
                    client_message_id: row.get(1)?,
                    harness: row.get(2)?,
                    send_id: row.get(3)?,
                    accepted_at: row.get(4)?,
                    steer_until: None,
                })
            })?
            .collect()
        })?)
    }

    /// The session's list as the apps read it: queued sends and those that ended
    /// unsent within [`UNSENT_RETENTION_MS`], oldest first, at most [`LIST_CAP`].
    /// `None` when there are none; the flag says more than the cap exist.
    pub fn list(&self, session: &str, now: i64) -> Result<Option<(Value, bool)>, crate::Error> {
        let rows =
            self.0.with_read(|conn| {
                conn.prepare(
                "SELECT q.client_message_id,q.send_id,q.accepted_at,q.state,q.reason,q.ended_at,\
                 w.runner_code,w.retry_after_ms FROM harness_app_send_queue q \
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

    /// Remove unsent rows whose listing window has passed, at most `chunk` at once.
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
        SendQueue::open(store).unwrap()
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
        };
        assert!(q.unsent("ses", "a", "error", Some(&wait), 40).unwrap());
        let final_code = EndedRun {
            runner_code: "account_selection_final".into(),
            retry_after_ms: None,
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
