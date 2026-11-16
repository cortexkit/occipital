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
//! stored once.

use std::sync::Arc;

use async_trait::async_trait;
use cortexkit_role_tool_display::{DisplayRead, WriteReply, DISPLAY_READ};
use serde_json::{json, Value};
use subc_protocol::{ErrorBody, Principal, RouteTarget};

use super::daemon::{CallFailure, DaemonCalls};
use super::{app, views, State};

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

/// Where a pulled record is stored: the display store and what a stored
/// write tells, as for a pushed one.
#[derive(Clone)]
pub struct Sink {
    pub state: Option<State>,
    pub views: Option<views::Fetcher>,
    pub hints: Option<tokio::sync::broadcast::Sender<app::Hint>>,
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
    match super::store_write(
        sink.state.as_ref(),
        sink.views.as_ref(),
        sink.hints.as_ref(),
        Some(writer),
        &reply,
    )
    .await
    {
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
}

impl DaemonProviders {
    pub fn new(calls: Arc<DaemonCalls>) -> Self {
        Self { calls }
    }
}

#[async_trait]
impl Providers for DaemonProviders {
    async fn verified(&self, module: &str) -> Result<bool, String> {
        let consumer = self
            .calls
            .consumer()
            .await
            .map_err(|failure| failure.to_string())?;
        match consumer.catalog_list().await {
            Ok(listed) => Ok(listed
                .modules
                .iter()
                .any(|entry| entry.module_id == module && entry.nonce_verified)),
            Err(error) => {
                self.calls.failed(error.code()).await;
                Err(error.to_string())
            }
        }
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
