//! Slash command dispatcher.
//!
//! TASK-AGS-623: ties parser + registry together. PATH A (hybrid):
//! the dispatcher acts as a gate at the top of `handle_slash_command`,
//! parsing input, looking up the handler in the registry, calling
//! `handler.execute` (currently a no-op stub from TASK-AGS-622), and
//! emitting "Unknown command: /{name}" via the TUI event channel for
//! unrecognized names. The legacy inline match in `main.rs` continues
//! to perform the actual command bodies until a future task migrates
//! handler bodies into the registry's stub `execute` methods.
//!
//! Spec note: TASK-AGS-623 originally targeted
//! `src/tui/input/keyboard.rs`, but that file does not exist in this
//! codebase — the slash-command match is inline in
//! `src/main.rs::handle_slash_command`. PATH A (approved) skips the
//! keyboard.rs migration and installs this dispatcher as a parallel
//! gate at the top of `handle_slash_command`. The legacy 43-arm match
//! remains intact and is untouched by this task.

use std::sync::Arc;

use crate::command::errors;
use crate::command::parser::{CommandParser, ParseError};
use crate::command::registry::{CommandContext, Registry};

/// Slash command dispatcher.
///
/// Owns a shared reference to the command [`Registry`]. A single
/// dispatcher is constructed at App start time and cloned (cheaply, via
/// `Arc`) into `SlashCommandContext` for reuse by every slash input.
pub(crate) struct Dispatcher {
    registry: Arc<Registry>,
}

impl Dispatcher {
    /// Build a dispatcher around the supplied shared registry.
    pub(crate) fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// Spec-mandated entry point. Parses `input`, looks the command up
    /// in the registry, and invokes the handler's `execute`. Returns
    /// `Ok(())` for both recognized and unknown commands; unknown
    /// names emit a `TuiEvent::Error("Unknown command: /{name}")`
    /// through `ctx.tui_tx` instead of propagating an error.
    ///
    /// Non-slash / empty / bare-`/` input is a no-op returning `Ok(())`
    /// with no events emitted — matching the pre-existing behaviour of
    /// the legacy inline match's `_ => false` arm for such inputs.
    ///
    /// ## TASK-AGS-803 wiring
    ///
    /// Tokenization is delegated to [`CommandParser::parse`] (TASK-AGS-801)
    /// for its richer `Result<ParsedCommand, ParseError>` surface. The
    /// leading-`/` gate stays HERE inside the dispatcher (option B from
    /// Steven's orchestrator directive) so the dispatcher does NOT steal
    /// non-slash input from the legacy inline match in `main.rs` — that
    /// behaviour is pinned by `dispatch_non_slash_input_returns_ok_no_emit`.
    ///
    /// Registry lookup uses `Registry::get`, which is alias-aware after
    /// TASK-AGS-802 — no extra alias code lives here.
    pub(crate) fn dispatch(&self, ctx: &mut CommandContext, input: &str) -> anyhow::Result<()> {
        let trimmed = input.trim();

        // PATH A hybrid gate: the dispatcher MUST NOT consume non-slash
        // input. `dispatch_non_slash_input_returns_ok_no_emit` and
        // `dispatch_whitespace_only_input_no_emit` pin this invariant.
        if !trimmed.starts_with('/') {
            return Ok(());
        }

        // Bare `/` is a silent no-op (matches the legacy inline match's
        // `_ => false` arm and the pre-existing
        // `dispatch_bare_slash_returns_ok_no_emit` test).
        if trimmed == "/" {
            return Ok(());
        }

        // Delegate tokenization to the structured-error wrapper.
        // `CommandParser::parse` itself relaxes the leading-`/`
        // requirement, but we already enforced it above, so the only
        // error variants reachable here are `UnclosedQuote` and
        // `MalformedFlag` (true tokenizer failures). `Empty` /
        // `MissingName` are defended as quiet no-ops for safety against
        // future refactors.
        let parsed = match CommandParser::parse(trimmed) {
            Ok(p) => p,
            Err(ParseError::Empty) | Err(ParseError::MissingName) => {
                return Ok(());
            }
            Err(ParseError::UnclosedQuote) => {
                ctx.emit(archon_tui::app::TuiEvent::Error(
                    "Parse error: unclosed quote".to_string(),
                ));
                return Ok(());
            }
            Err(ParseError::MalformedFlag(tok)) => {
                ctx.emit(archon_tui::app::TuiEvent::Error(format!(
                    "Parse error: malformed flag '{tok}'"
                )));
                return Ok(());
            }
        };

        match self.registry.get(&parsed.name) {
            Some(handler) => match handler.execute(ctx, &parsed.raw_args) {
                Ok(()) => Ok(()),
                Err(error) => {
                    ctx.emit(archon_tui::app::TuiEvent::Error(format!(
                        "Command /{} failed: {error}",
                        parsed.name
                    )));
                    ctx.emit(archon_tui::app::TuiEvent::SlashCommandComplete);
                    Err(error)
                }
            },
            None => {
                // TASK-AGS-804: delegate message assembly to the
                // dedicated formatter, which owns the zero / one /
                // many branching, the case-insensitive exact-match
                // fallback, and the defensive 3-suggestion cap. The
                // dispatcher is only responsible for emission.
                let msg = errors::format_unknown_command(&parsed.name, &self.registry);
                ctx.emit(archon_tui::app::TuiEvent::Error(msg));
                Ok(())
            }
        }
    }

    /// Returns `true` if `input` parses as a slash command whose name
    /// is registered (directly or via an alias). Used by
    /// `handle_slash_command` to decide whether to fall through to the
    /// legacy inline match (PATH A hybrid only — removed once handler
    /// bodies migrate into the registry).
    ///
    /// Mirrors the leading-`/` gate from `dispatch` so a plain-text
    /// input never claims to be a recognized slash command.
    pub(crate) fn recognizes(&self, input: &str) -> bool {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') || trimmed == "/" {
            return false;
        }
        CommandParser::parse(trimmed)
            .ok()
            .and_then(|p| self.registry.get(&p.name).map(|_| ()))
            .is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "dispatcher_tests/mod.rs"]
mod tests;
