//! Records a tool provider announces on the message bus instead of pushing
//! them with `display.write`.
//!
//! After a record commits, its provider publishes `display_record_ready.v1`
//! naming only the record's id. This module reads the record back from that
//! provider with `display.read`, which answers the exact `display.write` params
//! it would have pushed, and stores it through the same write path. The writer
//! is the module this module called, and only when the daemon's catalog lists
//! it as daemon-verified, so a pulled record carries the same proof of its
//! writer as a pushed one (`check_writer`). A record pushed and pulled is
//! stored once. A pulled write goes where the same write pushed would: a
//! wake record to the notices file, every other kind to the display store.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use cortexkit_role_tool_display::{DisplayRead, WriteReply, DISPLAY_READ};
use serde_json::{json, Value};
use subc_protocol::{ErrorBody, Principal, RouteTarget};

use super::daemon::{CallFailure, DaemonCalls};
use super::{app, views, State};
use crate::catalog_generation::{CatalogGeneration, Checked};

/// What became of one announced record, which says what to do with its event.
#[derive(Debug, PartialEq, Eq)]
pub enum Pulled {
    /// Stored now, or already stored: the event is done with.
    Stored,
    /// It can never be stored: the provider refused it for good, or what it
    /// answered can't be stored. The event is done with too.
    Final(String),
    /// Not this time: the provider or the store couldn't be reached. The event
    /// is delivered again later.
    Retry(String),
}

/// The providers, as the pull sees them.
#[async_trait]
pub trait Providers: Send + Sync {
    /// Whether the daemon's catalog lists `module` as daemon-verified (it
    /// proved its launch nonce or registration claim).
    async fn verified(&self, module: &str) -> Result<bool, String>;
    /// `module`'s `display.read` reply for `record_id`.
    async fn read(&self, module: &str, record_id: &str) -> Result<Value, CallFailure>;
}

/// Where a pulled record is stored, as a pushed one would be: the display
/// store, with the views to fetch and the hints to send once it's stored, or
/// the notices file for a wake record.
#[derive(Clone)]
pub struct Sink {
    pub state: Option<State>,
    pub views: Option<views::Fetcher>,
    pub hints: Option<tokio::sync::broadcast::Sender<app::Hint>>,
    pub notices: Option<Arc<crate::notices::Notices>>,
}

/// Route-open and call failures that may clear on their own. Every other
/// refusal is final, the contract's `display_read_denied`,
/// `display_record_unknown` and `invalid_params` among them: a read changes
/// nothing, but redelivering an event the provider will always refuse would
/// only repeat the refusal.
const RETRYABLE: [&str; 6] = [
    cortexkit_role_tool_display::NOT_READY,
    "module_reloading",
    "target_unavailable",
    "module_timeout",
    "route_busy",
    "unknown_channel",
];

/// Read `record_id` from `module` and store it.
pub async fn pull(providers: &dyn Providers, sink: &Sink, module: &str, record_id: &str) -> Pulled {
    match providers.verified(module).await {
        Ok(true) => {}
        Ok(false) => {
            return Pulled::Final(format!(
                "{module} is not daemon-verified, so its records aren't taken"
            ))
        }
        Err(error) => return Pulled::Retry(format!("catalog.list: {error}")),
    }
    let reply = match providers.read(module, record_id).await {
        Ok(reply) => reply,
        Err(failure) => {
            let said = format!("{module}: {DISPLAY_READ}: {failure}");
            return match failure.code.as_deref() {
                None => Pulled::Retry(said),
                Some(code) if RETRYABLE.contains(&code) => Pulled::Retry(said),
                Some(_) => Pulled::Final(said),
            };
        }
    };
    let writer = Principal::Reserved {
        module_id: module.to_owned(),
    };
    let stored = if reply.get("kind").and_then(Value::as_str) == Some("wake") {
        crate::notices::wakes::store(sink.notices.as_ref(), Some(writer), &reply).await
    } else {
        super::store_write(
            sink.state.as_ref(),
            sink.views.as_ref(),
            sink.hints.as_ref(),
            Some(writer),
            &reply,
        )
        .await
    };
    match stored {
        Ok(WriteReply::Stored | WriteReply::AlreadyStored) => Pulled::Stored,
        Ok(other) => Pulled::Final(format!("{module}: the store answered {other:?}")),
        Err(ErrorBody { code, message, .. }) => {
            let said = format!("{module}: storing {record_id}: {code}: {message}");
            // The store isn't open yet, or can't be: the record waits. Anything
            // else is a refusal of what the provider answered.
            if code == cortexkit_role_tool_display::NOT_READY || code == "provider_unavailable" {
                Pulled::Retry(said)
            } else {
                Pulled::Final(said)
            }
        }
    }
}

/// The providers, reached through the daemon as this module.
pub struct DaemonProviders {
    calls: Arc<DaemonCalls>,
    verified: VerifiedModules,
}

impl DaemonProviders {
    pub fn new(calls: Arc<DaemonCalls>) -> Self {
        Self {
            calls,
            verified: VerifiedModules::default(),
        }
    }
}

/// The daemon-verified modules as the last catalog read listed them. A
/// provider announces every record it writes, so reading the catalog per
/// record would cost the daemon a full catalog per tool call; the list is
/// read again only once the catalog's generation moves.
#[derive(Default)]
struct VerifiedModules(tokio::sync::Mutex<Option<(Checked, BTreeSet<String>)>>);

impl VerifiedModules {
    /// Whether `module` is verified, from the kept list while `generation`
    /// says it is fresh, else from `read`, a new catalog read.
    async fn contains<Read>(
        &self,
        module: &str,
        generation: &CatalogGeneration,
        read: Read,
    ) -> Result<bool, String>
    where
        Read: Future<Output = Result<BTreeSet<String>, String>>,
    {
        let mut kept = self.0.lock().await;
        if let Some((checked, modules)) = kept.as_ref() {
            if generation.fresh(Some(checked)) {
                return Ok(modules.contains(module));
            }
        }
        let checked = generation.check();
        let modules = read.await?;
        let verified = modules.contains(module);
        *kept = Some((checked, modules));
        Ok(verified)
    }
}

#[async_trait]
impl Providers for DaemonProviders {
    async fn verified(&self, module: &str) -> Result<bool, String> {
        let read = async {
            let consumer = self
                .calls
                .consumer()
                .await
                .map_err(|failure| failure.to_string())?;
            match consumer.catalog_list().await {
                Ok(listed) => Ok(listed
                    .modules
                    .into_iter()
                    .filter(|entry| entry.nonce_verified)
                    .map(|entry| entry.module_id)
                    .collect()),
                Err(error) => {
                    self.calls.failed(error.code()).await;
                    Err(error.to_string())
                }
            }
        };
        self.verified
            .contains(module, &self.calls.catalog_generation(), read)
            .await
    }

    async fn read(&self, module: &str, record_id: &str) -> Result<Value, CallFailure> {
        self.calls
            .call(
                RouteTarget::ManagementSurface {
                    module_id: module.to_owned(),
                },
                "display-read",
                &json!({"name": DISPLAY_READ, "arguments": DisplayRead::new(record_id)}),
                None,
            )
            .await
    }
}

#[cfg(test)]
mod tests;
