use super::{admit, AppReader, RouteBinding, DISPLAY_WATCH};
use async_trait::async_trait;
use occipital_app_reader::{
    claim::NewestPageFit,
    display::{DisplayProjection, DisplayReadError, DisplayReadReply, DisplayReadRequest},
    display_watch::{self, DisplayWatchHost, DisplayWatchStopped},
    source::{ReadError, TranscriptSource},
    transcript_watch::{self, TranscriptWatchEnd, TranscriptWatchHost},
    watches::WatchSlot,
};
use serde_json::Value;
use std::sync::Arc;
use subc_client_rs::{HandlerOutcome, RequestCtx};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// One app request's way back to the app: which request it is, where a watch's
/// notices go, and when the app stops listening. The SDK's request context is
/// the production exchange; tests supply their own to see every frame a
/// request produces.
#[async_trait]
pub(crate) trait Exchange: Send + Sync {
    /// The route's channel and epoch, and the request's correlation id.
    fn key(&self) -> (u16, u32, u64);
    /// Send one watch notice. `Err` once the notice can no longer be sent.
    async fn emit(&self, body: Vec<u8>) -> Result<(), ()>;
    /// Cancelled when the app cancels the request or its route goes.
    fn cancellation(&self) -> CancellationToken;
}

/// A request the daemon delivered through the module SDK.
pub(crate) struct Sdk(pub RequestCtx);

#[async_trait]
impl Exchange for Sdk {
    fn key(&self) -> (u16, u32, u64) {
        let handle = self.0.route_handle();
        (handle.channel, handle.epoch, self.0.corr())
    }
    async fn emit(&self, body: Vec<u8>) -> Result<(), ()> {
        self.0.emit(body).await.map_err(|_| ())
    }
    fn cancellation(&self) -> CancellationToken {
        self.0.cancellation_token()
    }
}

/// A read, a send, a Stop or an upload, answered as one reply.
pub(super) async fn serve_read(
    app: Arc<AppReader>,
    caller: super::Caller,
    method: String,
    params: Value,
    correlation: String,
) -> HandlerOutcome {
    let session = params["session"].as_str().unwrap_or_default().to_string();
    super::observe::request(&method, &session, correlation, async {
        let work = async {
            let result =
                super::observe::step("reader", app.read_as(Some(&caller), &method, params)).await;
            super::observe::synchronous("reply_encode", || match result {
                Ok(value) => result_outcome(value),
                Err(error) => {
                    super::observe::outcome(error.code);
                    error_outcome(&error)
                }
            })
        };
        match super::observe::deadline(&method, work).await {
            Ok(outcome) => outcome,
            Err(_) => {
                super::observe::outcome("deadline");
                error_outcome(
                    &ReadError::new(
                        "provider_unavailable",
                        "reader request exceeded its 45 second deadline; retry this read",
                    )
                    .with_detail(serde_json::json!({"retryable":true})),
                )
            }
        }
    })
    .await
}

/// A successful reply: the value under `result`, as every app operation answers.
pub(super) fn result_outcome(value: Value) -> HandlerOutcome {
    match serde_json::to_vec(&serde_json::json!({ "result": value })) {
        Ok(body) => HandlerOutcome::Response(body),
        Err(error) => {
            super::observe::outcome("reply_encode_failed");
            HandlerOutcome::Error {
                code: "internal".into(),
                message: format!("cannot encode the reply: {error}"),
            }
        }
    }
}

pub(super) fn error_outcome(error: &ReadError) -> HandlerOutcome {
    detailed_error(error.code, &error.message, error.detail.clone())
}

fn detailed_error(code: &str, message: &str, detail: Option<Value>) -> HandlerOutcome {
    match detail {
        Some(detail) => HandlerOutcome::ErrorWithDetail {
            code: code.into(),
            message: message.into(),
            detail,
        },
        None => HandlerOutcome::Error {
            code: code.into(),
            message: message.into(),
        },
    }
}

struct Watch {
    app: Arc<AppReader>,
    binding: Option<RouteBinding>,
    params: Value,
    exchange: Box<dyn Exchange>,
    ended: watch::Receiver<bool>,
    /// Set when the app cancels the watch or its route goes.
    cancelled: CancellationToken,
    source: Option<Arc<dyn TranscriptSource>>,
    quiet: watch::Sender<u64>,
}

async fn signalled(mut receiver: watch::Receiver<bool>) {
    loop {
        if *receiver.borrow_and_update() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}

async fn source_lost(receiver: Option<watch::Receiver<bool>>) {
    let Some(mut receiver) = receiver else {
        return std::future::pending().await;
    };
    loop {
        let live = *receiver.borrow_and_update();
        if !live || receiver.changed().await.is_err() {
            return;
        }
    }
}

impl Watch {
    /// A transcript watch notice with the session's queued and unsent sends on it, as
    /// every page reply carries them. A notice the list would push past the notice
    /// bound goes out without it; the next page read carries the list.
    async fn with_queued(&self, body: Vec<u8>) -> Vec<u8> {
        let Some(session) = self.params["session"].as_str() else {
            return body;
        };
        let Ok(mut notice) = serde_json::from_slice::<Value>(&body) else {
            return body;
        };
        if !notice.is_object() {
            return body;
        }
        self.app.sends.attach_queued(&mut notice, session).await;
        match serde_json::to_vec(&notice) {
            Ok(enriched) if enriched.len() <= transcript_watch::NOTICE_BOUND => enriched,
            _ => body,
        }
    }

    async fn stop(&self) {
        tokio::select! {
            _ = signalled(self.ended.clone()) => {},
            _ = self.cancelled.cancelled() => {},
        }
    }

    async fn emit_body(&self, body: Vec<u8>) -> Result<(), ()> {
        if body.len() > transcript_watch::NOTICE_BOUND {
            return Err(());
        }
        tokio::select! {
            biased;
            _ = self.stop() => Err(()),
            result = self.exchange.emit(body) => result,
        }
    }
}

/// Put each late send outcome for `session` on its transcript watch, as its own
/// notice beside the claim replies: a worker steer that expired before a run took
/// it is the app's only word that the message was never delivered. Never returns;
/// the watch's own end stops it.
async fn forward_late(
    host: &Watch,
    mut outcomes: tokio::sync::broadcast::Receiver<super::send::LateOutcome>,
    mut queue: Option<tokio::sync::broadcast::Receiver<String>>,
    session: &str,
) -> std::convert::Infallible {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        let changed = async {
            match queue.as_mut() {
                Some(queue) => queue.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            outcome = outcomes.recv() => match outcome {
                Ok(outcome) if outcome.session == session => {
                    // A JSON value's text is its compact serialization.
                    let body = outcome.notice.to_string().into_bytes();
                    if host.emit_body(body).await.is_err() {
                        return std::future::pending().await;
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return std::future::pending().await,
            },
            // A watch that opted in (`acceptQueuedSends`) is told when only the list
            // changed: a send queued or settled while nothing stored changed. A missed
            // change (`Lagged`) sends the current list too.
            change = changed => match change {
                Ok(changed) if changed != session => {}
                Ok(_) | Err(RecvError::Lagged(_)) => {
                    let list = host.app.sends.queued(session).await;
                    let mut notice = serde_json::json!({});
                    super::send_queue::attach(&mut notice, list);
                    // An emptied list is said as an empty one, so the phone clears it.
                    if let Some(notice) = notice.as_object_mut() {
                        notice
                            .entry("queuedSends")
                            .or_insert(serde_json::json!([]));
                    }
                    let body = notice.to_string().into_bytes();
                    if host.emit_body(body).await.is_err() {
                        return std::future::pending().await;
                    }
                }
                Err(RecvError::Closed) => queue = None,
            },
        }
    }
}

/// How long a records watch gathers hints before it sends them, so a burst of
/// writes, such as a record and its detail, is one notice.
const GATHER: std::time::Duration = std::time::Duration::from_millis(250);

/// `session.tool_records_watch {session, harness?}`: tell the app which of its
/// session's calls got new display data, as `{callKeys}`, and which pins got a
/// newer template, as `{pins}`, so it reads those again. Notices carry no data.
/// The first notice, `{callKeys: []}`, says the watch is listening, so a
/// records read after it misses nothing. A watch that falls behind ends, and
/// the app reads everything again when it reopens.
pub(super) async fn serve_records_watch(
    app: Arc<AppReader>,
    params: Value,
    exchange: Box<dyn Exchange>,
    slot: WatchSlot,
) -> HandlerOutcome {
    use crate::display::app::Hint;
    use std::collections::BTreeSet;
    use tokio::sync::broadcast::error::RecvError;

    let cancelled = exchange.cancellation();
    let ended = slot.ended();
    let work = async {
        let tool = app.tool_display()?.clone();
        let reference = app.session_reference(&params).await?;
        let mut hints = tool.hints().subscribe();
        let emit = |notice: Value| {
            let body = notice.to_string().into_bytes();
            exchange.emit(body)
        };
        if emit(serde_json::json!({"callKeys": []})).await.is_err() {
            return Ok(());
        }
        loop {
            let mut call_keys = BTreeSet::new();
            let mut pins = BTreeSet::new();
            let mut gathering: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
            loop {
                let next = match gathering.as_mut() {
                    None => hints.recv().await,
                    Some(deadline) => tokio::select! {
                        _ = deadline => break,
                        next = hints.recv() => next,
                    },
                };
                match next {
                    Ok(Hint::Write { runner, call_key }) => {
                        if runner == reference.runner
                            && tool
                                .is_session_call(&runner, &reference.session, &call_key)
                                .await
                        {
                            call_keys.insert(call_key);
                        }
                    }
                    Ok(Hint::Pin(pin)) => {
                        pins.insert(pin);
                    }
                    // Behind or closing: end, and the reopened watch starts over.
                    Err(RecvError::Lagged(_) | RecvError::Closed) => return Ok(()),
                }
                if gathering.is_none() && !(call_keys.is_empty() && pins.is_empty()) {
                    gathering = Some(Box::pin(tokio::time::sleep(GATHER)));
                }
            }
            if !call_keys.is_empty()
                && emit(serde_json::json!({ "callKeys": call_keys }))
                    .await
                    .is_err()
            {
                return Ok(());
            }
            if !pins.is_empty() && emit(serde_json::json!({ "pins": pins })).await.is_err() {
                return Ok(());
            }
        }
    };
    let result: Result<(), ReadError> = tokio::select! {
        biased;
        _ = signalled(ended) => Ok(()),
        _ = cancelled.cancelled() => Ok(()),
        result = work => result,
    };
    drop(slot);
    match result {
        Ok(()) => HandlerOutcome::Streamed,
        Err(error) => error_outcome(&error),
    }
}

/// `session.notices_watch {session, harness?}`: tell the app which of its
/// session's calls a notice rode, as `{callKeys}`, so it reads their notices
/// again. Notices carry no data. The first notice, `{callKeys: []}`, says the
/// watch is listening, so a notices read after it misses nothing.
///
/// The watch holds the session's runner subscription, as the transcript and
/// display watches do, since a riding notice arrives on it
/// (`notice_delivered`). When that subscription is lost, or the watch falls
/// behind, it ends, and the app reads again when it reopens.
pub(super) async fn serve_notices_watch(
    app: Arc<AppReader>,
    params: Value,
    exchange: Box<dyn Exchange>,
    slot: WatchSlot,
) -> HandlerOutcome {
    use std::collections::BTreeSet;
    use tokio::sync::broadcast::error::RecvError;

    let cancelled = exchange.cancellation();
    let ended = slot.ended();
    let work = async {
        let notices = app.notices()?.clone();
        let reference = app.session_reference(&params).await?;
        let harness = occipital_app_reader::page::optional_transcript_harness(&params)
            .map_err(|error| ReadError::new(error.code, error.message))?;
        let mut arrivals = notices.arrivals();
        let followed = app
            .resolver
            .watch(&reference.session, harness, app.display.clone())
            .await?;
        let _lease = followed.lease;
        let emit = |notice: Value| {
            let body = notice.to_string().into_bytes();
            exchange.emit(body)
        };
        if emit(serde_json::json!({"callKeys": []})).await.is_err() {
            return Ok(());
        }
        let hinting = async {
            loop {
                let mut call_keys = BTreeSet::new();
                let mut gathering: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
                loop {
                    let next = match gathering.as_mut() {
                        None => arrivals.recv().await,
                        Some(deadline) => tokio::select! {
                            _ = deadline => break,
                            next = arrivals.recv() => next,
                        },
                    };
                    match next {
                        Ok(arrived) => {
                            if arrived.runner == reference.runner
                                && arrived.session == reference.session
                            {
                                call_keys.insert(arrived.call_key);
                            }
                        }
                        // Behind or closing: end, and the reopened watch starts over.
                        Err(RecvError::Lagged(_) | RecvError::Closed) => return Ok(()),
                    }
                    if gathering.is_none() && !call_keys.is_empty() {
                        gathering = Some(Box::pin(tokio::time::sleep(GATHER)));
                    }
                }
                if emit(serde_json::json!({ "callKeys": call_keys }))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
        };
        tokio::select! {
            biased;
            _ = source_lost(followed.live) => Ok(()),
            result = hinting => result,
        }
    };
    let result: Result<(), ReadError> = tokio::select! {
        biased;
        _ = signalled(ended) => Ok(()),
        _ = cancelled.cancelled() => Ok(()),
        result = work => result,
    };
    drop(slot);
    match result {
        Ok(()) => HandlerOutcome::Streamed,
        Err(error) => error_outcome(&error),
    }
}

/// A stop interrupts source reads and stalled writes, not just the loop's idle wait.
pub(super) async fn serve(
    app: Arc<AppReader>,
    binding: Option<RouteBinding>,
    method: String,
    params: Value,
    exchange: Box<dyn Exchange>,
    correlation: String,
    slot: WatchSlot,
) -> HandlerOutcome {
    let cancelled = exchange.cancellation();
    let mut host = Watch {
        app,
        binding,
        params,
        exchange,
        ended: slot.ended(),
        cancelled: cancelled.clone(),
        source: None,
        quiet: watch::channel(0).0,
    };
    let ended = slot.ended();
    // Subscribed before the watch starts, so an outcome settled while the session
    // is still being resolved is not missed.
    let late = host.app.sends.late_outcomes();
    // Only a watch that opted in gets a notice for a list change alone: a phone build
    // that predates it ends a watch on a notice kind it doesn't know.
    let queue_changes = (host.params["acceptQueuedSends"] == Value::Bool(true))
        .then(|| host.app.sends.queue_changes());
    let work = async {
        let selection = if method == DISPLAY_WATCH {
            let request = DisplayReadRequest::parse(&host.params)
                .map_err(|e| (e.code.to_owned(), e.message, None))?;
            let harness = occipital_app_reader::page::optional_transcript_harness(&host.params)
                .map_err(|e| (e.code.to_owned(), e.message, None))?;
            Some((request.session.to_owned(), harness.map(str::to_string)))
        } else {
            occipital_app_reader::page::PageRequest::parse(&host.params)
                .ok()
                .map(|request| (request.session_id, request.requested_harness))
        };
        let watched = selection.as_ref().map(|(session, _)| session.clone());
        let source_watch = if let Some((session, harness)) = selection {
            // A watch is logged once, for its opening: the session's resolution, which
            // either starts the watch or ends it at once with the refusal.
            let resolver = host.app.resolver.clone();
            let display = host.app.display.clone();
            super::observe::request(&method, &session, correlation, async {
                let opened = resolver.watch(&session, harness.as_deref(), display).await;
                if let Err(error) = &opened {
                    super::observe::outcome(error.code);
                }
                opened
            })
            .await
            .map_err(|e| (e.code.to_owned(), e.message, e.detail))?
        } else {
            super::SourceWatch::default()
        };
        host.source = source_watch.source;
        let _lease = source_watch.lease;
        let running = async {
            if method == DISPLAY_WATCH {
                return display_watch::run(&host, host.params.clone())
                    .await
                    .map_err(|e| (e.code.to_owned(), e.message, None));
            }
            match transcript_watch::run(&host, &host.params).await {
                TranscriptWatchEnd::Streamed => Ok(()),
                TranscriptWatchEnd::Error { code, message } => Err((code, message, None)),
                TranscriptWatchEnd::Refused(e) => Err((e.code.to_owned(), e.message, e.detail)),
            }
        };
        let forwarding = async {
            match (method == DISPLAY_WATCH, watched) {
                (false, Some(session)) => forward_late(&host, late, queue_changes, &session).await,
                _ => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            _ = source_lost(source_watch.live) => Ok(()),
            result = running => result,
            never = forwarding => match never {},
        }
    };
    // Keep independent receivers: `work` may be borrowing the host mutably during resolution.
    let result = tokio::select! {
        biased;
        _ = signalled(ended) => Ok(()),
        _ = cancelled.cancelled() => Ok(()),
        result = work => result,
    };
    drop(slot);
    match result {
        Ok(()) => HandlerOutcome::Streamed,
        Err((code, message, detail)) => detailed_error(&code, &message, detail),
    }
}

#[async_trait]
impl TranscriptWatchHost for Watch {
    type Refusal = ReadError;

    async fn read(&self, params: Value) -> Result<Value, ReadError> {
        self.app
            .reader
            .read_for_watch(self.source.as_deref(), params)
            .await
    }

    fn trimmed_newest(
        &self,
        session: &str,
        version: &str,
        newest: &Value,
        fits: &mut (dyn FnMut(&Value) -> bool + Send),
    ) -> Option<NewestPageFit> {
        self.app
            .reader
            .trimmed_newest(session, version, newest, fits)
    }

    fn subscribe_changes(&self, _: &str) -> watch::Receiver<u64> {
        self.source
            .as_ref()
            .map_or_else(|| self.quiet.subscribe(), |source| source.changes())
    }

    fn readmit(&self) -> Result<Value, ReadError> {
        admit(self.binding.as_ref(), super::TRANSCRIPT_PAGE)?;
        Ok(self.params.clone())
    }

    async fn emit(&self, body: Vec<u8>) -> Result<(), ()> {
        let body = self.with_queued(body).await;
        self.emit_body(body).await
    }
    async fn stopped(&self) {
        self.stop().await;
    }
    fn is_stopped(&self) -> bool {
        *self.ended.borrow() || self.cancelled.is_cancelled()
    }
}

#[async_trait]
impl DisplayWatchHost for Watch {
    async fn read(&self, params: Value) -> Result<DisplayReadReply, DisplayReadError> {
        let request = DisplayReadRequest::parse(&params)?;
        Ok(request.read(self.app.display.projection(request.session).as_deref()))
    }
    fn projection(&self, session: &str) -> Option<Arc<DisplayProjection>> {
        self.app.display.projection(session)
    }
    fn projection_changes(&self, session: &str) -> watch::Receiver<u64> {
        self.app.display.changes(session)
    }
    fn readmit(&self) -> Result<(), DisplayReadError> {
        admit(self.binding.as_ref(), super::DISPLAY_READ)
            .map(|_| ())
            .map_err(|e| DisplayReadError {
                code: e.code,
                message: e.message,
            })
    }
    async fn emit(&self, body: Vec<u8>) -> Result<(), DisplayWatchStopped> {
        self.emit_body(body).await.map_err(|_| DisplayWatchStopped)
    }
    async fn stopped(&self) {
        self.stop().await;
    }
}
