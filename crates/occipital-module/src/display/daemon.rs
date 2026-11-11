//! Calls to other modules through the daemon, as this module, on one
//! connection. The connection is made on the first call, so startup never
//! waits on it, and made again after a call finds it closed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use subc_client_rs::{CallOptions, ConsumerIdentity, ConsumerOptions, SubcConsumer};
use subc_protocol::{BindIdentity, RouteTarget};

/// Why a call failed. `code` is the callee's refusal code; a failure without
/// one never reached the callee, or its reply was lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFailure {
    pub code: Option<String>,
    pub message: String,
}

impl std::fmt::Display for CallFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub struct DaemonCalls {
    connection_file: PathBuf,
    principal: ConsumerIdentity,
    consumer: tokio::sync::Mutex<Option<Arc<SubcConsumer>>>,
}

impl DaemonCalls {
    pub fn new(connection_file: PathBuf, principal: ConsumerIdentity) -> Self {
        Self {
            connection_file,
            principal,
            consumer: tokio::sync::Mutex::new(None),
        }
    }

    /// The connection, made now if there is none.
    pub async fn consumer(&self) -> Result<Arc<SubcConsumer>, CallFailure> {
        let mut held = self.consumer.lock().await;
        if let Some(consumer) = held.as_ref() {
            return Ok(consumer.clone());
        }
        let consumer = Arc::new(
            SubcConsumer::connect(&self.connection_file, ConsumerOptions::default())
                .await
                .map_err(|error| CallFailure {
                    code: None,
                    message: error.to_string(),
                })?,
        );
        *held = Some(consumer.clone());
        Ok(consumer)
    }

    /// Forget the connection after a failure that wasn't a refusal: it may
    /// have closed, and the next call connects again. A refusal leaves it
    /// usable.
    pub async fn failed(&self, code: Option<&str>) {
        if code.is_none() {
            *self.consumer.lock().await = None;
        }
    }

    /// The one module the daemon's catalog lists as providing `capability`,
    /// whatever its module id. None listed fails `capability_unprovided`, and
    /// more than one `capability_ambiguous`.
    pub async fn provider(&self, capability: &str) -> Result<String, CallFailure> {
        let consumer = self.consumer().await?;
        match consumer.resolve_provider(capability).await {
            Ok(module) => Ok(module),
            Err(error) => {
                let code = error.code().map(str::to_owned);
                self.failed(code.as_deref()).await;
                Err(CallFailure {
                    code,
                    message: error.to_string(),
                })
            }
        }
    }

    /// Send `body` to `target` and decode its reply. `purpose` names this
    /// module's reason in the callee's logs; callees find nothing by it.
    pub async fn call(
        &self,
        target: RouteTarget,
        purpose: &str,
        body: &Value,
        role_versions: Option<BTreeMap<String, String>>,
    ) -> Result<Value, CallFailure> {
        let open_within = CallOptions::default().route_retry_deadline;
        self.call_within(target, purpose, body, role_versions, open_within)
            .await
    }

    /// [`Self::call`], giving the route at most `open_within` to open. A
    /// target no module is registered as is refused at once whatever the time;
    /// one that is stopped or warming is asked again until the time is up, and
    /// the call then fails with that refusal. The time must leave room for
    /// one attempt: none at all means no open is tried.
    pub async fn call_within(
        &self,
        target: RouteTarget,
        purpose: &str,
        body: &Value,
        role_versions: Option<BTreeMap<String, String>>,
        open_within: Duration,
    ) -> Result<Value, CallFailure> {
        let consumer = self.consumer().await?;
        let called = consumer
            .call(
                target,
                BindIdentity::new(crate::MODULE_ID, purpose),
                body.to_string().into_bytes(),
                CallOptions {
                    consumer_identity: Some(self.principal.clone()),
                    role_versions,
                    route_retry_deadline: open_within,
                    ..Default::default()
                },
            )
            .await;
        let bytes = match called {
            Ok(bytes) => bytes,
            Err(error) => {
                let code = error.code().map(str::to_owned);
                self.failed(code.as_deref()).await;
                return Err(CallFailure {
                    code,
                    message: error.to_string(),
                });
            }
        };
        serde_json::from_slice(&bytes).map_err(|error| CallFailure {
            code: None,
            message: format!("the reply didn't parse: {error}"),
        })
    }
}
