//! Slash command subsystem.
//!
//! Decomposed from `src/main.rs` (TASK-AGS-621) so the slash-command
//! pipeline can be unit-tested in isolation. This module currently
//! ships only the parser. Registry (TASK-AGS-622) and dispatch
//! (TASK-AGS-623) land in later tasks.
//!
//! Declared as `mod command;` from `main.rs` so that `pub(crate)`
//! visibility scopes to the binary crate (not the library target).

pub(crate) mod dispatcher;
pub(crate) mod parser;
pub(crate) mod registry;
