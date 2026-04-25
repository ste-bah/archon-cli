//! Slash command subsystem.
//!
//! Decomposed from `src/main.rs` (TASK-AGS-621) so the slash-command
//! pipeline can be unit-tested in isolation. This module currently
//! ships only the parser. Registry (TASK-AGS-622) and dispatch
//! (TASK-AGS-623) land in later tasks.
//!
//! Declared as `mod command;` from `main.rs` so that `pub(crate)`
//! visibility scopes to the binary crate (not the library target).

pub(crate) mod add_dir;
pub(crate) mod agent;
pub(crate) mod background;
pub(crate) mod bug;
pub(crate) mod cancel;
pub(crate) mod checkpoint;
pub(crate) mod clear;
pub(crate) mod color;
// TASK-TUI-624: /commit AI git-commit prompt builder.
pub(crate) mod commit;
pub(crate) mod compact;
pub(crate) mod config;
pub(crate) mod context;
pub(crate) mod context_cmd;
pub(crate) mod copy;
pub(crate) mod cost;
pub(crate) mod denials;
pub(crate) mod diff;
pub(crate) mod dispatcher;
pub(crate) mod doctor;
pub(crate) mod effort;
pub(crate) mod errors;
// TASK-#206 SLASH-EXIT: /exit handler + /q alias.
pub(crate) mod exit;
pub(crate) mod export;
pub(crate) mod fast;
pub(crate) mod fork;
pub(crate) mod garden;
pub(crate) mod help;
pub(crate) mod hooks;
pub(crate) mod ide_stdio;
pub(crate) mod login;
pub(crate) mod logout;
pub(crate) mod mcp;
pub(crate) mod memory;
pub(crate) mod model;
pub(crate) mod parser;
pub(crate) mod permissions;
pub(crate) mod pipeline;
// TASK-TUI-626: /plan Plan Mode toggle via SNAPSHOT+EFFECT pattern.
pub(crate) mod plan;
// TASK-P0-B.3 (#174): plan-file I/O shim (re-exports from archon_core).
pub(crate) mod plan_file;
pub(crate) mod plugin;
pub(crate) mod recall;
pub(crate) mod registry;
pub(crate) mod release_notes;
pub(crate) mod reload;
pub(crate) mod remote;
pub(crate) mod rename;
pub(crate) mod resume;
// TASK-TUI-622: /review PR code-review prompt builder.
pub(crate) mod review;
// TASK-TUI-620: /rewind message-selector overlay launcher.
pub(crate) mod rewind;
pub(crate) mod rules;
// TASK-TUI-628: /sandbox handler — Bubble-mode flag flipper.
pub(crate) mod sandbox;
pub(crate) mod sessions;
// TASK-TUI-625: /session remote-URL + QR code handler.
pub(crate) mod session;
// TASK-TUI-627: /skills skills-menu overlay launcher.
pub(crate) mod skills;
pub(crate) mod slash;
pub(crate) mod status;
pub(crate) mod task;
pub(crate) mod team;
// TASK-TUI-623: /tag session tag toggle.
pub(crate) mod tag;
// TASK-TUI-621: hidden stub `/teleport` command (no is_visible() on
// trait — visibility handled by omission from archon-tui commands.rs).
pub(crate) mod teleport;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod theme;
pub(crate) mod thinking;
pub(crate) mod tui_helpers;
pub(crate) mod update;
pub(crate) mod usage;
pub(crate) mod utils;
pub(crate) mod vim;
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
pub(crate) use parser::{Arg, CommandParser, ParseError, ParsedCommand, suggest};
