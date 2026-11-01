//! Tool display data (`tool-display/v1`): what tool providers write for their
//! calls after answering them, kept for the apps' rows and pages.
//!
//! Providers write records, details and completions here; the app reader
//! serves them beside transcript pages, joined to calls only through views the
//! writer itself serves for the call's pin.

pub mod app;
pub mod call_pins;
pub mod daemon;
pub mod model_names;
pub mod store;
pub mod views;

use std::sync::Arc;

use cortexkit_role_tool_display::{check_writer, DisplayWrite, InvalidParams, WriteReply};
use serde::de::DeserializeOwned;
use serde_json::Value;
use subc_client_rs::HandlerOutcome;
use subc_protocol::{ErrorBody, Principal};

use store::DisplayStore;

/// The display store, once HELLO_ACK has named the module's data folder.
pub enum State {
    Open(Arc<DisplayStore>),
    /// The file couldn't be opened. Display data is never needed to serve a
    /// chat, so the module goes on without it and refuses writes.
    Failed(String),
}

/// Serve one `display.write` named request. `writer` is the route's attested
/// opener, `None` before the daemon stamped one; `arguments` are the request's.
/// The store write runs off the async workers, since a detail can be 3 MiB.
/// A newly stored write asks `views` for its writer's view of the pin, so the
/// view is usually kept before any app reads the call, and is announced to
/// `hints`, so a records watch on the call's session tells its app.
pub async fn write(
    state: Option<&State>,
    views: Option<&views::Fetcher>,
    hints: Option<&tokio::sync::broadcast::Sender<app::Hint>>,
    writer: Option<Principal>,
    arguments: &Value,
) -> HandlerOutcome {
    let store = match state {
        // Before HELLO_ACK: the contract's retryable refusal for a server that
        // isn't ready, which a provider may also skip.
        None => return refused(cortexkit_role_tool_display::not_ready()),
        Some(State::Failed(reason)) => {
            return refused(ErrorBody::new(
                "provider_unavailable",
                format!("display store unavailable: {reason}"),
            ))
        }
        Some(State::Open(store)) => store.clone(),
    };
    let writer = writer.unwrap_or(Principal::Unverified);
    if let Err(refusal) = check_writer(&writer) {
        return refused(refusal);
    }
    let write = match params(arguments) {
        Ok(write) => write,
        Err(refusal) => return refused(refusal),
    };
    let pin = write.schema_pin.clone();
    let call_key = write.call_key.clone();
    let runner = match &write.runner {
        Principal::Reserved { module_id } => Some(module_id.clone()),
        _ => None,
    };
    let module = match &writer {
        Principal::Reserved { module_id } => module_id.clone(),
        _ => String::new(),
    };
    let stored =
        tokio::task::spawn_blocking(move || store.write(&writer, &write, store::now_ms())).await;
    if let Ok(Ok(WriteReply::Stored)) = &stored {
        if let Some(views) = views {
            views.want(&module, &pin);
        }
        if let (Some(hints), Some(runner)) = (hints, runner) {
            // No watch open is no one to tell, not a failure.
            let _ = hints.send(app::Hint::Write { runner, call_key });
        }
    }
    match stored {
        Ok(Ok(reply)) => match serde_json::to_vec(&reply) {
            Ok(bytes) => HandlerOutcome::Response(bytes),
            Err(error) => refused(ErrorBody::new("internal", error.to_string())),
        },
        Ok(Err(refusal)) => refused(refusal),
        Err(error) => refused(ErrorBody::new("internal", error.to_string())),
    }
}

/// Decode the write's params one member at a time, so a refusal names the
/// first member that is missing or malformed, as `invalid_params` requires.
/// Unknown members are ignored.
fn params(arguments: &Value) -> Result<DisplayWrite, ErrorBody> {
    fn member<T: DeserializeOwned>(arguments: &Value, field: &str) -> Result<T, ErrorBody> {
        arguments
            .get(field)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .ok_or_else(|| InvalidParams::new(field).refusal())
    }
    Ok(DisplayWrite::new(
        member(arguments, "runner")?,
        member::<String>(arguments, "call_key")?,
        member(arguments, "kind")?,
        member::<String>(arguments, "schema_pin")?,
        member(arguments, "body")?,
    ))
}

fn refused(error: ErrorBody) -> HandlerOutcome {
    match error.detail {
        Some(detail) => HandlerOutcome::ErrorWithDetail {
            code: error.code,
            message: error.message,
            detail,
        },
        None => HandlerOutcome::Error {
            code: error.code,
            message: error.message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reserved(module: &str) -> Principal {
        Principal::Reserved {
            module_id: module.into(),
        }
    }

    /// `runner` is subc-protocol's `Principal` in its own serde form, written
    /// out here as literal bytes so a writer that hand-builds another spelling
    /// is caught: it is refused, naming the member.
    #[test]
    fn the_runner_is_decoded_as_subc_protocol_writes_a_principal() {
        let write = |runner: serde_json::Value| {
            params(&json!({
                "runner": runner,
                "call_key": "call-1",
                "kind": "record",
                "schema_pin": "tp1:bash:0123:1",
                "body": {"durationMs": 5, "outcome": "success"},
            }))
        };
        let decoded = write(json!({"kind": "reserved", "module_id": "broca"})).unwrap();
        assert_eq!(decoded.runner, reserved("broca"));
        let refused = write(json!({"reserved": {"module_id": "broca"}})).unwrap_err();
        assert_eq!(refused.code, "invalid_params");
        assert_eq!(refused.detail, Some(json!({"field": "runner"})));
    }

    /// Only a newly stored write asks for its writer's view of the pin and is
    /// announced to records watches: a repeat already was, and a refused write
    /// stored nothing.
    #[tokio::test]
    async fn a_newly_stored_write_asks_for_its_writer_s_view() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-display-wants");
        let descriptor = crate::store::tests::descriptor(&dir);
        let state = State::Open(Arc::new(
            store::DisplayStore::open(&descriptor, dir.path()).unwrap(),
        ));
        let (queue, mut wanted) = tokio::sync::mpsc::channel(8);
        let fetcher = views::Fetcher::from_queue(queue);
        let arguments = json!({
            "runner": serde_json::to_value(reserved("broca")).unwrap(),
            "call_key": "call-1",
            "kind": "record",
            "schema_pin": "tp1:bash:0123:1",
            "body": {"durationMs": 5, "outcome": "success"},
        });
        let (hints, mut announced) = tokio::sync::broadcast::channel(8);
        for writer in [reserved("aft"), reserved("aft"), Principal::Direct] {
            write(
                Some(&state),
                Some(&fetcher),
                Some(&hints),
                Some(writer),
                &arguments,
            )
            .await;
        }
        assert_eq!(
            wanted.try_recv().unwrap(),
            views::Want::Writer("aft".to_owned(), "tp1:bash:0123:1".to_owned())
        );
        assert!(wanted.try_recv().is_err());
        assert_eq!(
            announced.try_recv().unwrap(),
            app::Hint::Write {
                runner: "broca".into(),
                call_key: "call-1".into()
            }
        );
        assert!(announced.try_recv().is_err());
    }

    /// Asking an app for views asks the fetcher about each writer with a kept
    /// view or `unknown` for the pin, so one due again is fetched; a writer
    /// whose catalog didn't hold the pin is never asked.
    #[tokio::test]
    async fn reading_views_asks_about_each_kept_answer() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-display-view-wants");
        let descriptor = crate::store::tests::descriptor(&dir);
        let store = Arc::new(store::DisplayStore::open(&descriptor, dir.path()).unwrap());
        let pin = "tp1:bash:0123:1";
        for (writer, state) in [
            ("aft", store::ViewState::Unknown),
            ("core", store::ViewState::NotInCatalog),
        ] {
            store.keep_view(writer, pin, &state, "1.0", 1).unwrap();
        }
        let (queue, mut wanted) = tokio::sync::mpsc::channel(8);
        let (hints, _) = tokio::sync::broadcast::channel(8);
        let display = Arc::new(app::ToolDisplay::new(
            store,
            Some(views::Fetcher::from_queue(queue)),
            hints,
        ));
        display.views(&json!({"pins": [pin]})).await.unwrap();
        assert_eq!(
            wanted.try_recv().unwrap(),
            views::Want::Writer("aft".to_owned(), pin.to_owned())
        );
        assert!(wanted.try_recv().is_err());
    }
}
