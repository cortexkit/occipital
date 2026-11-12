//! The module the phone and desktop apps use for a chat, and the rich-UI reply teaching.
//!
//! `app_reader` serves the apps' transcript pages, live text, sends, Stop and
//! uploads for every session a runner hosts, reading each runner through the
//! llm-runner contract over the daemon. The tool-provider side serves the reply
//! teaching, with the optional `show_ui` tool.

#![forbid(unsafe_code)]
// Agent-written programs must produce typed failures rather than stop every caller.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented
    )
)]

#[cfg(not(panic = "unwind"))]
compile_error!("occipital-module requires panic = \"unwind\": its panic boundary does not work when panics abort");

pub mod app_reader;
pub mod backup;
pub mod base64;
pub mod display;
pub mod key_digest;
pub mod manifest;
pub mod media;
#[cfg(feature = "show-ui")]
pub mod native;
pub mod notices;
pub mod request_context;
pub mod serve;
#[cfg(feature = "show-ui")]
pub mod show_ui;
pub mod startup;
pub mod store;
pub mod system_text;
#[cfg(test)]
mod test_tracing;

pub use manifest::MODULE_ID;

/// The error type of the app reader's store and wire plumbing.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Milliseconds since the Unix epoch, saturating to 0 before it.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}
