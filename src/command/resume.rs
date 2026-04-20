//! TASK-AGS-810: /resume slash-command handler (body-migrate target,
//! Option C, DIRECT pattern).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs` and the legacy match arm at
//! `src/command/slash.rs:656-726`. Second DIRECT body-migrate under
//! Option C (after AGS-806 /tasks).
//!
//! # Why DIRECT (no snapshot, no effect slot)?
//!
//! The shipped /resume body reads `archon_session` synchronously —
//! every call into `archon_session::storage::SessionStore::open`,
//! `archon_session::search::search_sessions`, and
//! `archon_session::naming::resolve_by_name` is a plain sync function
//! (no `.await`). There are no `tokio::sync::Mutex` guards on the read
//! path and no writes back to `SlashCommandContext` state. Consequently:
//!
//! - NO `ResumeSnapshot` type (nothing to pre-compute inside an async
//!   lock guard, unlike `/status` / `/model` / `/cost`).
//! - NO `CommandContext` field added (nothing to thread through from
//!   the dispatch-site builder — AGS-822 Rule 5 respected: first ticket
//!   that ACTUALLY needs a field is the first ticket that adds it, and
//!   /resume does not).
//! - NO `CommandEffect` variant (the handler never mutates shared
//!   state; it only emits `TuiEvent`s that downstream UI layers own).
//! - NO `build_command_context` match arm change (the builder leaves
//!   every optional field `None` for /resume and /continue /open-session
//!   and that's correct).
//!
//! The sole side effect is `ctx.tui_tx.try_send(TuiEvent::…)` — which is
//! sync and legal inside `CommandHandler::execute`.
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted string is faithful to the deleted slash.rs:656-726
//! body. Concretely:
//! - Empty-arg + zero results -> `TuiEvent::TextDelta("\nNo previous \
//!   sessions found.\n")`
//! - Empty-arg + >=1 results -> `TuiEvent::ShowSessionPicker(entries)`
//!   with `SessionPickerEntry { id, name, turns = message_count / 2,
//!   cost = total_cost, last_active = first 10 chars of last_active }`
//! - Non-empty arg + match -> `TuiEvent::TextDelta(format!("\nSession \
//!   found: {}\nRestart with: archon --resume {}\n", meta.id, meta.id))`
//! - Non-empty arg + no match -> `TuiEvent::TextDelta(format!("\nNo \
//!   session matching '{arg}'. Use /sessions to list.\n"))`
//! - Any search/lookup/store-open error -> `TuiEvent::Error(format!(..))`
//!
//! The one emission-primitive change is `tui_tx.send(..).await` (async)
//! -> `ctx.tui_tx.try_send(..)` (sync), matching AGS-806/807/808/809
//! precedent. `/resume` output is best-effort UI — dropping a message
//! under channel backpressure is preferable to stalling the dispatcher.
//!
//! # Aliases
//!
//! Shipped pre-AGS-810: `[continue]` only. Spec TASK-AGS-810 validation
//! criterion 4 wants `[continue, open-session]`. `open-session` is
//! collision-free against the 38 primaries and the pre-AGS-810 alias
//! set (`cls, save, ctx, mem, ?, h, todo, ps, jobs, info, m,
//! switch-model, stop, abort, billing, continue` — none conflict).
//! Extension applied. Alias total grows by +1.
//!
//! # SessionPickerEntry path
//!
//! `archon_tui::events::SessionPickerEntry` is the canonical path after
//! TUI-330. `archon_tui::app::SessionPickerEntry` still compiles via a
//! `pub use crate::events::{…}` re-export in `crates/archon-tui/src/
//! app.rs`. Either path works; the canonical `events` path is used here
//! so future readers do not have to chase the re-export. `TuiEvent`
//! stays on `archon_tui::app` to match the AGS-806/807/808/809 import
//! style — switching TuiEvent's path is out of scope.

use archon_tui::app::TuiEvent;
use archon_tui::events::SessionPickerEntry;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/resume` command.
///
/// Aliases: `[continue, open-session]`. Shipped pre-AGS-810 handler
/// had `[continue]` only; spec validation criterion 4 asks for
/// `[continue, open-session]`. `open-session` is collision-free against
/// the 38 primaries and existing 16 aliases — extension applied.
pub(crate) struct ResumeHandler;

impl CommandHandler for ResumeHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // Shipped body used `s.strip_prefix("/resume").trim()` which
        // consumed the entire remainder as a single string. Since
        // `resolve_by_name` expects a session name or ID prefix (no
        // embedded whitespace by contract), `args.first()` is a
        // semantically equivalent, parser-friendly input. Preserves
        // behaviour for every shipped call site (single-token arg).
        let arg = args
            .first()
            .map(|s| s.as_str())
            .unwrap_or("")
            .trim();

        // 1. Open the session store. Every downstream branch depends on
        //    a valid `SessionStore`; a failure here surfaces as a user-
        //    facing `TuiEvent::Error`, matching the shipped Err arm at
        //    slash.rs:719-722.
        let db_path = archon_session::storage::default_db_path();
        match archon_session::storage::SessionStore::open(&db_path) {
            Ok(store) => {
                if arg.is_empty() {
                    // 2a. No-arg path: show interactive session picker.
                    //     Shipped behaviour at slash.rs:662-692.
                    let query =
                        archon_session::search::SessionSearchQuery::default();
                    match archon_session::search::search_sessions(&store, &query)
                    {
                        Ok(results) => {
                            if results.is_empty() {
                                let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                                    "\nNo previous sessions found.\n".into(),
                                ));
                            } else {
                                // Map SessionMetadata -> SessionPickerEntry
                                // verbatim from slash.rs:674-683.
                                let entries: Vec<SessionPickerEntry> = results
                                    .iter()
                                    .map(|m| SessionPickerEntry {
                                        id: m.id.clone(),
                                        name: m
                                            .name
                                            .clone()
                                            .unwrap_or_default(),
                                        turns: m.message_count / 2,
                                        cost: m.total_cost,
                                        last_active: m
                                            .last_active
                                            .chars()
                                            .take(10)
                                            .collect(),
                                    })
                                    .collect();
                                let _ = ctx
                                    .tui_tx
                                    .try_send(TuiEvent::ShowSessionPicker(entries));
                            }
                        }
                        Err(e) => {
                            let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                                format!("Search failed: {e}"),
                            ));
                        }
                    }
                } else {
                    // 2b. Named-arg path: resolve by name or ID prefix.
                    //     Shipped behaviour at slash.rs:694-716.
                    match archon_session::naming::resolve_by_name(&store, arg) {
                        Ok(Some(meta)) => {
                            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                                format!(
                                    "\nSession found: {}\nRestart with: \
                                     archon --resume {}\n",
                                    meta.id, meta.id
                                ),
                            ));
                        }
                        Ok(None) => {
                            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(
                                format!(
                                    "\nNo session matching '{arg}'. Use \
                                     /sessions to list.\n"
                                ),
                            ));
                        }
                        Err(e) => {
                            let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                                format!("Lookup failed: {e}"),
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                    "Session store error: {e}"
                )));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        "Resume a previous session by id"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["continue", "open-session"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-810: tests for /resume slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel.
    /// /resume is a DIRECT-pattern handler — no snapshot, no effect
    /// slot — so every optional field stays `None`. Mirrors the
    /// make_ctx fixtures in task.rs / cost.rs / model.rs / status.rs.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(16);
        (
            CommandContext {
                tui_tx: tx,
                status_snapshot: None,
                model_snapshot: None,
                cost_snapshot: None,
                // TASK-AGS-811: /resume tests never exercise /mcp paths — None.
                mcp_snapshot: None,
                // TASK-AGS-814: /resume tests never exercise /context paths — None.
                context_snapshot: None,
                // TASK-AGS-815: /resume tests never exercise /fork paths — None.
                session_id: None,
                // TASK-AGS-817: /resume tests never exercise /memory paths — None.
                memory: None,
                // TASK-AGS-POST-6-BODIES-B13-GARDEN: /resume tests never exercise /garden paths — None.
                garden_config: None,
                // TASK-AGS-POST-6-BODIES-B01-FAST: /resume tests never exercise /fast paths — None.
                fast_mode_shared: None,
                // TASK-AGS-POST-6-BODIES-B02-THINKING: /resume tests never exercise /thinking paths — None.
                show_thinking: None,
                // TASK-AGS-POST-6-BODIES-B04-DIFF: /resume tests never exercise /diff paths — None.
                working_dir: None,
                // TASK-AGS-POST-6-BODIES-B06-HELP: /resume tests never exercise /help paths — None.
                skill_registry: None,
                // TASK-AGS-POST-6-BODIES-B08-DENIALS: /resume tests never exercise /denials paths — None.
                denial_snapshot: None,
                effort_snapshot: None,
                permissions_snapshot: None,
                copy_snapshot: None,
                doctor_snapshot: None,
                usage_snapshot: None,
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    #[test]
    fn resume_handler_description_matches() {
        let h = ResumeHandler;
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("resume") || desc.contains("session"),
            "ResumeHandler description should reference 'resume' or \
             'session', got: {}",
            h.description()
        );
    }

    #[test]
    fn resume_handler_aliases_are_continue_and_open_session() {
        let h = ResumeHandler;
        assert_eq!(
            h.aliases(),
            &["continue", "open-session"],
            "ResumeHandler aliases must be [continue, open-session] per \
             AGS-810 spec validation criterion 4"
        );
    }

    /// Smoke test: execute() with zero args must return Ok(()) regardless
    /// of whether the default DB exists on disk. Branches:
    ///   - DB missing: `SessionStore::open` returns Err -> emit
    ///     `TuiEvent::Error`, still Ok(()).
    ///   - DB present + empty: emit `TuiEvent::TextDelta("No previous
    ///     sessions found.")`, Ok(()).
    ///   - DB present + results: emit `TuiEvent::ShowSessionPicker`,
    ///     Ok(()).
    /// We assert Ok(()) invariant only, because test environments have
    /// varying DB state and we do not want to couple this test to the
    /// operator's `~/.archon/sessions.db`.
    #[test]
    fn resume_handler_execute_with_empty_args_uses_default_db_path() {
        let (mut ctx, _rx) = make_ctx();
        let h = ResumeHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "ResumeHandler::execute(empty args) must return Ok(()) \
             regardless of session DB state, got: {res:?}"
        );
    }

    /// Executing with a clearly-bogus arg must still return Ok(()). The
    /// specific emitted event depends on whether the default DB opens
    /// (emits TextDelta with "No session matching …") or not (emits
    /// Error with "Session store error …") — both are valid branches.
    /// Test pins only the handler-return invariant.
    #[test]
    fn resume_handler_execute_with_unknown_name_emits_text_delta_or_error() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ResumeHandler;
        let res = h.execute(
            &mut ctx,
            &["definitely-not-a-real-session-xyz-810".to_string()],
        );
        assert!(
            res.is_ok(),
            "ResumeHandler::execute(unknown name) must return Ok(()) \
             regardless of session DB state, got: {res:?}"
        );
        // Drain whatever was emitted — at least one event must fire
        // (either the TextDelta no-match branch or the Error store-
        // open branch). Dropping all events under backpressure is
        // possible in principle but not in a 16-cap fresh channel.
        let mut emitted = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                TuiEvent::TextDelta(_) | TuiEvent::Error(_) => {
                    emitted = true;
                }
                other => panic!(
                    "ResumeHandler emitted unexpected event variant: \
                     {other:?}"
                ),
            }
        }
        assert!(
            emitted,
            "ResumeHandler must emit at least one TextDelta or Error \
             event for a named-arg miss path"
        );
    }
}
