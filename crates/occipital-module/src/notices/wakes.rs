//! Wake records (`wake-record-display/v1`, tool-display's §8): what a module
//! that woke an agent says the wake held, such as the peer messages and room
//! posts in core's inbox bundle, so the apps draw it as chips instead of the
//! prompt text the model received.
//!
//! The waking module writes the record before it sends the wake, through
//! `display.write` with `kind: "wake"`, keyed by its own send id. The sender is
//! the route's attested writer, never a member of the write. A record is kept
//! once per (session, sender, send id) in `notices.db`, never changed, for as
//! long as a notice. Writes and reads never wait on each other, so a record
//! that misses its writer's deadline is simply absent and the row draws its
//! plain card.
//!
//! The apps ask by message id, for the rows they draw (`session.wake_records`),
//! and a notice that rode a tool result gets its send's record in
//! `session.notices`. A row names its send only through the runner's read
//! facts (`sender` and `send_id`, llm-runner's `send_ids` group). So every page
//! read reports which send each of its messages came from: remembered at once,
//! so a records read right after the page finds it, and kept in the file, so
//! one after a restart does too. A record in the row's own session is the
//! row's. A row whose history moved here from another session takes the one
//! record its send has in any session, and none when two sessions hold one.

use std::collections::HashMap;

use cortexkit_role_tool_display::{write_outcome, InvalidParams, WakeWrite, WriteReply};
use rusqlite::OptionalExtension;
use serde_json::Value;
use subc_client_rs::HandlerOutcome;
use subc_protocol::{ErrorBody, Principal};

use super::{NoticeStore, Notices};
use crate::display::store::now_ms;
use cortexkit_store::StoreError;

/// The apps' read of the wake records for some of a session's messages.
pub const READ: &str = "session.wake_records";

/// The most messages one read asks about: a page's worth, as for tool records.
pub const MAX_MESSAGES: usize = crate::display::app::MAX_CALLS;

/// `notices.db` version 5. Each wake record, by the session it was sent to,
/// its sender and its send id; `runner` is the destination the writer named,
/// kept as provenance. Each message a page read named the send of, by runner,
/// session and message id.
pub(super) const TABLES: &str = "CREATE TABLE wake_records (
  session TEXT NOT NULL, sender TEXT NOT NULL, send_id TEXT NOT NULL,
  runner TEXT NOT NULL, body BLOB NOT NULL, written_at_ms INTEGER NOT NULL,
  PRIMARY KEY (session, sender, send_id)) WITHOUT ROWID;
CREATE INDEX wake_records_send ON wake_records (sender, send_id);
CREATE INDEX wake_records_age ON wake_records (written_at_ms);
CREATE TABLE sends (
  runner TEXT NOT NULL, session TEXT NOT NULL, mid TEXT NOT NULL,
  sender TEXT NOT NULL, send_id TEXT NOT NULL, seen_at_ms INTEGER NOT NULL,
  PRIMARY KEY (runner, session, mid)) WITHOUT ROWID;
CREATE INDEX sends_age ON sends (seen_at_ms);";

/// The send a stored message came from, as a page read's entry names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub runner: String,
    pub session: String,
    pub mid: String,
    pub sender: String,
    pub send_id: String,
}

/// Where a message is found: the digest of its runner, session and message id
/// ([`sent_key`]).
pub(super) type SentKey = crate::key_digest::KeyDigest;

pub(super) fn sent_key(runner: &str, session: &str, mid: &str) -> SentKey {
    crate::key_digest::key_digest(&[runner, session, mid])
}

/// A message's send: its sender and send id.
pub(super) type SentBy = (String, String);

/// The sends page reads named, by message, each with whether it has been
/// queued for the file.
pub(super) type Remembered = HashMap<SentKey, (SentBy, bool)>;

impl Sent {
    pub(super) fn key(&self) -> SentKey {
        sent_key(&self.runner, &self.session, &self.mid)
    }
}

/// The sends a page read's entries name: each entry with a `sender` and a
/// `send_id` beside its `mid`. Every other entry names none.
pub fn sends_of(entries: &[Value], runner: &str, session: &str) -> Vec<Sent> {
    let text = |entry: &Value, field: &str| {
        entry[field]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    entries
        .iter()
        .filter_map(|entry| {
            Some(Sent {
                runner: runner.to_owned(),
                session: session.to_owned(),
                mid: text(entry, "mid")?,
                sender: text(entry, "sender")?,
                send_id: text(entry, "send_id")?,
            })
        })
        .collect()
}

impl NoticeStore {
    /// Keep a wake record's canonical `body` for `sender`'s send `send_id` to
    /// `session`, unless it's already kept: the same bytes again are
    /// `already_stored`, different bytes a conflict, as for tool writes. The
    /// outer error is the file's; the inner one is the write's refusal.
    pub fn write_wake(
        &self,
        session: &str,
        sender: &str,
        send_id: &str,
        runner: &str,
        body: &[u8],
        now_ms: i64,
    ) -> Result<Result<WriteReply, ErrorBody>, StoreError> {
        self.file.with_conn_fenced(|tx| {
            let previous: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT body FROM wake_records
                     WHERE session = ?1 AND sender = ?2 AND send_id = ?3",
                    rusqlite::params![session, sender, send_id],
                    |row| row.get(0),
                )
                .optional()?;
            let decided = write_outcome(previous.as_deref(), body);
            if matches!(decided, Ok(WriteReply::Stored)) {
                tx.execute(
                    "INSERT INTO wake_records (session, sender, send_id, runner, body, written_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![session, sender, send_id, runner, body, now_ms],
                )?;
            }
            Ok(decided)
        })
    }

    /// The record for `sender`'s send `send_id`, as the row in `session`
    /// names it: the one kept for that session, else the only one kept for
    /// any session. Two in other sessions name no single send, so none.
    pub fn wake_of(
        &self,
        session: &str,
        sender: &str,
        send_id: &str,
    ) -> Result<Option<Value>, StoreError> {
        let found: Vec<(String, Vec<u8>)> = self.file.with_conn(|conn| {
            let mut statement = conn.prepare(
                "SELECT session, body FROM wake_records WHERE sender = ?1 AND send_id = ?2",
            )?;
            let found = statement
                .query_map(rusqlite::params![sender, send_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(found)
        })?;
        let body = match found.iter().find(|(kept, _)| kept == session) {
            Some((_, body)) => Some(body),
            None => match found.as_slice() {
                [(_, body)] => Some(body),
                _ => None,
            },
        };
        // The body was stored as checked canonical JSON, so it parses.
        Ok(body.and_then(|body| serde_json::from_slice(body).ok()))
    }

    /// Keep which send each of `sends` came from. A message's send never
    /// changes, so one kept before stays as it is.
    pub(super) fn keep_sends(&self, sends: &[Sent], now_ms: i64) -> Result<(), StoreError> {
        if sends.is_empty() {
            return Ok(());
        }
        self.file.with_conn_fenced(|tx| {
            let mut insert = tx.prepare(
                "INSERT OR IGNORE INTO sends (runner, session, mid, sender, send_id, seen_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for sent in sends {
                insert.execute(rusqlite::params![
                    sent.runner,
                    sent.session,
                    sent.mid,
                    sent.sender,
                    sent.send_id,
                    now_ms
                ])?;
            }
            Ok(())
        })
    }

    /// The sender and send id a page read named for message `mid`.
    pub(super) fn send_of(
        &self,
        runner: &str,
        session: &str,
        mid: &str,
    ) -> Result<Option<(String, String)>, StoreError> {
        self.file.with_conn(|conn| {
            conn.query_row(
                "SELECT sender, send_id FROM sends WHERE runner = ?1 AND session = ?2 AND mid = ?3",
                rusqlite::params![runner, session, mid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
        })
    }

    /// Drop the records written and the sends seen before `cutoff_ms`.
    pub(super) fn prune_wakes(&self, cutoff_ms: i64) -> Result<usize, StoreError> {
        self.file.with_conn_fenced(|tx| {
            let records = tx.execute(
                "DELETE FROM wake_records WHERE written_at_ms < ?1",
                [cutoff_ms],
            )?;
            let sends = tx.execute("DELETE FROM sends WHERE seen_at_ms < ?1", [cutoff_ms])?;
            Ok(records + sends)
        })
    }
}

impl Notices {
    /// `session.wake_records {messageIds}`: for each message of `runner`'s
    /// `session` whose send has a record, the record as its writer wrote it.
    /// A message with none is left out.
    pub async fn wake_records(
        &self,
        runner: &str,
        session: &str,
        params: &Value,
    ) -> Result<Value, occipital_app_reader::source::ReadError> {
        let mids = crate::display::app::ids(params, "messageIds", MAX_MESSAGES)?;
        let remembered: HashMap<String, SentBy> = {
            let sends = self
                .sends
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            mids.iter()
                .filter_map(|mid| {
                    sends
                        .get(&sent_key(runner, session, mid))
                        .map(|(send, _)| (mid.clone(), send.clone()))
                })
                .collect()
        };
        let (store, runner, session) = (self.store.clone(), runner.to_owned(), session.to_owned());
        let read = tokio::task::spawn_blocking(move || {
            let mut messages = serde_json::Map::new();
            for mid in mids {
                let send = match remembered.get(&mid) {
                    Some(send) => Some(send.clone()),
                    None => store
                        .send_of(&runner, &session, &mid)
                        .map_err(super::unavailable)?,
                };
                let Some((sender, send_id)) = send else {
                    continue;
                };
                if let Some(record) = store
                    .wake_of(&session, &sender, &send_id)
                    .map_err(super::unavailable)?
                {
                    messages.insert(mid, record);
                }
            }
            Ok(serde_json::json!({ "messages": messages }))
        });
        read.await.map_err(|error| {
            occipital_app_reader::source::ReadError::new(
                "internal",
                format!("wake records read stopped: {error}"),
            )
        })?
    }
}

/// Serve one `display.write` with `kind: "wake"`. `writer` is the route's
/// attested opener, `None` before the daemon stamped one. The write is checked
/// whole by the contract (`WakeWrite::check`): a verified writer, ids, the
/// record's kinds and refs, and its 16 KiB and 64-item limits, which are
/// refused rather than cut.
pub async fn write(
    notices: Option<&std::sync::Arc<Notices>>,
    writer: Option<Principal>,
    arguments: &Value,
) -> HandlerOutcome {
    // Before the notices file is open, or when it couldn't be: the contract's
    // retryable refusal for a server that isn't ready. The writer sends its
    // wake anyway, and the row draws its plain card.
    let Some(notices) = notices else {
        return crate::display::refused(cortexkit_role_tool_display::not_ready());
    };
    let writer = writer.unwrap_or(Principal::Unverified);
    let sender = match sender_of(&writer) {
        Ok(sender) => sender,
        Err(refusal) => return crate::display::refused(refusal),
    };
    let write = match wake_write(arguments) {
        Ok(write) => write,
        Err(refusal) => return crate::display::refused(refusal),
    };
    let body = match write.check(&writer) {
        Ok(body) => body,
        Err(refusal) => return crate::display::refused(refusal),
    };
    let runner = serde_json::to_string(&write.runner).unwrap_or_default();
    let store = notices.store.clone();
    let stored = tokio::task::spawn_blocking(move || {
        store.write_wake(
            &write.session,
            &sender,
            &write.send_id,
            &runner,
            &body,
            now_ms(),
        )
    })
    .await;
    match stored {
        Ok(Ok(Ok(reply))) => match serde_json::to_vec(&reply) {
            Ok(bytes) => HandlerOutcome::Response(bytes),
            Err(error) => crate::display::refused(ErrorBody::new("internal", error.to_string())),
        },
        Ok(Ok(Err(refusal))) => crate::display::refused(refusal),
        Ok(Err(error)) => crate::display::refused(ErrorBody::new(
            "provider_unavailable",
            format!("notices file unavailable: {error}"),
        )),
        Err(error) => crate::display::refused(ErrorBody::new("internal", error.to_string())),
    }
}

/// The module a write came from, as the route attests it, or the contract's
/// refusal for any other opener.
fn sender_of(writer: &Principal) -> Result<String, ErrorBody> {
    cortexkit_role_tool_display::check_writer(writer)?;
    match writer {
        Principal::Reserved { module_id } => Ok(module_id.clone()),
        // `check_writer` admits only reserved writers. Should it admit another
        // kind, that writer still names no module to key its record by.
        _ => Err(ErrorBody::new(
            cortexkit_role_tool_display::DISPLAY_WRITER_UNVERIFIED,
            "a wake record's writer must be a module",
        )),
    }
}

/// Decode a wake write. The contract's type takes no other member, so a
/// `sender` claimed in the payload is refused, as the sender comes only from
/// the route. A refusal names the first member that's missing, unknown or
/// malformed, as `invalid_params` requires.
fn wake_write(arguments: &Value) -> Result<WakeWrite, ErrorBody> {
    if let Ok(write) = serde_json::from_value::<WakeWrite>(arguments.clone()) {
        return Ok(write);
    }
    const MEMBERS: [&str; 5] = ["kind", "runner", "session", "send_id", "body"];
    let field = arguments
        .as_object()
        .and_then(|members| {
            MEMBERS
                .into_iter()
                .find(|member| !members.contains_key(*member))
                .map(str::to_owned)
                .or_else(|| {
                    members
                        .keys()
                        .find(|member| !MEMBERS.contains(&member.as_str()))
                        .cloned()
                })
        })
        .unwrap_or_else(|| "body".to_owned());
    Err(InvalidParams::new(field).refusal())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_test_support::ScratchDir;
    use serde_json::json;
    use std::sync::Arc;

    /// The contract's own wake vectors, packaged with tool-display.
    fn vectors() -> Value {
        let (_, text) = cortexkit_role_tool_display::vectors::FILES
            .iter()
            .find(|(name, _)| *name == "wakes.json")
            .unwrap();
        serde_json::from_str(text).unwrap()
    }

    fn started(dir: &ScratchDir) -> (crate::store::Store, Arc<Notices>) {
        let descriptor = crate::store::tests::descriptor(dir);
        let main = crate::store::open(&descriptor).unwrap();
        let file = Arc::new(NoticeStore::open(&descriptor, main.data_root()).unwrap());
        let (notices, _task) = Notices::start(file);
        (main, notices)
    }

    fn writer(module: &str) -> Option<Principal> {
        Some(Principal::Reserved {
            module_id: module.into(),
        })
    }

    /// The reply's body, or its error code.
    fn answer(outcome: HandlerOutcome) -> Result<Value, String> {
        match outcome {
            HandlerOutcome::Response(bytes) => Ok(serde_json::from_slice(&bytes).unwrap()),
            HandlerOutcome::Error { code, .. } | HandlerOutcome::ErrorWithDetail { code, .. } => {
                Err(code)
            }
            other => panic!("unexpected outcome {other:?}"),
        }
    }

    /// Each of the contract's writes is stored once, the same bytes again are
    /// already stored, and different bytes at the same send are a conflict.
    /// The record read back is the body as written.
    #[tokio::test]
    async fn the_contract_writes_are_stored_once_and_served_as_written() {
        let dir = ScratchDir::new("occipital-wake-writes");
        let (_main, notices) = started(&dir);
        for case in vectors()["writes"].as_array().unwrap() {
            let write = &case["write"];
            let tag = &case["tag"];
            for expected in ["stored", "already_stored"] {
                let reply = answer(write_one(&notices, "wake-writer", write).await);
                assert_eq!(reply, Ok(json!({"result": expected})), "{tag}");
            }
            let kept = notices
                .store
                .wake_of(
                    write["session"].as_str().unwrap(),
                    "wake-writer",
                    write["send_id"].as_str().unwrap(),
                )
                .unwrap();
            assert_eq!(kept.as_ref(), Some(&write["body"]), "{tag}");
        }
        let mut changed = vectors()["writes"][0]["write"].clone();
        changed["body"]["items"][0]["excerpt"] = json!("Different words.");
        assert_eq!(
            answer(write_one(&notices, "wake-writer", &changed).await),
            Err("display_write_conflict".into())
        );
    }

    async fn write_one(notices: &Arc<Notices>, sender: &str, write: &Value) -> HandlerOutcome {
        super::write(Some(notices), writer(sender), write).await
    }

    /// Each of the contract's refusals is refused with its code, and nothing
    /// is kept for it.
    #[tokio::test]
    async fn the_contract_refusals_are_refused_with_their_codes() {
        let dir = ScratchDir::new("occipital-wake-refusals");
        let (_main, notices) = started(&dir);
        for case in vectors()["refusals"].as_array().unwrap() {
            let principal: Principal = serde_json::from_value(case["writer"].clone()).unwrap();
            let reply = answer(super::write(Some(&notices), Some(principal), &case["write"]).await);
            assert_eq!(
                reply,
                Err(case["code"].as_str().unwrap().into()),
                "{}",
                case["tag"]
            );
            let write = &case["write"];
            let kept = notices
                .store
                .wake_of(
                    write["session"].as_str().unwrap(),
                    "wake-writer",
                    write["send_id"].as_str().unwrap(),
                )
                .unwrap();
            assert!(kept.is_none(), "{}", case["tag"]);
        }
    }

    /// The sender comes only from the route: a write naming one is refused,
    /// naming the member; so is one before the daemon stamped a writer, and
    /// one before the file is open.
    #[tokio::test]
    async fn a_write_claiming_its_sender_or_without_a_writer_is_refused() {
        let dir = ScratchDir::new("occipital-wake-sender");
        let (_main, notices) = started(&dir);
        let mut claimed = vectors()["writes"][0]["write"].clone();
        claimed["sender"] = json!("prefrontal-core");
        match super::write(Some(&notices), writer("wake-writer"), &claimed).await {
            HandlerOutcome::ErrorWithDetail { code, detail, .. } => {
                assert_eq!(code, "invalid_params");
                assert_eq!(detail["field"], "sender");
            }
            other => panic!("{other:?}"),
        }
        let write = &vectors()["writes"][0]["write"];
        assert_eq!(
            answer(super::write(Some(&notices), None, write).await),
            Err("display_writer_unverified".into())
        );
        assert_eq!(
            answer(super::write(None, writer("wake-writer"), write).await),
            Err(cortexkit_role_tool_display::not_ready().code)
        );
    }

    /// The contract's lookups: a record in the row's session is the row's; a
    /// moved row takes the only record its send has; two sessions holding one
    /// name no record; another module's record with the same send id is never
    /// the row's.
    #[test]
    fn the_contract_lookups_find_the_rows_record() {
        for case in vectors()["lookups"].as_array().unwrap() {
            let dir = ScratchDir::new("occipital-wake-lookups");
            let descriptor = crate::store::tests::descriptor(&dir);
            let main = crate::store::open(&descriptor).unwrap();
            let store = NoticeStore::open(&descriptor, main.data_root()).unwrap();
            for record in case["records"].as_array().unwrap() {
                let session = record["session"].as_str().unwrap();
                // Each record says which session it was written for.
                let body = serde_json::to_vec(
                    &json!({"items": [], "omittedItems": 0, "session": session}),
                )
                .unwrap();
                store
                    .write_wake(
                        session,
                        record["sender"].as_str().unwrap(),
                        record["send_id"].as_str().unwrap(),
                        "{}",
                        &body,
                        1,
                    )
                    .unwrap()
                    .unwrap();
            }
            let row = &case["row"];
            let found = store
                .wake_of(
                    row["session"].as_str().unwrap(),
                    row["sender"].as_str().unwrap(),
                    row["send_id"].as_str().unwrap(),
                )
                .unwrap();
            assert_eq!(
                found.map(|record| record["session"].clone()),
                Some(case["matched_session"].clone()).filter(|session| !session.is_null()),
                "{}",
                case["tag"]
            );
        }
    }

    /// A page read's entries name their sends. A records read right after
    /// finds the record through what was remembered, and one from a fresh
    /// start of the file finds it through what was kept. A message whose send
    /// has no record, and one from no module send, are left out.
    #[tokio::test]
    async fn a_page_reads_sends_find_their_records_now_and_after_a_restart() {
        let dir = ScratchDir::new("occipital-wake-sends");
        let descriptor = crate::store::tests::descriptor(&dir);
        let main = crate::store::open(&descriptor).unwrap();
        let file = Arc::new(NoticeStore::open(&descriptor, main.data_root()).unwrap());
        let (notices, task) = Notices::start(file.clone());
        let write = vectors()["writes"][0]["write"].clone();
        assert!(matches!(
            write_one(&notices, "prefrontal-core", &write).await,
            HandlerOutcome::Response(_)
        ));
        let entries = [
            json!({"mid": "m1", "sender": "prefrontal-core", "send_id": "send-example"}),
            json!({"mid": "m2", "sender": "prefrontal-core", "send_id": "send-unrecorded"}),
            json!({"mid": "m3"}),
        ];
        let sends = sends_of(&entries, "runner", "session-example");
        assert_eq!(sends.len(), 2);
        notices.sent(sends);
        let params = json!({"messageIds": ["m1", "m2", "m3"]});
        let expected = json!({"messages": {"m1": write["body"]}});
        let read = notices
            .wake_records("runner", "session-example", &params)
            .await
            .unwrap();
        assert_eq!(read, expected);

        for _ in 0..1_000 {
            if file
                .send_of("runner", "session-example", "m1")
                .unwrap()
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        task.abort();
        let (restarted, _task) = Notices::start(file);
        let read = restarted
            .wake_records("runner", "session-example", &params)
            .await
            .unwrap();
        assert_eq!(read, expected);
    }

    /// A riding notice's read carries its send's record as `wake`.
    #[tokio::test]
    async fn a_riding_notice_carries_its_sends_record() {
        let dir = ScratchDir::new("occipital-wake-riding");
        let (_main, notices) = started(&dir);
        let case = vectors()["writes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["tag"] == "keyed_riding_notice")
            .unwrap()
            .clone();
        let write = &case["write"];
        assert!(matches!(
            write_one(&notices, "prefrontal-core", write).await,
            HandlerOutcome::Response(_)
        ));
        let entry = json!({"mid": "m1", "notices": [{"part": 0, "call_key": "ck_one",
            "send_id": case["notice"]["send_id"], "sender": "prefrontal-core",
            "text": "Work updated.", "delivered_as": "tool_result"}]});
        let session = write["session"].as_str().unwrap();
        // Kept now: the notices task drops old notices when it starts, and one
        // dated far in the past could go before the read.
        notices
            .store
            .keep(
                &crate::notices::from_entries(&[entry], "runner", session),
                now_ms(),
            )
            .unwrap();
        let read = notices
            .read("runner", session, &json!({"callKeys": ["ck_one"]}))
            .await
            .unwrap();
        assert_eq!(read["calls"]["ck_one"][0]["wake"], write["body"]);
    }

    /// Records and sends go with the notices once they're as old as a tool
    /// display record.
    #[test]
    fn old_records_and_sends_are_dropped() {
        let dir = ScratchDir::new("occipital-wake-prune");
        let descriptor = crate::store::tests::descriptor(&dir);
        let main = crate::store::open(&descriptor).unwrap();
        let store = NoticeStore::open(&descriptor, main.data_root()).unwrap();
        store
            .write_wake("s", "core", "old", "{}", b"{\"items\":[]}", 10)
            .unwrap()
            .unwrap();
        store
            .write_wake("s", "core", "new", "{}", b"{\"items\":[]}", 30)
            .unwrap()
            .unwrap();
        let sent = |mid: &str| Sent {
            runner: "r".into(),
            session: "s".into(),
            mid: mid.into(),
            sender: "core".into(),
            send_id: mid.into(),
        };
        store.keep_sends(&[sent("old")], 10).unwrap();
        store.keep_sends(&[sent("new")], 30).unwrap();
        assert_eq!(store.prune(20).unwrap(), 2);
        assert!(store.wake_of("s", "core", "old").unwrap().is_none());
        assert!(store.wake_of("s", "core", "new").unwrap().is_some());
        assert!(store.send_of("r", "s", "old").unwrap().is_none());
        assert!(store.send_of("r", "s", "new").unwrap().is_some());
    }
}
