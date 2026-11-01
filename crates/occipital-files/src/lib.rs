//! File-system calls the standard library doesn't offer on every platform,
//! kept apart so the module crate can forbid `unsafe` code. Each call is a thin
//! wrapper over one operating-system function, on Windows, Linux and macOS.

pub mod files;
pub mod space;

pub use files::{identity, move_new};
pub use space::available_bytes;
