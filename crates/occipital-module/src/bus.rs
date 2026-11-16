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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cortexkit_bus_client::{BucketItem, BusClient, BusClientOptions, EventItem};
use cortexkit_role_tool_display::{
    DisplayRecordReady, DISPLAY_RECORD_READY_EVENT, DISPLAY_RECORD_READY_VERSION,
};
use cortexkit_role_tool_provider::views::ViewsReply;

use crate::app_reader::send::Sender;
use crate::display::daemon::DaemonCalls;
use crate::display::model_names::ModelNames;
use crate::display::pull::{self, Pulled};
use crate::display::views::Fetcher;

/// The durable this module reads.
pub const DURABLE: &str = "m_occipital";
/// The name of a tool provider's bucket of `tool.views` answers.
const VIEWS: &str = "views";
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
    /// Where views a provider puts in its views bucket are kept.
    pub views: Option<Fetcher>,
    /// What the reader has done, for the module's health report.
    pub stats: Arc<Stats>,
}

/// What the reader has done since the module started, counted for the health
/// report: a wake is logged at debug only, so these are how an operator sees
/// that events arrive and are handled.
#[derive(Default)]
pub struct Stats {
    attached: AtomicBool,
    send_states: AtomicU64,
    catalog_changes: AtomicU64,
    records_stored: AtomicU64,
    records_refused: AtomicU64,
    records_again: AtomicU64,
    views_served: AtomicU64,
    unreadable: AtomicU64,
}

impl Stats {
    /// The counts as the health report's `bus` member.
    pub fn metrics(&self) -> serde_json::Value {
        let count = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        serde_json::json!({
            "readerAttached": self.attached.load(Ordering::Relaxed),
            "sendStates": count(&self.send_states),
            "catalogChanges": count(&self.catalog_changes),
            "recordsStored": count(&self.records_stored),
            "recordsRefused": count(&self.records_refused),
            "recordsRetried": count(&self.records_again),
            "viewsServed": count(&self.views_served),
            "unreadable": count(&self.unreadable),
        })
    }

    fn add(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
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
                Stats::add(&self.stats.send_states);
                self.sends.nudge(send_id);
                Done::Ack
            }
            Wake::CatalogChanged => {
                Stats::add(&self.stats.catalog_changes);
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
                    Pulled::Stored => {
                        Stats::add(&self.stats.records_stored);
                        Done::Ack
                    }
                    Pulled::Final(why) => {
                        Stats::add(&self.stats.records_refused);
                        tracing::warn!(%provider, %record_id, %why, "an announced tool record is not stored");
                        Done::Ack
                    }
                    Pulled::Retry(why) => {
                        Stats::add(&self.stats.records_again);
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
                Ok(bus) => Arc::new(bus),
                Err(error) => {
                    backoff.failed("connect", error).await;
                    continue;
                }
            };
            let upkeep = bus.connection_changes();
            // The view watches live as long as this connection: dropping the
            // set when it is replaced ends them.
            let mut watches = tokio::task::JoinSet::new();
            if let Some(fetcher) = &handlers.views {
                for owner in view_buckets() {
                    let (bus, fetcher, stats) =
                        (bus.clone(), fetcher.clone(), handlers.stats.clone());
                    watches.spawn(async move { watch_views(&bus, owner, &fetcher, &stats).await });
                }
            }
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
                handlers.stats.attached.store(true, Ordering::Relaxed);
                let reason = read(&mut reader, &handlers).await;
                handlers.stats.attached.store(false, Ordering::Relaxed);
                tracing::info!(durable = DURABLE, %reason, "message bus reader lost");
                break;
            }
            backoff.wait().await;
        }
    })
}

/// The modules whose views bucket this module reads, as bus naming declares
/// them: every `views` bucket that names this module as a reader.
fn view_buckets() -> impl Iterator<Item = &'static str> {
    cortexkit_bus_naming::MODULE_BUCKETS
        .iter()
        .filter(|bucket| bucket.name == VIEWS && bucket.readers.contains(&crate::MODULE_ID))
        .map(|bucket| bucket.owner)
}

/// Watch `owner`'s views bucket for as long as the connection lasts, offering
/// each view it puts to the fetcher, which keeps it as if fetched. A watch that
/// fails is opened again after a pause; until then, and for any view not in
/// the bucket, the fetcher still asks the writer with `tool.views`.
async fn watch_views(bus: &BusClient, owner: &'static str, fetcher: &Fetcher, stats: &Stats) {
    let mut backoff = Backoff::default();
    loop {
        let reason = match bus.bucket(owner, VIEWS) {
            Ok(bucket) => match bucket.watch().await {
                Ok(mut watch) => {
                    backoff = Backoff::default();
                    tracing::info!(%owner, "message bus views watch opened");
                    loop {
                        match watch.next().await {
                            Ok(BucketItem::Put { key, value, .. }) => {
                                match served_view(&key, &value) {
                                    Ok((pin, reply)) => {
                                        Stats::add(&stats.views_served);
                                        fetcher.offer(owner, &pin, reply).await
                                    }
                                    Err(why) => {
                                        tracing::warn!(%owner, %key, %why, "a served display view can't be read")
                                    }
                                }
                            }
                            // A provider keeps serving every pin it ever offered, so a
                            // deleted key changes nothing kept here.
                            Ok(_) => {}
                            Err(error) => break error.to_string(),
                        }
                    }
                }
                Err(error) => error.to_string(),
            },
            Err(error) => error.to_string(),
        };
        backoff
            .failed(&format!("{owner} views watch"), reason)
            .await;
    }
}

/// The pin and `tool.views` answer of one views-bucket entry. The key is the
/// lowercase hex of the pin's UTF-8 bytes, since a bucket key can't hold the
/// pin's colons; the value is the writer's `tool.views` reply for that pin.
fn served_view(key: &str, value: &[u8]) -> Result<(String, ViewsReply), String> {
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let bytes = key.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
        return Err("the key isn't lowercase hex".into());
    }
    let pin = bytes
        .chunks(2)
        .map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?))
        .collect::<Option<Vec<u8>>>()
        .ok_or("the key isn't lowercase hex")?;
    let pin = String::from_utf8(pin).map_err(|_| "the key's pin isn't UTF-8")?;
    let reply = serde_json::from_slice(value).map_err(|error| format!("the value: {error}"))?;
    Ok((pin, reply))
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
                        Stats::add(&handlers.stats.unreadable);
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
                Stats::add(&handlers.stats.unreadable);
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

    /// Broca's send-state notices as its publisher sends them, copied unchanged
    /// (`tests/data/broca-send-state-1116/`): each subject goes through the bus
    /// naming parser the reader uses, then `wake`. Occipital's four notices each
    /// give `Wake::SendState` for their `send_id`, whatever the state hint says;
    /// AFT's four, under AFT's reader token, give no wake.
    #[test]
    fn broca_send_state_notices_wake_occipitals_sends_only() {
        let data = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/broca-send-state-1116");
        let sums = std::fs::read_to_string(data.join("SHA256SUMS")).unwrap();
        let names = cortexkit_bus_naming::AccountNames::derive("box_test").unwrap();
        let mut woken = 0;
        for line in sums.lines() {
            let (expected, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(data.join(name)).unwrap();
            assert_eq!(
                format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes)),
                expected,
                "{name}"
            );
            if !name.ends_with(".json") {
                continue;
            }
            let notice: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let subject = names
                .parse_event_subject(notice["subject"].as_str().unwrap())
                .unwrap();
            let got = wake(
                &subject.module,
                &subject.event,
                subject.version,
                subject.reader.as_deref(),
                notice["body"].as_str().unwrap().as_bytes(),
            );
            if name.contains("-occipital-") {
                assert_eq!(
                    got,
                    Ok(Some(Wake::SendState {
                        send_id: "app-send-7".into()
                    })),
                    "{name}"
                );
                woken += 1;
            } else {
                assert_eq!(got, Ok(None), "{name}");
            }
        }
        assert_eq!(woken, 4);
    }

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

    /// The health report carries every count under its own name.
    #[test]
    fn the_counts_reach_the_health_report() {
        let stats = Stats::default();
        stats.attached.store(true, Ordering::Relaxed);
        Stats::add(&stats.records_stored);
        Stats::add(&stats.records_stored);
        Stats::add(&stats.send_states);
        assert_eq!(
            stats.metrics(),
            serde_json::json!({"readerAttached": true, "sendStates": 1, "catalogChanges": 0,
                "recordsStored": 2, "recordsRefused": 0, "recordsRetried": 0,
                "viewsServed": 0, "unreadable": 0})
        );
    }

    /// Bus naming declares AFT's and Cerebellum's views buckets with this
    /// module as a reader, so both are watched.
    #[test]
    fn the_declared_views_buckets_are_watched() {
        let mut owners: Vec<&str> = view_buckets().collect();
        owners.sort_unstable();
        assert_eq!(owners, ["aft", "cerebellum"]);
    }

    /// A views-bucket key is the pin's UTF-8 bytes in lowercase hex, and its
    /// value the writer's `tool.views` reply.
    #[test]
    fn a_views_bucket_entry_decodes_to_its_pin_and_reply() {
        let pin = "tp1:bash:0123:1";
        let key: String = pin.bytes().map(|byte| format!("{byte:02x}")).collect();
        let value = serde_json::to_vec(&serde_json::json!({"views": {pin: "unknown"}})).unwrap();
        let (decoded, reply) = served_view(&key, &value).unwrap();
        assert_eq!(decoded, pin);
        assert_eq!(reply.views.len(), 1);
        for bad in ["", "7", "74P0", &key.to_uppercase(), "ff"] {
            assert!(served_view(bad, &value).is_err(), "{bad:?}");
        }
        assert!(served_view(&key, b"not json").is_err());
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
