//! Slash command subsystem.
//!
//! Decomposed from `src/main.rs` (TASK-AGS-621) so the slash-command
//! pipeline can be unit-tested in isolation. This module currently
//! ships only the parser. Registry (TASK-AGS-622) and dispatch
//! (TASK-AGS-623) land in later tasks.
//!
//! Declared as `mod command;` from `main.rs` so that `pub(crate)`
//! visibility scopes to the binary crate (not the library target).

pub(crate) mod agent;
pub(crate) mod background;
pub(crate) mod config;
pub(crate) mod dispatcher;
pub(crate) mod doctor;
pub(crate) mod ide_stdio;
pub(crate) mod login;
pub(crate) mod memory;
pub(crate) mod parser;
pub(crate) mod pipeline;
pub(crate) mod remote;
pub(crate) mod plugin;
pub(crate) mod registry;
pub(crate) mod sessions;
pub(crate) mod slash;
pub(crate) mod team;
pub(crate) mod task;
pub(crate) mod tui_helpers;
pub(crate) mod update;
pub(crate) mod utils;
pub(crate) mod web;
