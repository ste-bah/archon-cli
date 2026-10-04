//! TASK-AGS-POST-6-BODIES-B21-CHECKPOINT: /checkpoint slash-command handler
//! (DIRECT pattern, body-migrate).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!` stub
//! in `src/command/registry.rs:1361` and the legacy match arm at
//! `src/command/slash.rs:452-527`.
//!
//! # R1 — pattern = DIRECT (not EFFECT-SLOT)
//!
//! Parent-context recon proved every touched archon-session entry point
//! is sync (not async as the B21 task tag suggested):
//!
//! - `archon_session::checkpoint::CheckpointStore::open(&Path)` — sync
//!   (checkpoint.rs:83).
//! - `CheckpointStore::list_modified(&str)` — sync (checkpoint.rs:244).
//! - `CheckpointStore::restore(&str, &str)` — sync (checkpoint.rs:198).
//!
//! Consequently:
//!
//! - NO `CheckpointSnapshot` type (nothing to pre-compute inside an
//!   async guard).
//! - NO `CommandEffect` variant (handler never mutates shared
//!   SlashCommandContext state; it only emits `TuiEvent`s — matches
//!   AGS-815 /fork and B17 /rename precedent).
//! - NO `build_command_context` match arm added. Like /fork (AGS-815)
//!   and /rename (B17), /checkpoint reuses the UNCONDITIONAL
//!   `CommandContext::session_id: Option<String>` field populated by
//!   the builder. No new context.rs wiring required.
//!
//! # R2 — sync CommandHandler::execute rationale
//!
//! `CommandHandler::execute` is sync per the AGS-622 trait contract.
//! The shipped `/checkpoint` match arm at slash.rs:452-527 was *async*
//! only because it emitted via `tui_tx.send(..).await`. Every
//! archon-session call beneath it is 100% sync. In the new sync handler
//! we emit via `ctx.tui_tx.try_send(..)` (best-effort — dropping a UI
//! message under channel backpressure is preferable to stalling the
//! dispatcher). Matches AGS-815 /fork and B17 /rename precedent.
//!
//! # R3 — args reconstruction via `args.join(" ").trim()`
//!
//! The shipped body used
//! `s.strip_prefix("/checkpoint").unwrap_or("").trim()`
//! on the full input string, so `/checkpoint restore some/file.txt`
//! (two post-primary tokens) was forwarded verbatim as the arg string
//! `"restore some/file.txt"`. The registry parser tokenizes on
//! whitespace, so `args` is `["restore", "some/file.txt"]`. To preserve
//! shipped single-string semantics we `args.join(" ").trim()`, then
//! match the original branching (`arg == "list" || arg.is_empty()` /
//! `arg.strip_prefix("restore").map(|s| s.trim())`). Byte-equivalent to
//! shipped for all inputs; matches B17 /rename and B18 /recall
//! precedent.
//!
//! # R4 — byte-identity of all 8 event branches
//!
//! Preserved from slash.rs:452-527 byte-for-byte:
//!
//! 1. list / empty + Ok(empty) →
//!    `TuiEvent::TextDelta("\nNo checkpoints for this session.\n")`.
//! 2. list / empty + Ok(non-empty) → `TuiEvent::TextDelta` starting
//!    `"\nCheckpoints:\n"` then per-entry
//!    `format!("  turn {} | {} | {} | {}\n", s.turn_number, s.tool_name,
//!     s.file_path, s.timestamp)`.
//! 3. list / empty + Err → `TuiEvent::Error(format!("Checkpoint list
//!    error: {e}"))`.
//! 4. `restore` + empty path →
//!    `TuiEvent::Error("Usage: /checkpoint restore <file_path>")`.
//! 5. `restore <p>` + Ok → `TuiEvent::TextDelta(format!("\nRestored:
//!    {file_path}\n"))`.
//! 6. `restore <p>` + Err → `TuiEvent::Error(format!("Restore failed:
//!    {e}"))`.
//! 7. Store-open Err (both list and restore paths) →
//!    `TuiEvent::Error(format!("Checkpoint store error: {e}"))`.
//! 8. Catch-all (non-list, non-empty, non-"restore ...") →
//!    `TuiEvent::TextDelta("\nUsage: /checkpoint list | /checkpoint
//!    restore <file_path>\n")` — NOTE: `TextDelta` not `Error`, this is
//!    byte-identical to shipped.
//!
//! `description()` returns `"Create or restore a session checkpoint"` —
//! byte-identical to `declare_handler!` stub at registry.rs:1361.
//! `aliases()` returns `&[]` — shipped stub used the 2-arg form.
//!
//! # R5 — aliases = zero
//!
//! Shipped pre-B21: none (2-arg declare_handler! form). Spec lists
//! none. No aliases added. Matches /fork / /rename / /mcp / /context
//! precedent.
//!
//! # R6 — session_id reuse (no new context.rs snapshot wiring)
//!
//! `CommandContext::session_id: Option<String>` is already populated
//! unconditionally by `build_command_context` per AGS-815 /fork. This
//! ticket REUSES that exact field — there is no `checkpoint_snapshot`
//! type, no context.rs match arm added, no new `build_command_context`
//! wiring. Test fixtures pass `session_id: Some(..)` directly.
//!
//! # R7 — Gates 1-4 double-fire note
//!
//! During the Gates 1-4 window, BOTH the new `CheckpointHandler` (PATH
//! A, via the dispatcher at slash.rs:46) AND the legacy `s if s ==
//! "/checkpoint" || s.starts_with("/checkpoint ")` match arm at
//! slash.rs:452-527 are live. Every `/checkpoint` invocation therefore
//! fires twice — once via the handler and once via the legacy arm.
//! This is the Stage-6 body-migrate protocol: Gate 5 deletes the
//! legacy match arm in a SEPARATE subsequent subagent run (NOT this
//! subagent's responsibility). Do NOT touch slash.rs in this ticket.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/checkpoint` command.
///
/// No aliases. Shipped pre-B21 stub carried none (2-arg
/// declare_handler! form); spec lists none. Matches /fork / /rename /
/// /mcp / /context / /hooks precedent.
pub(crate) struct CheckpointHandler;

impl CheckpointHandler {
    /// Unit-struct constructor. Matches peer body-migrated handlers
    /// (`RenameHandler::new`, `DoctorHandler::new`, `UsageHandler::new`)
    /// even though the unit struct is constructible without it — the
    /// explicit constructor keeps the call site in registry.rs:1467
    /// copy-editable across peers.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for CheckpointHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for CheckpointHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // R6: require session_id. `build_command_context` populates
        // this unconditionally from `SlashCommandContext::session_id`
        // per the AGS-815 fork.rs precedent, so at the real dispatch
        // site this branch never fires. Test fixtures that construct
        // `CommandContext` directly with `session_id: None` will hit
        // this branch and observe an Err — mirroring the
        // `fork_handler_execute_without_session_id_returns_err` and
        // B17 rename pattern.
        let session_id = ctx.session_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "CheckpointHandler invoked without ctx.session_id populated — \
                 build_command_context bug"
            )
        })?;

        // R3: reconstruct the single arg-string from positional tokens.
        // Byte-equivalent to shipped `s.strip_prefix("/checkpoint")
        // .unwrap_or("").trim()` for all inputs.
        let joined = args.join(" ");
        let arg = joined.trim();

        // R4: path reproduced byte-identically from shipped slash.rs:455-458.
        let ckpt_path = archon_session::background::archon_data_dir().join("checkpoints.db");

        if arg == "list" || arg.is_empty() {
            match archon_session::checkpoint::CheckpointStore::open(&ckpt_path) {
                Ok(store) => match store.list_modified(session_id) {
                    Ok(snapshots) if snapshots.is_empty() => {
                        ctx.emit(TuiEvent::TextDelta(
                            "\nNo checkpoints for this session.\n".into(),
                        ));
                    }
                    Ok(snapshots) => {
                        let mut out = String::from("\nCheckpoints:\n");
                        for s in &snapshots {
                            out.push_str(&format!(
                                "  turn {} | {} | {} | {}\n",
                                s.turn_number, s.tool_name, s.file_path, s.timestamp
                            ));
                        }
                        ctx.emit(TuiEvent::TextDelta(out));
                    }
                    Err(e) => {
                        ctx.emit(TuiEvent::Error(format!("Checkpoint list error: {e}")));
                    }
                },
                Err(e) => {
                    ctx.emit(TuiEvent::Error(format!("Checkpoint store error: {e}")));
                }
            }
        } else if let Some(file_path) = arg.strip_prefix("restore").map(|s| s.trim()) {
            if file_path.is_empty() {
                ctx.emit(TuiEvent::Error(
                    "Usage: /checkpoint restore <file_path>".into(),
                ));
            } else {
                match archon_session::checkpoint::CheckpointStore::open(&ckpt_path) {
                    Ok(store) => match store.restore(session_id, file_path) {
                        Ok(()) => {
                            ctx.emit(TuiEvent::TextDelta(format!("\nRestored: {file_path}\n")));
                        }
                        Err(e) => {
                            ctx.emit(TuiEvent::Error(format!("Restore failed: {e}")));
                        }
                    },
                    Err(e) => {
                        ctx.emit(TuiEvent::Error(format!("Checkpoint store error: {e}")));
                    }
                }
            }
        } else {
            ctx.emit(TuiEvent::TextDelta(
                "\nUsage: /checkpoint list | /checkpoint restore <file_path>\n".into(),
            ));
        }

        Ok(())
    }

    fn description(&self) -> &'static str {
        // R4: byte-identical to declare_handler! stub at registry.rs:1361.
        "Create or restore a session checkpoint"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R5: zero aliases. Shipped stub used the 2-arg
        // declare_handler! form (no aliases slice); spec lists none.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B21-CHECKPOINT: tests for /checkpoint body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
