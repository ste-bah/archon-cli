//! TASK-AGS-POST-6-BODIES-B13-GARDEN: /garden slash-command handler
//! (DIRECT-sync-via-MemoryTrait pattern, body-migrated from the legacy
//! match arm at `src/command/slash.rs:124-155` and the `declare_handler!`
//! stub at `src/command/registry.rs:958`).
//!
//! # Pattern reclassification — DIRECT-sync-via-MemoryTrait (NOT SNAPSHOT-async)
//!
//! The original B13 ticket label read "SNAPSHOT async" but that
//! classification was mechanical, not actual. Both underlying
//! archon-memory entry points are fully SYNC:
//!
//!   * `archon_memory::garden::format_garden_stats(&dyn MemoryTrait,
//!     usize) -> Result<String, MemoryError>`
//!   * `archon_memory::garden::consolidate(&dyn MemoryTrait,
//!     &GardenConfig) -> Result<GardenReport, MemoryError>`
//!
//! Neither function is `async`, neither takes a future, and both operate
//! on a plain `&dyn MemoryTrait` borrow (all 12 MemoryTrait methods are
//! sync). This matches the AGS-817 `/memory` DIRECT-pattern precedent at
//! `src/command/memory.rs` verbatim — no SNAPSHOT type, no async
//! mutex-lock traffic inside the builder, no `CommandEffect` variant.
//!
//! # R-item inventory (mirror of memory.rs R-items)
//!
//! * **R1 — description/aliases pinned.** Description
//!   "Run memory garden consolidation or show stats" is preserved
//!   byte-for-byte from the shipped `declare_handler!` stub at
//!   `registry.rs:958`. Aliases list is empty (`&[]`) to match the
//!   shipped two-arg declare_handler! form (no aliases slice). Shipped-
//!   wins drift-reconcile rule per AGS-817.
//!
//! * **R2 — try_send ergonomics.** The shipped legacy arm used
//!   `tui_tx.send(..).await` (async). The sync `CommandHandler::execute`
//!   signature forbids `.await`, so every emission is switched to
//!   `ctx.tui_tx.try_send(..)` with the Result discarded via `let _ =`.
//!   Mirrors AGS-817 /memory R2. Dropping a message under 16-cap
//!   channel backpressure is preferable to stalling the dispatcher
//!   (same trade-off as /memory; /garden output is best-effort
//!   informational UI).
//!
//! * **R3 — error-first-returns.** Missing `ctx.memory` or (for the
//!   consolidate path) missing `ctx.garden_config` returns
//!   `Err(anyhow::Error)` describing the wiring bug. At the real
//!   dispatch site `build_command_context` populates both
//!   UNCONDITIONALLY so this branch never fires in production;
//!   test-fixture and wiring regressions observe the explicit Err
//!   instead of a panic. Mirrors AGS-815 /fork + AGS-817 /memory
//!   builder-contract guards.
//!
//! * **R4 — args-path reconciliation.** The shipped match arm peeled
//!   the subcommand off the raw input string via
//!   `s.strip_prefix("/garden").unwrap_or("").trim()`. The registry
//!   parser tokenises on whitespace before dispatch, so the handler
//!   receives `args: &[String]` where the subcommand (if any) is the
//!   first token. `args.first().map(|s| s.as_str()).unwrap_or("").trim()`
//!   reconstructs the shipped semantics exactly: missing first token
//!   (empty args) maps to `""` which falls into the default-consolidate
//!   branch, and `args[0] == "stats"` fires the stats branch. Mirrors
//!   AGS-817 /memory R4.
//!
//! * **R5 — no snapshot / no effect-slot required.** Unlike /status
//!   (AGS-807), /model (AGS-808), /cost (AGS-809), /mcp (AGS-811),
//!   /context (AGS-814), /denials (B08), /effort (B11), or
//!   /permissions (B12) — all of which required snapshot pre-capture
//!   or effect-slot deferral for async operations — /garden's two
//!   archon-memory entry points are pure sync and run directly inside
//!   `execute`. No `CommandContext::garden_snapshot` field is added.
//!   The `CommandEffect` enum is NOT extended. Only a
//!   `CommandContext::garden_config` DIRECT field is added (mirrors
//!   AGS-817 `CommandContext::memory`) so the consolidate path can
//!   reach the `&GardenConfig` borrow without crossing the
//!   `SlashCommandContext` boundary.
//!
//! * **R6 — emission ordering swap vs. shipped.** The shipped legacy
//!   arm used `tui_tx.send(..).await` so the emission completes
//!   (post-await) before the match arm returns. The sync handler
//!   uses `try_send` which returns synchronously — the emission
//!   enters the channel immediately and the handler returns `Ok(())`
//!   without waiting for the TUI event loop to drain. From the TUI's
//!   perspective, ordering of observed events is unchanged (events are
//!   read in the order they were pushed into the channel); only the
//!   handler-side timing is different. Mirrors B10/B11/B12 precedent
//!   for the async->sync emission swap.
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted string is faithful to the legacy match arm at
//! `slash.rs:124-155`:
//!   * `"stats"` Ok -> `TextDelta(format!("\n{stats}\n"))` via `try_send`
//!   * `"stats"` Err -> `Error(format!("Garden stats failed: {e}"))` via `try_send`
//!   * default Ok -> `TextDelta(format!("\n{formatted}\n"))` where
//!     `formatted = report.format()` via `try_send`
//!   * default Err -> `Error(format!("Garden consolidation failed: {e}"))` via `try_send`
//!
//! Leading AND trailing newlines are preserved (shipped used
//! `format!("\n{stats}\n")` and `format!("\n{formatted}\n")`).

use archon_tui::app::TuiEvent;
use std::sync::Arc;

use archon_memory::MemoryTrait;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/garden` command.
///
/// Aliases: `[]` — PRESERVED from the shipped declare_handler! stub at
/// `registry.rs:958` (shipped-wins drift-reconcile; the stub used the
/// two-arg declare_handler! form with no aliases slice).
///
/// Subcommands dispatched inside `execute`:
/// * `"stats"` — call `archon_memory::garden::format_garden_stats(memory, 10)`
///   and emit formatted stats (or error branch).
/// * any other token (including empty) — call
///   `archon_memory::garden::consolidate(memory, &garden_config)` and
///   emit the formatted report (or error branch).
pub(crate) struct GardenHandler;

impl CommandHandler for GardenHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // 1. Require memory handle. `build_command_context` populates
        //    this unconditionally from `SlashCommandContext::memory` so
        //    at the real dispatch site this branch never fires. Test
        //    fixtures that construct `CommandContext` directly with
        //    `memory: None` will hit this branch and observe an
        //    Err — mirroring the AGS-817 /memory pattern.
        let Some(memory_arc): Option<&Arc<dyn MemoryTrait>> = ctx.memory.as_ref() else {
            return Err(anyhow::anyhow!(
                "/garden dispatched without memory handle — \
                 CommandContext population missing in dispatch-site \
                 builder (build_command_context always populates it; \
                 this is a test-fixture or wiring bug)"
            ));
        };
        let memory: &dyn MemoryTrait = memory_arc.as_ref();

        // 2. Args-path reconciliation. Shipped legacy arm used
        //    `s.strip_prefix("/garden").unwrap_or("").trim()`. The
        //    registry parser tokenises on whitespace, so `args` is
        //    already a `Vec<String>` of individual tokens. We read
        //    `args.first()` as the subcommand (trimmed) — missing token
        //    defaults to `""`, which falls into the default consolidate
        //    branch below. See module rustdoc R4.
        let sub = args.first().map(|s| s.as_str()).unwrap_or("").trim();

        if sub == "stats" {
            match archon_memory::garden::format_garden_stats(memory, 10) {
                Ok(stats) => {
                    let _ = ctx.tui_tx.send(TuiEvent::TextDelta(format!("\n{stats}\n")));
                }
                Err(e) => {
                    let _ = ctx
                        .tui_tx
                        .send(TuiEvent::Error(format!("Garden stats failed: {e}")));
                }
            }
        } else {
            // Consolidate path — requires `ctx.garden_config`. Same
            // builder-contract guard as `ctx.memory` above.
            let Some(garden_config) = ctx.garden_config.as_ref() else {
                return Err(anyhow::anyhow!(
                    "/garden dispatched without garden_config — \
                     CommandContext population missing in dispatch-site \
                     builder (build_command_context always populates it; \
                     this is a test-fixture or wiring bug)"
                ));
            };
            match archon_memory::garden::consolidate(memory, garden_config) {
                Ok(report) => {
                    let formatted = report.format();
                    let _ = ctx
                        .tui_tx
                        .send(TuiEvent::TextDelta(format!("\n{formatted}\n")));
                }
                Err(e) => {
                    let _ = ctx
                        .tui_tx
                        .send(TuiEvent::Error(format!("Garden consolidation failed: {e}")));
                }
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Preserved byte-for-byte from the shipped declare_handler! stub
        // at registry.rs:958 (shipped-wins drift-reconcile).
        "Run memory garden consolidation or show stats"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Preserved from the shipped declare_handler! stub (two-arg
        // form, no aliases slice). See module rustdoc R1.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B13-GARDEN: tests for /garden body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "garden_tests/mod.rs"]
mod tests;
