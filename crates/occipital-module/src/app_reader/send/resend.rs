//! Sends kept for a runner that couldn't take them, and resent until it does
//! (`super::super::send_retry` says what is kept and for how long).
//!
//! A send is kept when a call to the session's runner fails before the send
//! goes out, such as while the runner restarts for an update, or when the
//! send's own call gets no answer. The app is answered at once, with the usual
//! accepted receipt plus `held: true`, and the apps list the send with `held`
//! until the runner takes it. One resender per session sends its kept sends
//! again in the order the app sent them, under the same send id, so a send the
//! runner did get is answered with that send and never runs twice. A later
//! send to the same session is kept behind them rather than sent past them.
//! The resender gives a send up, listed unsent, when the runner refuses it for
//! good or when it has been kept for [`RESEND_LIMIT_MS`].

use super::super::send_retry::{Kept, RESEND_LIMIT_MS, RUNNER_UNAVAILABLE};
use super::*;

/// The resender's first wait after a resend the runner couldn't take, doubled
/// after each one up to [`RESEND_MAX_WAIT`]. A runner's restart takes seconds.
pub const RESEND_FIRST_WAIT: Duration = Duration::from_secs(1);
pub const RESEND_MAX_WAIT: Duration = Duration::from_secs(10);

/// What one resend did.
enum Resent {
    /// The session has no kept send left.
    Nothing,
    /// The first kept send is settled: taken, refused for good or given up.
    Settled,
    /// The runner still can't take it; try again later.
    Later,
}

impl Sender {
    /// Whether `session` has sends kept for its runner, which a new send to it
    /// must wait behind. A store that can't be read answers no: the new send
    /// then goes to the runner, as it would without this module keeping any.
    pub(super) async fn keeps(&self, session: &str) -> bool {
        let Some(receipts) = self.receipts.clone() else {
            return false;
        };
        let session = session.to_owned();
        tokio::task::spawn_blocking(move || receipts.queue().first_kept(&session))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .is_some()
    }

    /// Whether this message is kept to be resent, so "Send now" can't move it:
    /// the runner hasn't got it.
    pub(super) async fn is_kept(&self, session: &str, id: &str) -> bool {
        let Some(receipts) = self.receipts.clone() else {
            return false;
        };
        let (session, id) = (session.to_owned(), id.to_owned());
        tokio::task::spawn_blocking(move || receipts.queue().is_kept(&session, &id))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    }

    /// Keep `request` to be resent, and answer the app with an accepted receipt
    /// marked `held`. `why` says what the runner call answered, for the log.
    /// Without receipts there is nowhere to keep it, and it is answered
    /// `not_sent`, which the app may resend under the same id.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn keep(
        &self,
        request: &SendRequest,
        device: Option<&str>,
        params: &Value,
        digest: &str,
        send_id: &str,
        admitted: Option<crate::moves::state::Admitted>,
        why: &str,
    ) -> Result<Value, SendRefusal> {
        let (Some(id), Some(receipts)) = (request.client_message_id.clone(), self.receipts.clone())
        else {
            return Ok(not_sent(Some(send_id), request, why));
        };
        let now = (self.clock)();
        let kept = Kept {
            session: request.session.clone(),
            client_message_id: id,
            device: device.map(str::to_owned),
            params: params.clone(),
            kept_at: now,
            give_up_at: now.saturating_add(RESEND_LIMIT_MS),
        };
        let mut receipt = accepted(send_id, request);
        receipt["held"] = json!(true);
        let (row, digest, sid) = (receipt.clone(), digest.to_owned(), send_id.to_owned());
        let mode = request.mode.unwrap_or(Mode::Queue).as_str();
        let harness = request.harness.clone();
        let stored = kept.clone();
        tokio::task::spawn_blocking(move || {
            // Hold the move admission until the rows are written, as a send does.
            let _admitted = admitted;
            receipts.keep(&stored, &digest, mode, &sid, &row, harness.as_deref())
        })
        .await
        .map_err(|error| store_error(error.to_string()))?
        .map_err(|error| store_error(error.to_string()))?;
        crate::app_reader::observe::answer(format!("mode={mode} kept"));
        tracing::info!(target: "app_send", session = %kept.session,
            client_message_id = %kept.client_message_id, reason = why,
            "send kept to resend");
        let _ = self.queue_changed.send(kept.session.clone());
        self.resume(kept.session);
        Ok(receipt)
    }

    /// Start a resender for every session with kept sends, as a restart finds
    /// them.
    pub(super) fn resume_kept(&self) {
        let Some(receipts) = self.receipts.clone() else {
            return;
        };
        let Some(me) = self.me.get().cloned() else {
            return;
        };
        self.tasks.guard().spawn(async move {
            let sessions = tokio::task::spawn_blocking(move || receipts.queue().kept_sessions())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            if let Some(sender) = me.upgrade() {
                for session in sessions {
                    sender.resume(session);
                }
            }
        });
    }

    /// Start `session`'s resender unless one is running. It holds the sender
    /// weakly, so it ends with it.
    fn resume(&self, session: String) {
        let Some(me) = self.me.get().cloned() else {
            return;
        };
        if !self.resending.guard().insert(session.clone()) {
            return;
        }
        self.tasks.guard().spawn(resend(me, session));
    }

    /// Resend the session's first kept send, or give it up.
    async fn resend_first(&self, session: &str) -> Resent {
        let Some(receipts) = self.receipts.clone() else {
            return Resent::Nothing;
        };
        let read = session.to_owned();
        let first = tokio::task::spawn_blocking(move || receipts.queue().first_kept(&read)).await;
        let kept = match first {
            Ok(Ok(Some(kept))) => kept,
            Ok(Ok(None)) => return Resent::Nothing,
            _ => return Resent::Later,
        };
        if (self.clock)() >= kept.give_up_at {
            return self.give_up(&kept, RUNNER_UNAVAILABLE).await;
        }
        match self.send_kept(&kept).await {
            Ok(true) => Resent::Settled,
            Ok(false) => Resent::Later,
            // A store or placement that can't be read now may be readable on
            // the next try, a file the runner lost is copied again, and a
            // refusal the runner says may pass later is tried again until the
            // limit. Any other refusal is final for this send.
            Err(refusal)
                if refusal.code == "provider_unavailable"
                    || refusal.code == UPLOAD_GONE
                    || refusal
                        .detail
                        .as_ref()
                        .is_some_and(|detail| detail["class"] == "transient") =>
            {
                Resent::Later
            }
            Err(refusal) => self.give_up(&kept, refusal.code).await,
        }
    }

    /// Send `kept` again as the app first sent it. `Ok(true)` when the runner
    /// answered, `Ok(false)` when it still can't be reached.
    async fn send_kept(&self, kept: &Kept) -> Result<bool, SendRefusal> {
        let request = SendRequest::parse(&kept.params)?;
        let key = send_key(&request)?;
        let digest = self.digest(kept.device.as_deref(), &request).await?;
        let requested = request.mode.unwrap_or(Mode::Queue);
        let lock = self.lock(&request.session, &kept.client_message_id);
        let _guard = lock.lock().await;
        if let Some(cuts) = self.moves.get() {
            cuts.check_readable()?;
        }
        let admitted = self
            .moves
            .get()
            .map(|cuts| cuts.admit(&request.session))
            .transpose()?;
        let target = match self
            .targets
            .open(&request.session, request.harness.as_deref())
            .await
        {
            Ok(target) => target,
            Err(error) if error.code == "session_unknown" => {
                return Err(SendRefusal::new(codes::NOT_FOUND, error.message))
            }
            Err(error) if error.code == "session_on_other_machine" => {
                return Err(SendRefusal::new(error.code, error.message))
            }
            Err(_) => return Ok(false),
        };
        let head_id = runner_send_id(target.reference(), key, Role::Head);
        let delivered = self
            .deliver(
                target,
                &request,
                kept.device.as_deref(),
                &digest,
                requested,
                head_id,
                admitted,
                true,
            )
            .await?;
        Ok(matches!(delivered, Delivered::Answered(_)))
    }

    /// Resend the session's first kept send once, as its resender would, and
    /// say whether it settled.
    #[cfg(test)]
    pub(crate) async fn resend_kept_now(&self, session: &str) -> bool {
        matches!(self.resend_first(session).await, Resent::Settled)
    }

    /// List `kept` unsent with `reason`, and stop keeping it.
    async fn give_up(&self, kept: &Kept, reason: &str) -> Resent {
        let Some(receipts) = self.receipts.clone() else {
            return Resent::Nothing;
        };
        let (session, id, why, now) = (
            kept.session.clone(),
            kept.client_message_id.clone(),
            reason.to_owned(),
            (self.clock)(),
        );
        let written =
            tokio::task::spawn_blocking(move || receipts.give_up(&session, &id, &why, now)).await;
        if !matches!(written, Ok(Ok(()))) {
            return Resent::Later;
        }
        tracing::info!(target: "app_send", session = %kept.session,
            client_message_id = %kept.client_message_id, reason,
            "kept send given up");
        let _ = self.queue_changed.send(kept.session.clone());
        Resent::Settled
    }
}

/// One session's resender: each kept send in turn, waiting between tries
/// while the runner can't take one, until none is left.
async fn resend(me: Weak<Sender>, session: String) {
    let mut wait = RESEND_FIRST_WAIT;
    loop {
        let Some(sender) = me.upgrade() else {
            return;
        };
        match sender.resend_first(&session).await {
            Resent::Settled => wait = RESEND_FIRST_WAIT,
            Resent::Later => {
                drop(sender);
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(RESEND_MAX_WAIT);
            }
            Resent::Nothing => {
                // A send kept between the read and this point found the
                // resender still marked running, so look once more after
                // clearing the mark: it either goes on here or has its own.
                sender.resending.guard().remove(&session);
                if !sender.keeps(&session).await
                    || !sender.resending.guard().insert(session.clone())
                {
                    return;
                }
            }
        }
    }
}
