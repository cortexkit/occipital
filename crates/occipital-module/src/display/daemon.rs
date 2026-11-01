//! Calls to other modules through the daemon, as this module, on one
//! connection. The connection is made on the first call, so startup never
//! waits on it, and made again after a call finds it closed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

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

    /// Send `body` to `target` and decode its reply. `purpose` names this
    /// module's reason in the callee's logs; callees find nothing by it.
    pub async fn call(
        &self,
        target: RouteTarget,
        purpose: &str,
        body: &Value,
        role_versions: Option<BTreeMap<String, String>>,
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
