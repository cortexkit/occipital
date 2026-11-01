//! The tool template contract shared by the Occipital module, tool providers
//! and the apps: the `occipital.template/1` document ([`Template`]), its
//! canonical bytes and digest, so a template's digest never differs between
//! the person's computer and their phone, and the checks a tool's view must
//! pass ([`view`]), which the `occipital-check` command runs for providers.
//!
//! The library is pure Rust and performs no I/O.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
// The apps link this crate and feed it views from other modules. A panic
// there crashes the person's app, so a broken internal assumption must
// become a typed error the caller can handle. Library code may not panic.
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

pub mod canon;
pub mod digest;
mod document;
pub mod error;
pub mod grammar;
pub mod model;
mod rules;
mod shape;
pub mod view;
mod walk;

pub use canon::{canonicalize, CanonError, CanonErrorKind};
pub use digest::sha256_digest;
pub use error::{RegistryError, RuleCode};
pub use model::Template;

/// The README's example, run as a doc test so the published one can't rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeExample;
