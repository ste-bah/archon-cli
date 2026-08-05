//! TASK-AGS-817: /memory slash-command handler (Option C, DIRECT pattern,
//! THIRD Batch-3 body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:521-525` and the legacy match arm
//! at `src/command/slash.rs:342-345` (the pre-AGS-817 async
//! `handle_memory_command` free function lived in this same file and
//! has been REPLACED with the sync handler below).
//!
//! # Why DIRECT (no snapshot, no effect slot)?
//!
//! `archon_memory::MemoryTrait` (defined at
//! `crates/archon-memory/src/access.rs:24`) is FULLY SYNC. All 12 trait
//! methods are plain `fn ... -> Result<_, MemoryError>` — zero `async
//! fn`, zero `.await` needed anywhere in the handler body. The trait
//! carries a `Send + Sync` bound so `Arc<dyn MemoryTrait>` is cheap to
//! clone (atomic refcount bump, ~8 bytes). Consequently:
//!
//! - NO `MemorySnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/model` / `/cost` / `/mcp` / `/context`).
//! - NO `CommandEffect` variant (`clear_all` mutates the graph but via
//!   a direct sync call; no write-back to a `Mutex`-guarded
//!   `SlashCommandContext` field).
//! - A new `CommandContext::memory: Option<Arc<dyn MemoryTrait>>` field
//!   (8 -> 9) populated UNCONDITIONALLY by `build_command_context`,
//!   mirroring the AGS-815 `session_id` cross-cutting precedent. Any
//!   future handler that needs a memory handle inherits this field for
//!   free without proliferating per-command builder match arms.
//!
//! The sole side effect is `ctx.tui_tx.send(TuiEvent::…)` — sync
//! and legal inside `CommandHandler::execute`. Matches
//! AGS-810/812/815/816 DIRECT-pattern precedent.
//!
//! # Why UNCONDITIONAL populate (not per-dispatch match arm)?
//!
//! `Arc::clone(&Arc<dyn MemoryTrait>)` is a single atomic refcount
//! bump. Every dispatch pays ~8 bytes + one atomic op regardless of
//! whether the target handler reads `memory`. Peer precedent:
//! AGS-815 session_id UNCONDITIONAL populate (36-byte UUID String
//! clone per dispatch) was accepted as negligible. Field count
//! 8 -> 9 (refactor threshold is 10+; post-AGS-817 projection is 9
//! because Batch-3 tail /export and /theme are DIRECT no-new-field
//! per pre-analysis).
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted string is faithful to the deleted async
//! `handle_memory_command` body:
//! - `"" | "list"` empty -> `TextDelta("\nNo memories stored.\n")`
//! - `"" | "list"` non-empty -> `TextDelta(format!("\nRecent memories \
//!   ({len}):\n  [{short_id}] {title} ({mtype}, {date})\n..."))`
//! - `"" | "list"` err -> `Error(format!("Memory graph error: {e}"))`
//! - `"search"` no arg -> `Error("Usage: /memory search <query>")`
//! - `"search"` empty results -> `TextDelta(format!("\nNo memories \
//!   matching \"{arg}\".\n"))`
//! - `"search"` non-empty -> `TextDelta(format!("\nMemories matching \
//!   \"{arg}\" ({len}):\n  [{short_id}] {title} -- {snippet}\n..."))`
//! - `"search"` err -> `Error(format!("Memory search error: {e}"))`
//! - `"clear"` ok -> `TextDelta(format!("\nCleared {n} memories from \
//!   the graph.\n"))`
//! - `"clear"` err -> `Error(format!("Failed to clear memories: {e}"))`
//! - unknown sub -> `Error(format!("Unknown memory subcommand: \
//!   {other}. Use list, search, or clear."))`
//!
//! The one emission-primitive change is `tui_tx.send(..).await` (async)
//! -> `ctx.tui_tx.try_send(..)` (sync), matching every peer migrated
//! handler (AGS-806..816). `/memory` output is best-effort informational
//! UI — dropping a message under 16-cap channel backpressure is
//! preferable to stalling the dispatcher.
//!
//! The `truncate_str` UTF-8 helper is preserved byte-for-byte from the
//! pre-migration module (char-boundary safe, 80-byte limit with "..."
//! suffix).
//!
//! # Aliases
//!
//! Shipped pre-AGS-817: `&["mem"]` (from `declare_handler!`). Drift-
//! reconcile shipped-wins: the `mem` alias is PRESERVED. Dropping it
//! would regress any operator workflow depending on `/mem list` /
//! `/mem search ...` working today through the stub's dispatcher path.
//! Matches AGS-813 shipped-wins precedent for alias-set preservation.
//!
//! # Args-path reconciliation
//!
//! Shipped body used `input.strip_prefix("/memory").trim()` followed by
//! `rest.split_once(' ')` to split into subcommand + single-string
//! argument. The registry parser tokenizes on whitespace, so `args` is
//! already a `Vec<String>` of individual tokens. To preserve the
//! shipped semantics where `/memory search hello world` forwards
//! `"hello world"` (not `"hello"` alone) to `recall_memories`, the
//! handler:
//!
//! 1. Reads `args.first()` as the subcommand.
//! 2. Joins `args.get(1..)` with a single space to rebuild the
//!    original single-string query argument.
//!
//! Empty / missing subcommand defaults to `"list"` (matches shipped
//! `"" | "list"` arm).

#[allow(unused_imports)]
use archon_tui::app::TuiEvent;

use archon_memory::MemoryTrait;

use crate::command::registry::{CommandContext, CommandHandler};

/// Truncate a string to at most `max` bytes, appending "..." if truncated.
/// Safe for multi-byte UTF-8: always splits on a char boundary.
///
/// Preserved byte-for-byte from the pre-AGS-817 async
/// `handle_memory_command` helper. Private to this module.
fn truncate_str(s: &str, max: usize) -> String {
    let trimmed = s.replace('\n', " ");
    if trimmed.len() <= max {
        trimmed
    } else {
        let mut end = max.saturating_sub(3);
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &trimmed[..end])
    }
}

/// Zero-sized handler registered as the primary `/memory` command.
///
/// Aliases: `["mem"]` — PRESERVED from the shipped declare_handler! stub
/// (shipped-wins drift-reconcile; see module rustdoc Aliases section).
///
/// Subcommands dispatched inside `execute`:
/// * `""` / `list` — call `MemoryTrait::list_recent(10)` and emit
///   formatted recent-memories list (or empty/error branch).
/// * `search <query>` — call `MemoryTrait::recall_memories(query, 10)`
///   and emit formatted results (or usage/empty/error branch).
/// * `clear` — call `MemoryTrait::clear_all()` and emit count (or error).
/// * any other token — emit unknown-subcommand hint.
pub(crate) struct MemoryHandler;

impl CommandHandler for MemoryHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // 1. Require memory handle. `build_command_context` populates
        //    this unconditionally from `SlashCommandContext::memory` so
        //    at the real dispatch site this branch never fires. Test
        //    fixtures that construct `CommandContext` directly with
        //    `memory: None` will hit this branch and observe an
        //    Err — mirroring the AGS-815
        //    `fork_handler_execute_without_session_id_returns_err`
        //    pattern.
        let Some(memory_arc) = ctx.memory.as_ref() else {
            return Err(anyhow::anyhow!(
                "/memory dispatched without memory handle — \
                 CommandContext population missing in dispatch-site \
                 builder (build_command_context always populates it; \
                 this is a test-fixture or wiring bug)"
            ));
        };
        // Borrow through Arc<dyn MemoryTrait>. No clone needed — the
        // handler body never outlives `ctx`.
        let memory: &dyn MemoryTrait = memory_arc.as_ref();

        // 2. Args-path reconciliation: shipped body used
        //    `input.split_once(' ')` on the whole rest-string; the
        //    parser instead hands us a tokenized args vec. Rebuild the
        //    shipped single-string arg by joining tokens with ' '.
        //    See module rustdoc "Args-path reconciliation" section.
        let subcmd = args.first().map(|s| s.as_str()).unwrap_or("").trim();
        let arg_joined = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
        let arg = arg_joined.trim();

        match subcmd {
            "" | "list" => match memory.list_recent(10) {
                Ok(memories) if memories.is_empty() => {
                    ctx.emit(TuiEvent::TextDelta("\nNo memories stored.\n".into()));
                }
                Ok(memories) => {
                    let mut out = format!("\nRecent memories ({}):\n", memories.len());
                    for m in &memories {
                        let short_id = &m.id[..8.min(m.id.len())];
                        let date = m.created_at.format("%Y-%m-%d %H:%M");
                        out.push_str(&format!(
                            "  [{short_id}] {title} ({mtype}, {date})\n",
                            title = m.title,
                            mtype = m.memory_type,
                        ));
                    }
                    ctx.emit(TuiEvent::TextDelta(out));
                }
                Err(e) => {
                    ctx.emit(TuiEvent::Error(format!("Memory graph error: {e}")));
                }
            },
            "search" => {
                if arg.is_empty() {
                    ctx.emit(TuiEvent::Error("Usage: /memory search <query>".into()));
                    return Ok(());
                }
                match memory.recall_memories(arg, 10) {
                    Ok(results) if results.is_empty() => {
                        ctx.emit(TuiEvent::TextDelta(format!(
                            "\nNo memories matching \"{arg}\".\n"
                        )));
                    }
                    Ok(results) => {
                        let mut out =
                            format!("\nMemories matching \"{arg}\" ({}):\n", results.len());
                        for m in &results {
                            let short_id = &m.id[..8.min(m.id.len())];
                            out.push_str(&format!(
                                "  [{short_id}] {title} -- {snippet}\n",
                                title = m.title,
                                snippet = truncate_str(&m.content, 80),
                            ));
                        }
                        ctx.emit(TuiEvent::TextDelta(out));
                    }
                    Err(e) => {
                        ctx.emit(TuiEvent::Error(format!("Memory search error: {e}")));
                    }
                }
            }
            "clear" => match memory.clear_all() {
                Ok(n) => {
                    ctx.emit(TuiEvent::TextDelta(format!(
                        "\nCleared {n} memories from the graph.\n"
                    )));
                }
                Err(e) => {
                    ctx.emit(TuiEvent::Error(format!("Failed to clear memories: {e}")));
                }
            },
            // Two-step on purpose. Deleting memories is irreversible and the
            // selection rules are heuristics, so the plan is shown first and
            // nothing is removed without `apply`.
            "prune" => {
                let apply = arg == "apply";
                if !arg.is_empty() && !apply {
                    ctx.emit(TuiEvent::Error("Usage: /memory prune [apply]".to_string()));
                    return Ok(());
                }
                match archon_memory::hygiene::plan_prune(memory) {
                    Ok(mut plan) => {
                        if apply && !plan.is_empty() {
                            match archon_memory::hygiene::apply_prune(memory, &plan) {
                                Ok(_) => plan.applied = true,
                                Err(e) => {
                                    ctx.emit(TuiEvent::Error(format!("Prune failed: {e}")));
                                    return Ok(());
                                }
                            }
                        }
                        ctx.emit(TuiEvent::TextDelta(
                            archon_memory::hygiene::format_prune_report(&plan),
                        ));
                    }
                    Err(e) => {
                        ctx.emit(TuiEvent::Error(format!("Prune planning failed: {e}")));
                    }
                }
            }
            other => {
                ctx.emit(TuiEvent::Error(format!(
                    "Unknown memory subcommand: {other}. Use list, \
                     search, prune, or clear."
                )));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Preserved from the shipped declare_handler! stub at
        // registry.rs:522 (shipped-wins drift-reconcile).
        "Inspect or manage long-term memory"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Preserved from the shipped declare_handler! stub (see module
        // rustdoc Aliases section).
        &["mem"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-817: tests for /memory slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "memory_tests/mod.rs"]
mod tests;
