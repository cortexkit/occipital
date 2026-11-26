//! Notice records (`notice-record-display/v1`, tool-display §10): the data a
//! module writes for one of its own notices, such as a background command that
//! finished, so the apps draw the notice as a card from the module's own notice
//! view instead of as the folded notice text.
//!
//! The sending module writes the record before it sends the notice, through
//! `display.write` with `kind: "notice"`, keyed by its own send id, or stores it
//! and announces it on the message bus for `display.read`
//! (`crate::display::pull`). The sender is the route's attested writer, or the
//! verified module the read went to, never a member of the write. Like a wake
//! record (`super::wakes`), a record is kept once per (session, sender, send id)
//! in `notices.db`, never changed, for as long as a notice. It joins a row
//! through the send the runner's read named for that row, and a riding notice
//! through the send that notice carries. It joins only when its writer serves
//! the notice view it names, which is kept beside the tool views as
//! `nv1:<writer>:<name>:<version>`, so a module draws only its own notices with
//! its own views. The apps read it beside the wake records, when they ask with
//! `acceptNoticeRecords`.
//!
//! A module may also write a record for work another module tells the agent
//! about, such as a background command whose end prefrontal's wake reports
//! (tool-display §8.4). Then the record is keyed by a record id the writer
//! never sends, and prefrontal's wake record points at it as
//! `notice_record: {writer, id}`. The row takes the pointed record in place of
//! the wake's own card only when the wake holds that one notice and nothing
//! else (one item, no `omittedItems`), so a card in its place hides nothing,
//! and the record is kept for the wake's own session under the module the
//! pointer names, as attested at its write, that writer serves its view, and
//! its `data` fits the view's data schema. Otherwise the row keeps the wake's
//! card, so it never shows both.

use std::sync::Arc;

use cortexkit_role_tool_display::{
    NoticeBody, NoticeRecord, NoticeRecordRef, NoticeWrite, WriteReply,
};
use occipital_lang::Schema;
use serde_json::Value;
use subc_client_rs::HandlerOutcome;
use subc_protocol::{ErrorBody, Principal};

use super::wakes::{decode_write, sender_of, SendTable};
use super::{NoticeStore, Notices};
use crate::display::store::now_ms;
use cortexkit_store::StoreError;

/// `notices.db` version 6. Each notice record, by the session it was sent to,
/// its sender and its send id; `runner` is the destination the writer named,
/// kept as provenance. It has the wake records' shape, since both are one
/// module's record of one of its sends.
pub(super) const TABLES: &str = "CREATE TABLE notice_records (
  session TEXT NOT NULL, sender TEXT NOT NULL, send_id TEXT NOT NULL,
  runner TEXT NOT NULL, body BLOB NOT NULL, written_at_ms INTEGER NOT NULL,
  PRIMARY KEY (session, sender, send_id)) WITHOUT ROWID;
CREATE INDEX notice_records_send ON notice_records (sender, send_id);
CREATE INDEX notice_records_age ON notice_records (written_at_ms);";

/// The request member that asks for notice records beside the wake records
/// (`session.wake_records`) or the riding notices (`session.notices`). Without
/// it, both replies stay as they were.
pub const OPT_IN: &str = "acceptNoticeRecords";

/// Whether `writer` serves the notice view whose qualified key is `key`: the
/// join's last condition. When it does, the view's data schema, in the
/// language's subset, which a pointed record's `data` must fit. The app reader
/// answers it from the views kept beside the tool views.
pub type ServesView = Arc<dyn Fn(&str, &str) -> Option<Schema> + Send + Sync>;

/// Whether a read asked for notice records.
pub(super) fn asked(params: &Value) -> bool {
    params[OPT_IN] == Value::Bool(true)
}

impl NoticeStore {
    /// `sender`'s notice record for its send `send_id`, as a row in `session`
    /// finds it, served as the apps read it: `{sender, view, key?, data}`
    /// with the view's key qualified by the sender. None when no record is
    /// kept, or `serves` says the sender doesn't serve the view.
    pub(super) fn notice_record_of(
        &self,
        session: &str,
        sender: &str,
        send_id: &str,
        serves: &ServesView,
    ) -> Result<Option<Value>, StoreError> {
        let Some(body) = self.kept_for_send(SendTable::Notices, session, sender, send_id)? else {
            return Ok(None);
        };
        Ok(served(sender, &body, serves, false))
    }

    /// The notice record a row draws for `sender`'s send `send_id` in
    /// `session`: the send's own record, else the one its wake points at.
    /// `wake` is the send's wake record with the session it was kept for.
    pub(super) fn notice_record_for(
        &self,
        session: &str,
        sender: &str,
        send_id: &str,
        wake: Option<&(String, Value)>,
        serves: &ServesView,
    ) -> Result<Option<Value>, StoreError> {
        if let Some(record) = self.notice_record_of(session, sender, send_id, serves)? {
            return Ok(Some(record));
        }
        match wake {
            Some((kept, wake)) => self.pointed_record_of(kept, wake, serves),
            None => Ok(None),
        }
    }

    /// The record `wake`, kept for `session`, points at, when it qualifies
    /// (tool-display §8.4): the wake's only item, with nothing omitted, since
    /// the record is drawn in the wake's place; kept for that same session,
    /// never another's, under the writer the pointer names (the sender column
    /// holds each record's attested writer); its view served and its `data`
    /// fitting that view.
    fn pointed_record_of(
        &self,
        session: &str,
        wake: &Value,
        serves: &ServesView,
    ) -> Result<Option<Value>, StoreError> {
        let alone = wake["items"]
            .as_array()
            .is_some_and(|items| items.len() == 1)
            && wake.get("omittedItems").is_none();
        let Some(pointer) = wake
            .get("notice_record")
            .filter(|_| alone)
            .and_then(|pointer| serde_json::from_value::<NoticeRecordRef>(pointer.clone()).ok())
        else {
            return Ok(None);
        };
        let Some(body) = self.kept_in(SendTable::Notices, session, &pointer.writer, &pointer.id)?
        else {
            return Ok(None);
        };
        Ok(served(&pointer.writer, &body, serves, true))
    }
}

/// `sender`'s kept notice record `body` as the apps read it, when `serves`
/// says the sender serves its view, and with `fits`, when its `data` also fits
/// that view's data schema.
fn served(sender: &str, body: &[u8], serves: &ServesView, fits: bool) -> Option<Value> {
    // The body was stored after its checks, so it decodes and qualifies.
    let body = serde_json::from_slice::<NoticeBody>(body).ok()?;
    let writer = Principal::Reserved {
        module_id: sender.to_owned(),
    };
    let record = NoticeRecord::from_body(&writer, body).ok()?;
    let schema = serves(sender, &record.view)?;
    let served = serde_json::to_value(record).ok()?;
    if fits && !schema.accepts(&served["data"]) {
        return None;
    }
    Some(served)
}

/// Serve one `display.write` with `kind: "notice"`. `writer` is the route's
/// attested opener, `None` before the daemon stamped one.
pub async fn write(
    notices: Option<&Arc<Notices>>,
    views: Option<&crate::display::views::Fetcher>,
    writer: Option<Principal>,
    arguments: &Value,
) -> HandlerOutcome {
    match store(notices, views, writer, arguments).await {
        Ok(reply) => match serde_json::to_vec(&reply) {
            Ok(bytes) => HandlerOutcome::Response(bytes),
            Err(error) => crate::display::refused(ErrorBody::new("internal", error.to_string())),
        },
        Err(refusal) => crate::display::refused(refusal),
    }
}

/// Store one notice write's `arguments` as `writer`'s, whichever way they
/// arrived: pushed with `display.write`, or read back from the writer with
/// `display.read` after its bus event. The write is checked whole by the
/// contract (`NoticeWrite::check`): a verified writer, ids, the view's name and
/// version, the fold key, and the 16 KiB limit, which is refused rather than
/// cut. `data` isn't checked against the view, which may arrive later. Once
/// the record is stored, the view it names is asked for (`views`), so it is
/// kept by the time an app reads the record.
pub(crate) async fn store(
    notices: Option<&Arc<Notices>>,
    views: Option<&crate::display::views::Fetcher>,
    writer: Option<Principal>,
    arguments: &Value,
) -> Result<WriteReply, ErrorBody> {
    // Before the notices file is open, or when it couldn't be: the contract's
    // retryable refusal for a server that isn't ready. The writer sends its
    // notice anyway, and the row draws the folded notice.
    let Some(notices) = notices else {
        return Err(cortexkit_role_tool_display::not_ready());
    };
    let writer = writer.unwrap_or(Principal::Unverified);
    let sender = sender_of(&writer)?;
    let write: NoticeWrite = decode_write(arguments)?;
    let body = write.check(&writer)?;
    let view = NoticeRecord::from_body(&writer, write.body.clone())?.view;
    let runner = serde_json::to_string(&write.runner).unwrap_or_default();
    let store = notices.store.clone();
    let module = sender.clone();
    let stored = tokio::task::spawn_blocking(move || {
        store.write_once(
            SendTable::Notices,
            &write.session,
            &module,
            &write.send_id,
            &runner,
            &body,
            now_ms(),
        )
    })
    .await;
    let reply = match stored {
        Ok(Ok(result)) => result?,
        Ok(Err(error)) => {
            return Err(ErrorBody::new(
                "provider_unavailable",
                format!("notices file unavailable: {error}"),
            ))
        }
        Err(error) => return Err(ErrorBody::new("internal", error.to_string())),
    };
    if matches!(reply, WriteReply::Stored) {
        if let Some(views) = views {
            views.want(&sender, &view);
        }
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;
