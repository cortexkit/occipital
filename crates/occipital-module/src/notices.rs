//! The notices modules send an agent that ride its tool results, kept so the
//! apps can show each one under the call that carried it.
//!
//! A notice is a module's message to the agent, such as Magic Context's
//! reminders or core's status line. A runner delivers one either as its own
//! message, which makes a transcript entry the pages carry, or riding the next
//! tool result, which makes none (llm-runner's `notices` group). A riding
//! notice is learned two ways: a page read lists it in the `notices` display
//! fact of the message holding the call, and the runner's live stream sends
//! `notice_delivered` when it rides. It usually lands seconds after its call's
//! page was read, on a message the app already holds, and the page's version
//! doesn't move for it. So the apps read notices beside pages: `session.notices`
//! for the calls they show, and `session.notices_watch` to learn when one
//! arrives.
//!
//! Each notice is kept in `notices.db`, beside the store, by its runner, the
//! carrying call's key and its send, so one seen twice is kept once. Engram
//! captures the file, and a notice still shows after the runner prunes its own
//! record of it. A notice is kept as long as a tool display record.
//!
//! Page building and the live stream never wait on this: notices go to a task
//! through a bounded queue, and a batch that doesn't fit is dropped. Its
//! notices are kept the next time a page listing them is read.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use cortexkit_store::{SqliteStore, StorageDescriptor, StoreError};
use occipital_app_reader::source::ReadError;
use serde_json::{json, Map, Value};
use tokio::sync::{broadcast, mpsc};

use crate::display::store::now_ms;
use crate::store::OpenError;

/// The apps' read of the notices that rode some of a session's calls.
pub const READ: &str = "session.notices";
/// The apps' watch saying which calls got a notice.
pub const WATCH: &str = "session.notices_watch";

/// The file, beside the store.
pub const FILE: &str = "notices.db";
/// Its storage namespace, so its lease is its own.
const NAMESPACE: &str = "notices";

/// The most calls one `session.notices` asks about: a page's worth, as for
/// tool records.
pub const MAX_CALLS: usize = crate::display::app::MAX_CALLS;

/// Each riding notice, by runner, call key and send. `position` is the
/// notice's place among the call's notices on a page read, and null for one
/// seen only on the live stream until a page lists it.
const NOTICES: &str = "CREATE TABLE notices (
  runner TEXT NOT NULL, call_key TEXT NOT NULL, send_id TEXT NOT NULL,
  session TEXT NOT NULL, sender TEXT NOT NULL, text TEXT NOT NULL,
  delivered_as TEXT NOT NULL, position INTEGER, seen_at_ms INTEGER NOT NULL,
  PRIMARY KEY (runner, call_key, send_id)) WITHOUT ROWID;
CREATE INDEX notices_age ON notices (seen_at_ms);";

/// How long a notice is kept: as long as a tool display record.
const KEEP_MS: i64 = crate::display::store::Retention::DEFAULT.record_age_ms;

/// How often old notices are dropped. The first run is at startup.
const PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// How many batches wait to be kept before more are dropped.
const QUEUE: usize = 256;

/// How many notices are remembered as already queued, so a page read again
/// and again costs no store write. Past this the memory starts over.
const REMEMBERED: usize = 100_000;

/// How many arrivals wait for a slow watch before it falls behind. A watch
/// that falls behind ends, and the app reads again when it reopens.
const ARRIVALS: usize = 1024;

/// A riding notice, as page reads and the live stream report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The runner's module id.
    pub runner: String,
    pub session: String,
    /// The key of the call whose tool result carried it.
    pub call_key: String,
    /// The send that delivered it, which makes a notice seen twice the same.
    pub send_id: String,
    /// The module that sent it, as the daemon attested it.
    pub sender: String,
    pub text: String,
    /// How the runner delivered it, as the runner names it: `tool_result`.
    pub delivered_as: String,
    /// Its place among the call's notices on a page read, if seen there.
    pub position: Option<i64>,
}

/// The delivery mode of a notice that rode a tool result, which the live
/// event implies.
const TOOL_RESULT: &str = cortexkit_role_llm_runner::delivery_order::modes::TOOL_RESULT;

/// The riding notices a page read's entries list in their `notices` fact,
/// each placed among its call's notices in the order the entry lists them. A
/// notice missing a call key, send, sender or text names nothing an app could
/// show, and is skipped.
pub fn from_entries(entries: &[Value], runner: &str, session: &str) -> Vec<Notice> {
    let mut notices = Vec::new();
    for entry in entries {
        let mut placed: HashMap<String, i64> = HashMap::new();
        for fact in entry["notices"].as_array().into_iter().flatten() {
            let Some(mut notice) = notice_of(fact, runner, session) else {
                continue;
            };
            let position = placed.entry(notice.call_key.clone()).or_insert(0);
            notice.position = Some(*position);
            *position += 1;
            notices.push(notice);
        }
    }
    notices
}

/// The notice a live `notice_delivered` event carries. It has no place yet:
/// only a page read lists a call's notices in order.
pub fn from_event(event: &Value, runner: &str, session: &str) -> Option<Notice> {
    notice_of(event, runner, session)
}

fn notice_of(fact: &Value, runner: &str, session: &str) -> Option<Notice> {
    let text = |field: &str| {
        fact[field]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    Some(Notice {
        runner: runner.to_owned(),
        session: session.to_owned(),
        call_key: text("call_key")?,
        send_id: text("send_id")?,
        sender: text("sender")?,
        text: text("text")?,
        delivered_as: text("delivered_as").unwrap_or_else(|| TOOL_RESULT.to_owned()),
        position: None,
    })
}

/// A call's kept notice, as the apps are served it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub sender: String,
    pub text: String,
    pub delivered_as: String,
}

/// `notices.db`.
pub struct NoticeStore {
    file: SqliteStore,
}

impl NoticeStore {
    /// Open the file beside the store `main` names.
    pub fn open(main: &StorageDescriptor, data_root: &Path) -> Result<Self, OpenError> {
        let file =
            crate::display::store::open_file(main, data_root, NAMESPACE, FILE, &[&[NOTICES]])?;
        Ok(Self { file })
    }

    /// Keep each of `notices`, and return the ones not kept before. A notice
    /// kept again keeps its text; a place from a page read fills in one the
    /// live stream left out.
    pub fn keep(&self, notices: &[Notice], now_ms: i64) -> Result<Vec<Notice>, StoreError> {
        if notices.is_empty() {
            return Ok(Vec::new());
        }
        self.file.with_conn_fenced(|tx| {
            let mut insert = tx.prepare(
                "INSERT OR IGNORE INTO notices (runner, call_key, send_id, session, sender, text,
                 delivered_as, position, seen_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            let mut place = tx.prepare(
                "UPDATE notices SET position = ?4
                 WHERE runner = ?1 AND call_key = ?2 AND send_id = ?3 AND position IS NULL",
            )?;
            let mut fresh = Vec::new();
            for notice in notices {
                let inserted = insert.execute(rusqlite::params![
                    notice.runner,
                    notice.call_key,
                    notice.send_id,
                    notice.session,
                    notice.sender,
                    notice.text,
                    notice.delivered_as,
                    notice.position,
                    now_ms
                ])?;
                if inserted == 1 {
                    fresh.push(notice.clone());
                } else if let Some(position) = notice.position {
                    place.execute(rusqlite::params![
                        notice.runner,
                        notice.call_key,
                        notice.send_id,
                        position
                    ])?;
                }
            }
            Ok(fresh)
        })
    }

    /// The notices kept for a call of `runner`'s `session`: in the order a
    /// page listed them, then any seen only live, in the order they came.
    pub fn for_call(
        &self,
        runner: &str,
        session: &str,
        call_key: &str,
    ) -> Result<Vec<Kept>, StoreError> {
        self.file.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT sender, text, delivered_as FROM notices
                 WHERE runner = ?1 AND call_key = ?2 AND session = ?3
                 ORDER BY position IS NULL, position, seen_at_ms, send_id",
            )?;
            let kept = statement
                .query_map(rusqlite::params![runner, call_key, session], |row| {
                    Ok(Kept {
                        sender: row.get(0)?,
                        text: row.get(1)?,
                        delivered_as: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(kept)
        })
    }

    /// Drop the notices first seen before `cutoff_ms`. Returns how many went.
    pub fn prune(&self, cutoff_ms: i64) -> Result<usize, StoreError> {
        self.file.with_conn_fenced(|tx| {
            tx.execute("DELETE FROM notices WHERE seen_at_ms < ?1", [cutoff_ms])
        })
    }
}

/// A notice newly kept for a call of `runner`'s `session`, announced to the
/// notices watches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrived {
    pub runner: String,
    pub session: String,
    pub call_key: String,
}

/// Where page reads and the live stream report notices, once the file is open.
pub type NoticeSlot = Arc<OnceLock<Arc<Notices>>>;

/// Keeping and serving riding notices.
pub struct Notices {
    store: Arc<NoticeStore>,
    queue: mpsc::Sender<Vec<Notice>>,
    /// Each notice already queued, by runner, call key and send, with whether
    /// it had its place: a page read's place still has to reach a notice
    /// first seen on the live stream.
    queued: Mutex<HashMap<(String, String, String), bool>>,
    arrivals: broadcast::Sender<Arrived>,
}

impl Notices {
    /// Serve `store`, and start the task that keeps reported notices in it,
    /// announces each new one, and drops old ones.
    pub fn start(store: Arc<NoticeStore>) -> (Arc<Self>, tokio::task::JoinHandle<()>) {
        let (queue, mut batches) = mpsc::channel::<Vec<Notice>>(QUEUE);
        let arrivals = broadcast::channel(ARRIVALS).0;
        let notices = Arc::new(Self {
            store: store.clone(),
            queue,
            queued: Mutex::new(HashMap::new()),
            arrivals: arrivals.clone(),
        });
        let task = tokio::spawn(async move {
            let mut prune = tokio::time::interval(PRUNE_EVERY);
            loop {
                let batch = tokio::select! {
                    _ = prune.tick() => {
                        let store = store.clone();
                        let pruned = tokio::task::spawn_blocking(move || {
                            store.prune(now_ms().saturating_sub(KEEP_MS))
                        })
                        .await;
                        if let Ok(Err(error)) = pruned {
                            tracing::warn!(%error, "cannot drop old notices");
                        }
                        continue;
                    }
                    batch = batches.recv() => batch,
                };
                // Every reporter holds the sender through `Notices`, so this
                // ends only when the module does.
                let Some(first) = batch else { break };
                let mut notices = first;
                while let Ok(next) = batches.try_recv() {
                    notices.extend(next);
                }
                let store = store.clone();
                let kept = tokio::task::spawn_blocking(move || store.keep(&notices, now_ms()));
                match kept.await {
                    Ok(Ok(fresh)) => {
                        for notice in fresh {
                            // No watch open is no one to tell.
                            let _ = arrivals.send(Arrived {
                                runner: notice.runner,
                                session: notice.session,
                                call_key: notice.call_key,
                            });
                        }
                    }
                    Ok(Err(error)) => tracing::warn!(%error, "cannot keep notices"),
                    Err(error) => tracing::warn!(%error, "keeping notices stopped early"),
                }
            }
        });
        (notices, task)
    }

    /// Report notices a page read or the live stream carried. Notices already
    /// queued are skipped, unless this one brings the place the earlier one
    /// lacked. Never waits.
    pub fn seen(&self, notices: Vec<Notice>) {
        if notices.is_empty() {
            return;
        }
        let Ok(mut queued) = self.queued.lock() else {
            return;
        };
        let key = |notice: &Notice| {
            (
                notice.runner.clone(),
                notice.call_key.clone(),
                notice.send_id.clone(),
            )
        };
        let fresh: Vec<Notice> = notices
            .into_iter()
            .filter(|notice| match queued.get(&key(notice)) {
                Some(placed) => notice.position.is_some() && !placed,
                None => true,
            })
            .collect();
        if fresh.is_empty() {
            return;
        }
        if queued.len() + fresh.len() > REMEMBERED {
            queued.clear();
        }
        let keys: Vec<_> = fresh
            .iter()
            .map(|notice| (key(notice), notice.position.is_some()))
            .collect();
        // Remembered only once queued, so a dropped batch is reported again.
        if self.queue.try_send(fresh).is_ok() {
            queued.extend(keys);
        }
    }

    /// Every notice newly kept from now on.
    pub fn arrivals(&self) -> broadcast::Receiver<Arrived> {
        self.arrivals.subscribe()
    }

    /// `session.notices {callKeys}`: for each call of `runner`'s `session`
    /// that a notice rode, its notices as `{sender, text, deliveredAs}`, in
    /// order. A call no notice rode is left out.
    pub async fn read(
        &self,
        runner: &str,
        session: &str,
        params: &Value,
    ) -> Result<Value, ReadError> {
        let call_keys = crate::display::app::ids(params, "callKeys", MAX_CALLS)?;
        let (store, runner, session) = (self.store.clone(), runner.to_owned(), session.to_owned());
        let read = tokio::task::spawn_blocking(move || {
            let mut calls = Map::new();
            for call_key in call_keys {
                let kept = store
                    .for_call(&runner, &session, &call_key)
                    .map_err(unavailable)?;
                if kept.is_empty() {
                    continue;
                }
                let notices: Vec<Value> = kept
                    .into_iter()
                    .map(|kept| {
                        json!({
                            "sender": kept.sender,
                            "text": kept.text,
                            "deliveredAs": kept.delivered_as,
                        })
                    })
                    .collect();
                calls.insert(call_key, Value::Array(notices));
            }
            Ok(json!({ "calls": calls }))
        });
        read.await
            .map_err(|error| ReadError::new("internal", format!("notices read stopped: {error}")))?
    }
}

fn unavailable(error: impl std::fmt::Display) -> ReadError {
    ReadError::new(
        "provider_unavailable",
        format!("notices store unavailable: {error}"),
    )
    .with_detail(json!({ "retryable": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_test_support::ScratchDir;

    fn opened(dir: &ScratchDir) -> (crate::store::Store, NoticeStore) {
        let descriptor = crate::store::tests::descriptor(dir);
        let main = crate::store::open(&descriptor).unwrap();
        let notices = NoticeStore::open(&descriptor, main.data_root()).unwrap();
        (main, notices)
    }

    fn notice(send_id: &str, text: &str, position: Option<i64>) -> Notice {
        Notice {
            runner: "broca".into(),
            session: "s".into(),
            call_key: "ck".into(),
            send_id: send_id.into(),
            sender: "magic-context".into(),
            text: text.into(),
            delivered_as: TOOL_RESULT.into(),
            position,
        }
    }

    fn texts(store: &NoticeStore) -> Vec<String> {
        store
            .for_call("broca", "s", "ck")
            .unwrap()
            .into_iter()
            .map(|kept| kept.text)
            .collect()
    }

    /// A notice is kept once per send: seen again it isn't new and keeps its
    /// text. A call's notices come in the order a page listed them, and one
    /// first seen on the live stream takes its place once a page lists it.
    /// Only the call's own session sees them, and they age out.
    #[test]
    fn notices_are_kept_once_and_served_in_page_order() {
        let dir = ScratchDir::new("occipital-notices");
        let (_main, store) = opened(&dir);
        // The first notice is seen only live, then a page lists a later one.
        let live = notice("send-1", "first", None);
        assert_eq!(
            store.keep(std::slice::from_ref(&live), 100).unwrap(),
            [live]
        );
        let later = notice("send-2", "second", Some(1));
        assert_eq!(
            store.keep(std::slice::from_ref(&later), 200).unwrap(),
            [later]
        );
        assert_eq!(texts(&store), ["second", "first"]);
        // A page listing both puts the live one in its place, and nothing is new.
        let page = [
            notice("send-1", "changed", Some(0)),
            notice("send-2", "second", Some(1)),
        ];
        assert!(store.keep(&page, 300).unwrap().is_empty());
        assert_eq!(texts(&store), ["first", "second"]);
        assert!(store.for_call("broca", "other", "ck").unwrap().is_empty());
        assert_eq!(store.prune(150).unwrap(), 1);
        assert_eq!(texts(&store), ["second"]);
    }

    /// A page's facts are placed per call in the order each entry lists them,
    /// a live event carries none, and a fact missing what an app would show
    /// is skipped.
    #[test]
    fn facts_and_events_become_notices() {
        let entries = [json!({"mid": "m1", "notices": [
            {"part": 0, "call_key": "ck_a", "send_id": "s1", "sender": "magic-context",
             "text": "one", "delivered_as": "tool_result"},
            {"part": 1, "call_key": "ck_b", "send_id": "s2", "sender": "prefrontal-core",
             "text": "two", "delivered_as": "tool_result"},
            {"part": 1, "call_key": "ck_b", "send_id": "s3", "sender": "magic-context",
             "text": "three", "delivered_as": "tool_result"},
            {"part": 1, "call_key": "ck_b", "send_id": "s4", "sender": "magic-context",
             "text": ""}
        ]})];
        let notices = from_entries(&entries, "broca", "s");
        let placed: Vec<_> = notices
            .iter()
            .map(|notice| {
                (
                    notice.call_key.as_str(),
                    notice.text.as_str(),
                    notice.position,
                )
            })
            .collect();
        assert_eq!(
            placed,
            [
                ("ck_a", "one", Some(0)),
                ("ck_b", "two", Some(0)),
                ("ck_b", "three", Some(1))
            ]
        );
        let event = json!({"type": "notice_delivered", "mid": "m1", "part": 2,
            "call_key": "ck_c", "send_id": "s5", "sender": "magic-context", "text": "late"});
        let live = from_event(&event, "broca", "s").unwrap();
        assert_eq!(
            (live.delivered_as.as_str(), live.position),
            (TOOL_RESULT, None)
        );
    }
}
