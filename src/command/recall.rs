//! TASK-AGS-POST-6-BODIES-B18-RECALL: /recall slash-command handler
//! (DIRECT-sync-via-MemoryTrait pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:1305` and the legacy match arm at
//! `src/command/slash.rs:569-615`.
//!
//! # R1 — pattern = DIRECT-sync-via-MemoryTrait (no snapshot, no effect slot)
//!
//! The shipped `/recall` body calls
//! `archon_memory::MemoryTrait::recall_memories(query, limit)` which
//! is a plain sync method on the object-safe trait (see
//! `crates/archon-memory/src/access.rs` — all 12 methods are sync and
//! the trait carries `Send + Sync`). No `tokio::sync::Mutex` guards on
//! the read path and no writes back to `SlashCommandContext` state.
//! Consequently:
//!
//! - NO `RecallSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/cost` / `/mcp` SNAPSHOT variants).
//! - NO `CommandEffect` variant (handler never mutates shared state;
//!   it only emits `TuiEvent`s — matches AGS-815 /fork precedent).
//! - NO `build_command_context` match arm added for `/recall`. Unlike
//!   the SNAPSHOT-ONLY tickets (AGS-807/808/809/811/814) which gate
//!   their populate step on the primary name, AGS-817 /memory already
//!   extended `CommandContext` with
//!   `memory: Option<Arc<dyn MemoryTrait>>` populated UNCONDITIONALLY
//!   in the builder (`context.rs:69` —
//!   `memory: Some(Arc::clone(&slash_ctx.memory))`). `/recall` reuses
//!   that exact field — no new context.rs wiring required for this
//!   ticket. Matches the cross-cutting precedent set by AGS-817 for
//!   `memory` and AGS-815 for `session_id`.
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/recall` match arm at slash.rs:569-615 was *async*
//! only because it lived inside the async dispatch loop and emitted
//! via `tui_tx.send(..).await`. The underlying `recall_memories`
//! call is 100% sync. In the new sync handler, we emit via
//! `ctx.tui_tx.try_send(..)` (best-effort — dropping a UI message
//! under channel backpressure is preferable to stalling the
//! dispatcher). Matches AGS-815 /fork + AGS-817 /memory precedent.
//!
//! # R3 — args reconstruction via `args.join(" ").trim()`
//!
//! The shipped body used `s.strip_prefix("/recall").unwrap_or("").trim()`
//! on the full input string, so `/recall hello world` (two tokens) was
//! forwarded verbatim as the query `"hello world"`. The registry
//! parser tokenizes on whitespace, so `args` is `["hello", "world"]`.
//! To preserve the shipped single-string semantics while going through
//! the parser, the handler joins `args` with a single space then
//! `.trim()`s. This is byte-equivalent to the shipped behaviour for
//! all inputs: single-token queries pass through unchanged, multi-token
//! queries preserve the whitespace-joined substring, empty args →
//! empty string → usage-error branch identical to the shipped
//! `if query.is_empty()` check at slash.rs:572. See
//! `src/command/add_dir.rs:155-180` and `src/command/rename.rs:139-140`
//! for the same pattern.
//!
//! # R4 — byte-identity of description / aliases / emitted events
//!
//! - `description()` returns `"Recall memories matching a query"` —
//!   byte-identical to the `declare_handler!` stub at registry.rs:1305.
//! - `aliases()` returns `&[]` — the shipped stub used the 2-arg
//!   `declare_handler!` form (no aliases slice) and the Steven
//!   directive at registry.rs:1302-1304 explicitly forbids adding
//!   `recall` as an alias on `/memory` or any other handler.
//! - Emitted events preserve the shipped slash.rs:569-615 format
//!   strings BYTE-FOR-BYTE, including the EM-DASH (U+2014, NOT a
//!   hyphen) in the empty-query usage error:
//!   * Empty-query → `TuiEvent::Error("Usage: /recall <query> — \
//!     search memories by keyword")`. The `—` character between
//!     `<query>` and `search` is Unicode EM DASH (U+2014), NOT a
//!     hyphen-minus. Any ASCII-ification here is a byte-identity
//!     violation and Sherlock will flag it.
//!   * No-match → `TuiEvent::TextDelta(format!("\nNo memories found \
//!     for '{query}'.\n"))` — literal single-quotes around `{query}`.
//!   * Match header → `format!("\n{} memories matching '{query}':\n\
//!     \n", memories.len())` — count FIRST, then word "memories",
//!     single-quotes around query, trailing colon + blank line.
//!   * Per-entry → `"  [{id_short}] {title}\n    {snippet}...\n\n"` —
//!     TWO spaces then bracket, ONE space between bracket and title,
//!     FOUR spaces before snippet, literal trailing `...` then blank
//!     line.
//!   * Title fallback → `if m.title.is_empty() { "(untitled)" } else
//!     { &m.title }` — parens around "untitled".
//!   * Snippet → `m.content.chars().take(100).collect::<String>()` —
//!     CHAR take (UTF-8 safe), NOT byte slice.
//!   * id_short → `&m.id[..8.min(m.id.len())]` — byte slice with
//!     length cap.
//!   * Recall limit → `10` (hardcoded).
//!   * Search failure → `TuiEvent::Error(format!("Memory search \
//!     failed: {e}"))`.
//!
//! # R5 — aliases = zero (Steven directive)
//!
//! Shipped pre-B18: none (2-arg declare_handler! form at
//! registry.rs:1305). The comment block at registry.rs:1302-1304
//! encodes the Steven directive explicitly:
//!
//! > "/recall stays a standalone primary command and has NO aliases
//! > — Steven directive. Do NOT add \"recall\" as an alias on
//! > /memory or any other handler."
//!
//! No aliases added. Matches /fork / /mcp / /context / /hooks /
//! /rename precedent.
//!
//! # R6 — memory field reuse (no new context.rs snapshot wiring)
//!
//! `CommandContext::memory: Option<Arc<dyn MemoryTrait>>` is already
//! populated unconditionally by `build_command_context` per AGS-817
//! /memory (`context.rs:69` —
//! `memory: Some(Arc::clone(&slash_ctx.memory))`). This ticket REUSES
//! that exact field — there is no `recall_snapshot` type, no
//! context.rs match arm added, no new `build_command_context` wiring.
//! The test fixture helper (`make_recall_ctx`) mirrors the AGS-817
//! /memory `make_ctx(memory)` shape.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `RecallHandler` (PATH A,
//! via the dispatcher at slash.rs:46) AND the legacy
//! `s if s.starts_with("/recall")` match arm at slash.rs:569-615 are
//! live. Every `/recall` invocation therefore fires twice — once via
//! the handler and once via the legacy arm. This is the Stage-6
//! body-migrate protocol: Gate 5 deletes the legacy match arm in a
//! SEPARATE subsequent subagent run (NOT this subagent's
//! responsibility). Do NOT touch slash.rs in this ticket.

use archon_memory::MemoryTrait;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/recall` command.
///
/// No aliases. Shipped pre-B18 stub carried none (2-arg
/// declare_handler! form) and the Steven directive at
/// registry.rs:1302-1304 explicitly forbids adding `recall` as an
/// alias on any other handler. Matches /fork / /mcp / /context /
/// /hooks / /rename precedent.
pub(crate) struct RecallHandler;

impl RecallHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`DoctorHandler::new`, `UsageHandler::new`, `RenameHandler::new`)
    /// even though the unit struct is constructible without it — the
    /// explicit constructor keeps the call site in registry.rs:1363
    /// copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for RecallHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for RecallHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // R3: join multi-token args with " " and trim. Byte-equivalent
        // to the shipped `s.strip_prefix("/recall").unwrap_or("").trim()`
        // for all inputs — single-token queries collapse to the same
        // value as `args.first().unwrap_or("").as_str()`, multi-token
        // queries preserve the whitespace-joined substring. Empty args
        // and a whitespace-only join both produce the empty string,
        // routing to the usage-error branch identical to the shipped
        // `if query.is_empty()` check at slash.rs:572.
        let joined = args.join(" ");
        let query = joined.trim();

        if query.is_empty() {
            // Empty-query branch — byte-for-byte preservation of
            // shipped format string at slash.rs:574-576. The `—`
            // between `<query>` and `search` is Unicode EM DASH
            // (U+2014), NOT a hyphen-minus. Do NOT ASCII-ify.
            ctx.emit(TuiEvent::Error(
                "Usage: /recall <query> — search memories by keyword".into(),
            ));
            return Ok(());
        }

        // R6: require memory handle. `build_command_context` populates
        // this unconditionally from `SlashCommandContext::memory` per
        // the AGS-817 /memory precedent (context.rs:69), so at the
        // real dispatch site this branch never fires. Test fixtures
        // that construct `CommandContext` directly with `memory: None`
        // will hit this branch and observe an Err — mirroring the
        // AGS-817 `memory_handler_execute_without_memory_returns_err`
        // pattern.
        let memory_arc = ctx.memory.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "RecallHandler invoked without ctx.memory populated — \
                 build_command_context bug"
            )
        })?;
        let memory: &dyn MemoryTrait = memory_arc.as_ref();

        match memory.recall_memories(query, 10) {
            Ok(memories) => {
                if memories.is_empty() {
                    // No-match branch — byte-for-byte preservation of
                    // shipped format string at slash.rs:585-587.
                    ctx.emit(TuiEvent::TextDelta(format!(
                        "\nNo memories found for '{query}'.\n"
                    )));
                } else {
                    // Match branch — byte-for-byte preservation of
                    // shipped format loop at slash.rs:590-604.
                    let mut out = format!("\n{} memories matching '{query}':\n\n", memories.len());
                    for m in &memories {
                        let title = if m.title.is_empty() {
                            "(untitled)"
                        } else {
                            &m.title
                        };
                        // Snippet: char-based take(100) is UTF-8
                        // safe (byte slice `&m.content[..100]` would
                        // panic on a non-char-boundary split).
                        let snippet: String = m.content.chars().take(100).collect();
                        let id_short = &m.id[..8.min(m.id.len())];
                        out.push_str(&format!("  [{id_short}] {title}\n    {snippet}...\n\n"));
                    }
                    ctx.emit(TuiEvent::TextDelta(out));
                }
            }
            Err(e) => {
                // Search-failure branch — byte-for-byte preservation
                // of shipped format string at slash.rs:608-610.
                ctx.emit(TuiEvent::Error(format!("Memory search failed: {e}")));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // R4: byte-identical to declare_handler! stub at
        // registry.rs:1305.
        "Recall memories matching a query"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice) and the Steven
        // directive at registry.rs:1302-1304 explicitly forbids
        // adding `recall` as an alias on any other handler.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B18-RECALL: tests for /recall slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "recall_tests/mod.rs"]
mod tests;
