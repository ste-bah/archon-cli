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
pub(crate) mod bug;
pub(crate) mod color;
pub(crate) mod config;
pub(crate) mod context;
pub(crate) mod context_cmd;
pub(crate) mod cost;
pub(crate) mod denials;
pub(crate) mod diff;
pub(crate) mod dispatcher;
pub(crate) mod doctor;
pub(crate) mod errors;
pub(crate) mod export;
pub(crate) mod fast;
pub(crate) mod fork;
pub(crate) mod help;
pub(crate) mod hooks;
pub(crate) mod ide_stdio;
pub(crate) mod login;
pub(crate) mod mcp;
pub(crate) mod memory;
pub(crate) mod model;
pub(crate) mod parser;
pub(crate) mod pipeline;
pub(crate) mod remote;
pub(crate) mod plugin;
pub(crate) mod registry;
pub(crate) mod release_notes;
pub(crate) mod resume;
pub(crate) mod sessions;
pub(crate) mod slash;
pub(crate) mod status;
pub(crate) mod team;
pub(crate) mod task;
pub(crate) mod theme;
pub(crate) mod thinking;
pub(crate) mod vim;
pub(crate) mod tui_helpers;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod update;
pub(crate) mod utils;
pub(crate) mod voice;
pub(crate) mod web;

// TASK-AGS-800 (Stage 6, Q1=A): spec-name discoverability shim.
//
// The phase-8 spec (`TASK-AGS-800.md`) used the name `SlashCommand` for
// the trait. Shipped code (TASK-AGS-622) calls it `CommandHandler`.
// Stage 6 orchestrator decision Q1=A preserves the shipped trait
// verbatim (sync, `anyhow::Result<()>`, no `CommandOutcome`/`CommandError`/
// `ViewId` enums, no `inventory` registration). This re-export is a
// zero-cost namespace alias so future readers grepping for
// `SlashCommand` land on the real trait.
//
// Purely additive: no runtime behavior change, no new dependencies, no
// new types. See the TASK-AGS-800 commit body for the full R-item list.
#[allow(unused_imports)]
pub(crate) use registry::CommandHandler as SlashCommand;

// TASK-AGS-801 (Stage 6, Q1=A): parser drift-reconcile + gap-fill.
//
// Re-export the parser types so future readers grepping for
// `CommandParser` / `ParseError` / `Arg` / `suggest` land on the real
// definitions without having to dig through the `parser` submodule
// directly. This matches the additive-shim pattern established by the
// `SlashCommand` alias above and is the `mod.rs` re-export mandated by
// TASK-AGS-801 (G9).
//
// Note: `ParsedCommand` is already reachable via `parser::ParsedCommand`
// from dispatcher.rs; the re-export just widens the surface to match
// the spec's "mod.rs re-exports the 5 parser types" wiring check.
#[allow(unused_imports)]
pub(crate) use parser::{suggest, Arg, CommandParser, ParseError, ParsedCommand};
