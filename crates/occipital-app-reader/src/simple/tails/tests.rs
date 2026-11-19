use super::*;
use crate::source::Positioned;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::AtomicU64;
use tokio::sync::watch;

/// A chat whose messages are hidden tool turns, except those named visible,
/// which are the person's. It counts the messages it serves.
struct Chat {
    rows: std::sync::Mutex<Vec<Row>>,
    served: AtomicU64,
    version: std::sync::Mutex<u64>,
    changes: watch::Sender<u64>,
}

fn row(ordinal: u64, visible: bool) -> Row {
    let display: Value = if visible {
        json!({"role": "user", "parts": [{"type": "text", "text": format!("message {ordinal}")}]})
    } else {
        json!({"role": "assistant", "parts": [{"type": "tool", "tool": "bash", "input": {}}]})
    };
    Row {
        mid: format!("m{ordinal}"),
        ordinal: Some(ordinal),
        written_at: None,
        display,
    }
}

impl Chat {
    fn new(length: u64, visible: &[u64]) -> Self {
        Self {
            rows: std::sync::Mutex::new(
                (1..=length)
                    .map(|at| row(at, visible.contains(&at)))
                    .collect(),
            ),
            served: AtomicU64::new(0),
            version: std::sync::Mutex::new(1),
            changes: watch::channel(0).0,
        }
    }

    fn page(&self, rows: Vec<Row>, older: bool) -> HistoryPage {
        self.served.fetch_add(rows.len() as u64, Ordering::SeqCst);
        HistoryPage {
            rows,
            older: older.then(|| "older".to_owned()),
            newer: None,
            history_version: Some(*self.version.lock().unwrap()),
            incarnation: Some("inc".into()),
        }
    }

    fn head(&self) -> Head {
        Head {
            history_version: Some(*self.version.lock().unwrap()),
            incarnation: Some("inc".into()),
            first_ordinal: Some(1),
            last_ordinal: self.rows.lock().unwrap().last().and_then(|row| row.ordinal),
            ..Default::default()
        }
    }

    fn served(&self) -> u64 {
        self.served.swap(0, Ordering::SeqCst)
    }
}

#[async_trait]
impl TranscriptSource for Chat {
    fn harness(&self) -> &'static str {
        "broca"
    }
    async fn head(&self) -> Result<Head, ReadError> {
        Ok(Chat::head(self))
    }
    async fn newest(&self, limit: u32) -> Result<HistoryPage, ReadError> {
        let rows = self.rows.lock().unwrap().clone();
        let start = rows.len().saturating_sub(limit as usize);
        Ok(self.page(rows[start..].to_vec(), start > 0))
    }
    async fn older(&self, anchor: &Anchor, limit: u32) -> Result<HistoryPage, ReadError> {
        let Anchor::Ordinal(before) = anchor else {
            panic!("a Simple read goes back by ordinal: {anchor:?}");
        };
        // As a runner does, an anchor past the newest message is refused.
        let last = self.rows.lock().unwrap().last().and_then(|row| row.ordinal);
        if last.is_some_and(|last| *before > last) {
            return Err(ReadError::new("cursor_expired", "no such message"));
        }
        let below: Vec<Row> = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.ordinal.unwrap() < *before)
            .cloned()
            .collect();
        let start = below.len().saturating_sub(limit as usize);
        Ok(self.page(below[start..].to_vec(), start > 0))
    }
    async fn newer(&self, _: &str, _: u32) -> Result<HistoryPage, ReadError> {
        panic!("a Simple read never reads newer by message id")
    }
    async fn after(&self, ordinal: u64, limit: u32) -> Result<Positioned, ReadError> {
        let rows = self.rows.lock().unwrap().clone();
        let newest = rows.last().and_then(|row| row.ordinal);
        let page: Vec<Row> = rows
            .into_iter()
            .filter(|row| row.ordinal.unwrap() > ordinal)
            .take(limit as usize)
            .collect();
        Ok(Positioned {
            page: self.page(page, true),
            anchor: None,
            newest,
        })
    }
    async fn exact(&self, _: &Anchor) -> Result<Option<Row>, ReadError> {
        panic!("a Simple read never reads one message")
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    fn activity(&self) -> Option<(&'static str, i64)> {
        None
    }
}

fn ordinals(found: &Found) -> Vec<u64> {
    found.rows.iter().map(|row| row.ordinal.unwrap()).collect()
}

const VERSION: &str = "history:1:incarnation:inc:simple:1";

/// A cold newest page reads back until it holds its rows, and goes on from
/// where it stopped; the page before it is read on from the kept range and
/// reaches the start.
#[tokio::test]
async fn a_newest_page_reads_back_until_it_holds_its_rows() {
    let chat = Chat::new(500, &[10, 200, 450, 490]);
    let tails = Tails::default();
    let head = chat.head();
    let newest = tails.newest(&chat, "s", &head, VERSION, 3).await.unwrap();
    assert_eq!(ordinals(&newest), [200, 450, 490]);
    assert_eq!(newest.next_before, Some(101), "where the read stopped");
    assert_eq!(chat.served(), 400);

    let older = tails
        .older(&chat, "s", &head, VERSION, 101, 3)
        .await
        .unwrap();
    assert_eq!(ordinals(&older), [10]);
    assert_eq!(older.next_before, None, "the start");
    assert_eq!(chat.served(), 100);
}

/// A read that finds nothing within its bound says where it stopped, with no
/// rows and not at the start.
#[tokio::test]
async fn a_read_stops_at_its_bound() {
    let chat = Chat::new(4_000, &[]);
    let tails = Tails::default();
    let head = chat.head();
    let newest = tails.newest(&chat, "s", &head, VERSION, 20).await.unwrap();
    assert!(newest.rows.is_empty());
    assert_eq!(newest.next_before, Some(2_001));
    assert_eq!(chat.served(), NEWEST_BOUND);

    let older = tails
        .older(&chat, "s", &head, VERSION, 2_001, 20)
        .await
        .unwrap();
    assert!(older.rows.is_empty());
    assert_eq!(older.next_before, Some(1_001));
    assert_eq!(chat.served(), BACKWARD_BOUND);
}

/// Once a chat's rows are kept, a newest page reads only what was added
/// since and the newest messages again, where a row can still change.
#[tokio::test]
async fn a_kept_chat_reads_only_its_newest_messages_again() {
    let chat = Chat::new(1_000, &[100, 990]);
    let tails = Tails::default();
    let first = tails
        .newest(&chat, "s", &chat.head(), VERSION, 2)
        .await
        .unwrap();
    assert_eq!(ordinals(&first), [100, 990]);
    chat.served();

    // Five more messages, the last the person's, and a step in the recheck
    // window whose call turned out to file an ask.
    {
        let mut rows = chat.rows.lock().unwrap();
        for at in 1_001..=1_005 {
            rows.push(row(at, at == 1_005));
        }
        rows[979].display = json!({"role": "assistant", "parts": [
            {"type": "tool", "tool": "ask", "input": {"question": "Ship it?"}}]});
    }
    let second = tails
        .newest(&chat, "s", &chat.head(), VERSION, 3)
        .await
        .unwrap();
    assert_eq!(ordinals(&second), [980, 990, 1_005]);
    assert_eq!(
        chat.served(),
        RECHECK + 1 + 5,
        "the recheck window and the new five"
    );
}

/// A record written for a call a kept row draws, or a new version, makes the
/// next read find the chat's rows afresh.
#[tokio::test]
async fn kept_rows_go_with_a_late_record_or_a_new_version() {
    let chat = Chat::new(300, &[150]);
    chat.rows.lock().unwrap()[49].display = json!({"role": "assistant", "parts": [
        {"type": "message", "callKey": "k-say", "text": "Done."}]});
    let tails = Tails::default();
    let head = chat.head();
    tails.newest(&chat, "s", &head, VERSION, 2).await.unwrap();
    chat.served();

    tails.forget_call("k-other");
    tails.newest(&chat, "s", &head, VERSION, 2).await.unwrap();
    assert_eq!(chat.served(), RECHECK + 1, "still kept");

    tails.forget_call("k-say");
    tails.newest(&chat, "s", &head, VERSION, 2).await.unwrap();
    assert_eq!(chat.served(), 300, "read afresh");

    *chat.version.lock().unwrap() = 2;
    let head = chat.head();
    tails
        .newest(&chat, "s", &head, "history:2:incarnation:inc:simple:1", 2)
        .await
        .unwrap();
    assert_eq!(chat.served(), 300, "a new version is read afresh");
}

/// A page from another version than the head's is refused for now: its rows
/// can't join the head's copy.
#[tokio::test]
async fn a_page_of_another_version_is_refused() {
    let chat = Chat::new(10, &[5]);
    let head = chat.head();
    *chat.version.lock().unwrap() = 2;
    let refused = Tails::default()
        .newest(&chat, "s", &head, VERSION, 2)
        .await
        .unwrap_err();
    assert_eq!(refused.code, "provider_unavailable");
}
