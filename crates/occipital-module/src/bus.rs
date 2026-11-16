//! What Occipital reads from the local message bus (ck-bus, NATS).
//!
//! Occipital publishes nothing. It reads one durable, `m_occipital`, whose
//! filters `cortexkit-bus-naming` declares and ck-bus provisions. Each event is
//! only a wake: the fact itself is read again through the call that owns it,
//! so a missed, repeated or late event costs at most one extra read.
//!
//! - A runner's `send_state.v1.occipital` says one of this module's sends
//!   changed state. Its follower asks `session.send_status` at once
//!   ([`Sender::nudge`]) instead of at its next timed check, which stays as the
//!   backstop.
//! - Fusiform's `catalog_changed.v1` makes the model names held old
//!   ([`ModelNames::catalog_changed`]).
//! - A tool provider's `display_record_ready.v1` names a record it stored.
//!   This module reads it back with `display.read` and stores it
//!   ([`crate::display::pull`]). Providers still push each record by
//!   `display.write` too, until this reader has stored records pulled from
//!   them in production; a record that arrives both ways is stored once.
//!
//! A send-state or catalog event is acknowledged once its wake is handed on.
//! Nothing it wakes needs the event again: a followed send is stored, and after
//! a restart the sender's periodic sweep of stored sends asks the runner about
//! each one again (`Sender::start`). A record event is acknowledged
//! once its record is stored or refused for good, and delivered again later
//! when the provider or the store can't be reached. An event that can never be
//! read (a recognised subject with a body that doesn't decode, or a record the
//! client can't parse) is terminated, so it isn't redelivered forever.

use std::sync::Arc;
use std::time::Duration;

use cortexkit_bus_client::{BusClient, BusClientOptions, EventItem};
use cortexkit_role_tool_display::{
    DisplayRecordReady, DISPLAY_RECORD_READY_EVENT, DISPLAY_RECORD_READY_VERSION,
};

use crate::app_reader::send::Sender;
use crate::display::daemon::DaemonCalls;
use crate::display::model_names::ModelNames;
use crate::display::pull::{self, Pulled};

/// The durable this module reads.
pub const DURABLE: &str = "m_occipital";
/// The longest wait before connecting or attaching again after a failure.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// How long a record event waits before it is delivered again, when its
/// provider or the display store couldn't be reached.
const RECORD_RETRY: Duration = Duration::from_secs(30);

/// What one event asks of this module.
#[derive(Debug, PartialEq, Eq)]
pub enum Wake {
    /// A runner says one of this module's sends changed state.
    SendState { send_id: String },
    /// Fusiform's catalog changed.
    CatalogChanged,
    /// A tool provider stored the record `record_id`.
    ToolRecord { provider: String, record_id: String },
}

/// Why an event can never be read.
#[derive(Debug, PartialEq, Eq)]
pub struct Unreadable(pub String);

/// The wake for an event, from its publisher (`module`), name, version, reader
/// token and body. `Ok(None)` for an event this module doesn't read.
pub fn wake(
    module: &str,
    event: &str,
    version: u32,
    reader: Option<&str>,
    body: &[u8],
) -> Result<Option<Wake>, Unreadable> {
    if event == DISPLAY_RECORD_READY_EVENT
        && u64::from(version) == DISPLAY_RECORD_READY_VERSION
        && reader.is_none()
    {
        // Any provider may publish it. Its record is taken only from a module
        // the daemon has verified, which the pull checks.
        let ready: DisplayRecordReady = serde_json::from_slice(body)
            .map_err(|error| Unreadable(format!("{event} body: {error}")))?;
        ready
            .validate()
            .map_err(|_| Unreadable(format!("{event} body: a record_id that isn't an id")))?;
        return Ok(Some(Wake::ToolRecord {
            provider: module.to_owned(),
            record_id: ready.record_id,
        }));
    }
    match (module, event, version, reader) {
        ("thalamus-harness" | "broca", "send_state", 1, Some(crate::MODULE_ID)) => {
            #[derive(serde::Deserialize)]
            struct Notice {
                send_id: String,
            }
            let notice: Notice = serde_json::from_slice(body)
                .map_err(|error| Unreadable(format!("send_state body: {error}")))?;
            Ok(Some(Wake::SendState {
                send_id: notice.send_id,
            }))
        }
        ("fusiform", "catalog_changed", 1, None) => Ok(Some(Wake::CatalogChanged)),
        _ => Ok(None),
    }
}

/// Where the wakes go.
pub struct Handlers {
    pub sends: Arc<Sender>,
    pub model_names: Option<Arc<ModelNames>>,
    /// The providers a record is read from, and where it is stored.
    pub records: Option<(Arc<dyn pull::Providers>, pull::Sink)>,
}

/// What to do with an event once its wake is handed on.
#[derive(Debug, PartialEq, Eq)]
enum Done {
    Ack,
    /// Deliver it again after this long.
    Again(Duration),
}

impl Handlers {
    async fn hand_on(&self, wake: &Wake) -> Done {
        match wake {
            Wake::SendState { send_id } => {
                self.sends.nudge(send_id);
                Done::Ack
            }
            Wake::CatalogChanged => {
                if let Some(names) = &self.model_names {
                    names.catalog_changed();
                }
                Done::Ack
            }
            Wake::ToolRecord {
                provider,
                record_id,
            } => {
                let Some((providers, sink)) = &self.records else {
                    return Done::Again(RECORD_RETRY);
                };
                match pull::pull(providers.as_ref(), sink, provider, record_id).await {
                    Pulled::Stored => Done::Ack,
                    Pulled::Final(why) => {
                        tracing::warn!(%provider, %record_id, %why, "an announced tool record is not stored");
                        Done::Ack
                    }
                    Pulled::Retry(why) => {
                        tracing::debug!(%provider, %record_id, %why, "an announced tool record waits");
                        Done::Again(RECORD_RETRY)
                    }
                }
            }
        }
    }
}
/// Read the durable for as long as the module runs. One connection is kept: a
/// failed attach (the durable isn't provisioned yet, say) is retried on it, so
/// each retry doesn't take a fresh credential. Only a lost reader, or a
/// connection whose credential upkeep has stopped, connects again. Each attach
/// and each loss is logged at INFO, since nothing else shows whether the reader
/// is live; a failure to connect or attach is logged at INFO once per streak,
/// then at debug.
pub fn start(calls: Arc<DaemonCalls>, handlers: Handlers) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Backoff::default();
        loop {
            let connected = match calls.consumer().await {
                Ok(consumer) => match BusClientOptions::new(crate::MODULE_ID) {
                    Ok(options) => BusClient::connect(consumer, options)
                        .await
                        .map_err(|error| error.to_string()),
                    Err(error) => Err(error.to_string()),
                },
                Err(failure) => Err(failure.to_string()),
            };
            let bus = match connected {
                Ok(bus) => bus,
                Err(error) => {
                    backoff.failed("connect", error).await;
                    continue;
                }
            };
            let upkeep = bus.connection_changes();
            loop {
                let mut reader = match bus.event_reader(DURABLE).await {
                    Ok(reader) => reader,
                    Err(error) => {
                        backoff.failed("attach", error.to_string()).await;
                        // A closed change stream means the credential can no
                        // longer be renewed: connect again.
                        if upkeep.has_changed().is_err() {
                            break;
                        }
                        continue;
                    }
                };
                backoff = Backoff::default();
                tracing::info!(durable = DURABLE, "message bus reader attached");
                let reason = read(&mut reader, &handlers).await;
                tracing::info!(durable = DURABLE, %reason, "message bus reader lost");
                break;
            }
            backoff.wait().await;
        }
    })
}

/// Hand on each event until the reader fails, and say why it did.
async fn read(reader: &mut cortexkit_bus_client::EventReader, handlers: &Handlers) -> String {
    loop {
        let done = match reader.next().await {
            Ok(EventItem::Delivery(delivery)) => {
                let subject = &delivery.subject;
                match wake(
                    &subject.module,
                    &subject.event,
                    subject.version,
                    subject.reader.as_deref(),
                    &delivery.body,
                ) {
                    Ok(found) => {
                        let done = match &found {
                            Some(wake) => handlers.hand_on(wake).await,
                            None => Done::Ack,
                        };
                        match done {
                            Done::Ack => delivery.ack().await,
                            Done::Again(after) => delivery.nak(after).await,
                        }
                    }
                    Err(Unreadable(why)) => {
                        tracing::warn!(
                            module = %subject.module,
                            event = %subject.event,
                            %why,
                            "a message bus event can't be read"
                        );
                        delivery.term().await
                    }
                }
            }
            Ok(EventItem::Malformed(record)) => {
                tracing::warn!(
                    reason = ?record.reason,
                    "a malformed message bus record is terminated"
                );
                record.term().await
            }
            Ok(_) => Ok(()),
            Err(error) => return error.to_string(),
        };
        if let Err(error) = done {
            return error.to_string();
        }
    }
}

/// The wait between attempts, doubling to [`MAX_BACKOFF`], and whether this
/// streak of failures has been logged at INFO yet.
struct Backoff {
    wait: Duration,
    logged: bool,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            wait: Duration::from_secs(1),
            logged: false,
        }
    }
}

impl Backoff {
    async fn failed(&mut self, what: &str, error: String) {
        if self.logged {
            tracing::debug!(%error, "message bus {what} failed");
        } else {
            tracing::info!(%error, "message bus {what} failed");
            self.logged = true;
        }
        self.wait().await;
    }

    async fn wait(&mut self) {
        tokio::time::sleep(self.wait).await;
        self.wait = (self.wait * 2).min(MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_read_subject_wakes_what_it_names() {
        let notice = br#"{"v":1,"session_ref":"32db1a5342168526c10b5dedf01c1760","send_id":"app-7f","seq":4,"state":"finished"}"#;
        for runner in ["thalamus-harness", "broca"] {
            assert_eq!(
                wake(runner, "send_state", 1, Some("occipital"), notice),
                Ok(Some(Wake::SendState {
                    send_id: "app-7f".into()
                }))
            );
        }
        assert_eq!(
            wake(
                "fusiform",
                "catalog_changed",
                1,
                None,
                br#"{"catalog_version":12}"#
            ),
            Ok(Some(Wake::CatalogChanged))
        );
        for provider in ["aft", "cerebellum"] {
            assert_eq!(
                wake(
                    provider,
                    "display_record_ready",
                    1,
                    None,
                    br#"{"record_id":"r1"}"#
                ),
                Ok(Some(Wake::ToolRecord {
                    provider: provider.into(),
                    record_id: "r1".into()
                }))
            );
        }
        // Cerebellum's old `tool_display` event name is not read.
        assert_eq!(wake("cerebellum", "tool_display", 1, None, b"{}"), Ok(None));
    }

    /// The contract's record event body is `{record_id}` and nothing else.
    #[test]
    fn a_record_event_with_another_body_is_unreadable() {
        for body in [
            &br#"{}"#[..],
            br#"{"record_id":"r1","call_key":"c1"}"#,
            br#"{"record_id":"has space"}"#,
        ] {
            assert!(
                wake("aft", "display_record_ready", 1, None, body).is_err(),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        // Addressed to a reader, or another version: not this module's event.
        let body = br#"{"record_id":"r1"}"#;
        assert_eq!(
            wake("aft", "display_record_ready", 1, Some("occipital"), body),
            Ok(None)
        );
        assert_eq!(wake("aft", "display_record_ready", 2, None, body), Ok(None));
    }

    #[test]
    fn a_send_state_for_another_reader_or_version_wakes_nothing() {
        let notice = br#"{"send_id":"app-7f"}"#;
        assert_eq!(
            wake("broca", "send_state", 1, Some("aft"), notice),
            Ok(None)
        );
        assert_eq!(
            wake("broca", "send_state", 2, Some("occipital"), notice),
            Ok(None)
        );
        assert_eq!(wake("broca", "send_state", 1, None, notice), Ok(None));
        // Only the runners Occipital sends through.
        assert_eq!(
            wake("basal", "send_state", 1, Some("occipital"), notice),
            Ok(None)
        );
        assert_eq!(wake("broca", "run.ended", 1, None, b"{}"), Ok(None));
    }

    #[test]
    fn a_send_state_whose_body_doesnt_decode_is_unreadable() {
        assert!(wake("broca", "send_state", 1, Some("occipital"), b"{}").is_err());
        assert!(wake("broca", "send_state", 1, Some("occipital"), b"not json").is_err());
    }
}
