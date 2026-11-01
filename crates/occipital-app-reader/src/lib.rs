//! Transport-independent session reads for the phone and desktop apps: transcript pages and their
//! claims, activity and change signals, and the live display of a turn in progress.

pub mod activity;
pub mod broca_entries;
pub mod claim;
pub mod details;
pub mod display;
pub mod display_events;
pub mod display_watch;
pub mod opencode_entries;
pub mod page;
pub mod send;
pub mod signals;
pub mod source;
pub mod tool_input;
pub mod tool_times;
pub mod transcript_watch;
pub mod watches;
