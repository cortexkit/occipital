//! The C ABI that embeds occipital-lang in an app as a static library.
//!
//! The contract is the hand-written header `include/occipital.h`. Entry
//! points take NUL-terminated UTF-8 JSON strings, byte buffers or plain
//! integers, return an `int32_t` status, and hand their result back through an
//! `out` pointer as a Rust-owned JSON string or an opaque handle; the `_free`
//! functions return nothing. This crate only encodes, decodes and maps
//! statuses; checking, evaluation and layout all stay in occipital-lang, and
//! finding issue, pull request and `@NAME` mentions in cortexkit-occipital-refs.
//!
//! Every `extern "C"` function runs its body inside `catch_unwind`, so a panic
//! in the library never unwinds into C code (which is undefined behaviour) or
//! aborts the app: status-returning functions report it as `OCC_PANIC`, and
//! the `void` free functions discard it.

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
// The library runs inside apps, where a panic is at best an OCC_PANIC status
// and an abandoned session. Like occipital-lang, code outside tests may not
// panic on purpose: a broken assumption must become a status code.
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

// `catch_unwind` only catches panics that unwind. With `panic = "abort"` a
// panic would end the app before the boundary could report OCC_PANIC.
#[cfg(not(panic = "unwind"))]
compile_error!(
    "occipital-ffi requires panic = \"unwind\": its panic boundary does not work when panics abort"
);

mod account;
mod base64;
mod boundary;
mod decimal;
mod envelope;
mod fact;
#[cfg(any(test, feature = "test-hooks"))]
mod hooks;
mod measure;
mod metrics;
mod output;
mod program;
mod refs;
mod reply;
mod status;
mod strings;
#[cfg(test)]
mod testing;
mod tree;
#[cfg(test)]
mod vectors;
mod version;

pub use account::{occ_account_free, occ_account_new, OccAccount};
pub use fact::{occ_format_fact, occ_format_schedule};
#[cfg(any(test, feature = "test-hooks"))]
pub use hooks::{occ_test_decode_layout, occ_test_force_panic};
pub use measure::{occ_measure_block, occ_measure_tree};
pub use metrics::{LineHeight, OccMetrics, TextHeight, TextWidth};
pub use program::{occ_program_check, occ_template_evaluate};
pub use refs::occ_text_references;
pub use reply::{
    occ_reply_chunk, occ_reply_event, occ_reply_finish, occ_reply_free, occ_reply_measure,
    occ_reply_new, OccReply,
};
pub use status::{
    OCC_INTERNAL, OCC_INVALID_ARGUMENT, OCC_MEASURE_ERROR, OCC_OK, OCC_PANIC, OCC_REFUSED,
    OCC_REJECTED, OCC_SESSION_STATE,
};
pub use strings::occ_string_free;
pub use version::occ_version;
