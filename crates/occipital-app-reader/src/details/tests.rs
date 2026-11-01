//! Every reply that describes the session (the newest page, the claim verdicts that
//! carry turns, and `session.details`) keeps the send members' rules: `sender` is an
//! object or absent, never `null`, and never appears beside `senderUnknown`.

use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::watch;

use super::{read, DetailsRequest};
use crate::source::{
    Anchor, Head, HistoryPage, ReadError, Row, SendGrant, TranscriptReader, TranscriptSource,
    SENDER_UNKNOWN,
};

/// A scan-route source: no history counter or lineage, so claims compare retained
/// content and every verdict, `diverged` included, can occur.
struct Scan {
    rows: Mutex<Vec<Row>>,
    head: Head,
    changes: watch::Sender<u64>,
}

fn row(ordinal: u64, text: &str) -> Row {
    Row {
        mid: format!("m{ordinal}"),
        ordinal: Some(ordinal),
        written_at: Some(1_700_000_000_000),
        display: json!({"role":"assistant", "text":text, "blockSummaries":[], "parts":[]}),
    }
}

impl Scan {
    fn new(head: Head) -> Self {
        Self {
            rows: Mutex::new((0..3).map(|ordinal| row(ordinal, "original")).collect()),
            head,
            changes: watch::channel(0).0,
        }
    }

    fn page(&self, start: usize, end: usize) -> HistoryPage {
        HistoryPage {
            rows: self.rows.lock().unwrap()[start..end].to_vec(),
            ..Default::default()
        }
    }

    fn position(&self, mid: &str) -> Result<usize, ReadError> {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .position(|row| row.mid == mid)
            .ok_or_else(|| ReadError::new("cursor_expired", "no such message"))
    }
}

#[async_trait]
impl TranscriptSource for Scan {
    fn harness(&self) -> &'static str {
        "opencode"
    }
    async fn head(&self) -> Result<Head, ReadError> {
        Ok(self.head.clone())
    }
    async fn newest(&self, limit: u32) -> Result<HistoryPage, ReadError> {
        let len = self.rows.lock().unwrap().len();
        Ok(self.page(len.saturating_sub(limit as usize), len))
    }
    async fn older(&self, anchor: &Anchor, limit: u32) -> Result<HistoryPage, ReadError> {
        let end = match anchor {
            Anchor::Mid(mid) => self.position(mid)?,
            _ => self.rows.lock().unwrap().len(),
        };
        Ok(self.page(end.saturating_sub(limit as usize), end))
    }
    async fn newer(&self, from_mid: &str, limit: u32) -> Result<HistoryPage, ReadError> {
        let start = self.position(from_mid)? + 1;
        let len = self.rows.lock().unwrap().len();
        Ok(self.page(start, (start + limit as usize).min(len)))
    }
    async fn exact(&self, anchor: &Anchor) -> Result<Option<Row>, ReadError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|row| match anchor {
                Anchor::Mid(mid) => row.mid == *mid,
                Anchor::Ordinal(ordinal) => row.ordinal == Some(*ordinal),
                _ => false,
            })
            .cloned())
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
    fn activity(&self) -> Option<(&'static str, i64)> {
        None
    }
}

fn claim(version: &Value) -> Value {
    json!({"session":"s", "claim":{"heldFloorMessageId":"m0", "sinceMessageId":"m2",
        "heldVersion":version}})
}

/// Each reply shape this crate emits for one head, by name.
async fn replies(head: &Head) -> Vec<(&'static str, Value)> {
    let mut replies = Vec::new();

    let source = Scan::new(head.clone());
    let reader = TranscriptReader::default();
    let newest = reader
        .read(Some(&source), json!({"session":"s"}))
        .await
        .unwrap();
    source.rows.lock().unwrap().push(row(3, "appended"));
    let matched = reader
        .read(Some(&source), claim(&newest["version"]))
        .await
        .unwrap();
    replies.push(("newest", newest));
    replies.push(("matched", matched));

    let source = Scan::new(head.clone());
    let reader = TranscriptReader::default();
    let newest = reader
        .read(Some(&source), json!({"session":"s"}))
        .await
        .unwrap();
    source.rows.lock().unwrap()[1].display["text"] = json!("rewritten");
    replies.push((
        "diverged",
        reader
            .read(Some(&source), claim(&newest["version"]))
            .await
            .unwrap(),
    ));
    // A version this reader never served has no retained snapshot to compare.
    replies.push((
        "cursor_expired",
        reader
            .read(Some(&source), claim(&json!("never-served")))
            .await
            .unwrap(),
    ));

    let request = DetailsRequest::parse(&json!({"session":"s"})).unwrap();
    replies.push(("details", read(Some(&source), &request).await.unwrap()));
    replies
}

#[tokio::test]
async fn every_described_reply_keeps_sender_an_object_or_absent_and_never_beside_the_flag() {
    let grant = json!({"modes":["steer"], "interrupt":false, "owner":true});
    let model = json!({"provider":"p", "model":"m"});
    for (state, sender) in [
        (
            "granted",
            SendGrant::Granted(grant.as_object().unwrap().clone()),
        ),
        ("unknown", SendGrant::Unknown),
        ("refused", SendGrant::MayNotSend),
    ] {
        for modelled in [true, false] {
            let head = Head {
                model: modelled.then(|| model.clone()),
                sender: sender.clone(),
                ..Default::default()
            };
            let replies = replies(&head).await;
            let names: Vec<_> = replies.iter().map(|(name, _)| *name).collect();
            assert_eq!(
                names,
                ["newest", "matched", "diverged", "cursor_expired", "details"]
            );
            for (name, reply) in &replies {
                let context = format!("{state} {name}: {reply}");
                if *name != "newest" && *name != "details" {
                    assert_eq!(reply["claim"], *name, "{context}");
                } else if *name == "newest" {
                    assert!(reply["messages"].is_array(), "{context}");
                }
                match reply.get("sender") {
                    None => {}
                    Some(Value::Object(_)) => assert!(
                        reply.get(SENDER_UNKNOWN).is_none(),
                        "both members: {context}"
                    ),
                    Some(other) => panic!("sender {other} is not an object: {context}"),
                }
                match &sender {
                    SendGrant::Granted(_) => assert_eq!(reply["sender"], grant, "{context}"),
                    SendGrant::Unknown => {
                        assert_eq!(reply[SENDER_UNKNOWN], json!(true), "{context}")
                    }
                    SendGrant::MayNotSend => {
                        assert!(reply.get("sender").is_none(), "{context}");
                        assert!(reply.get(SENDER_UNKNOWN).is_none(), "{context}");
                    }
                }
                if modelled {
                    assert_eq!(reply["model"], model, "{context}");
                } else {
                    assert!(reply.get("model").is_none(), "{context}");
                }
            }
        }
    }
}
