//! Slash command handler. Extracted from main.rs.

use std::path::PathBuf;
// TASK-AGS-POST-6-BODIES-B19-RULES: /rules body migrated to
// src/command/rules.rs (DIRECT-sync-via-MemoryTrait pattern). The
// shipped `use archon_consciousness::rules::RulesEngine;` import is
// removed — the legacy arm at previously :591-706 has been replaced
// with a breadcrumb, and the new RulesHandler constructs
// `RulesEngine::new(memory.as_ref())` inside its own module.
use crate::command::config::handle_config_command;
use archon_llm::effort::EffortState;
use archon_llm::fast_mode::FastModeState;
use archon_tui::app::TuiEvent;
// TASK-AGS-POST-6-BODIES-B15-DOCTOR: /doctor body migrated to
// src/command/doctor.rs (SNAPSHOT-DELEGATE pattern). The shipped
// `use crate::command::doctor::handle_doctor_command;` import is
// removed — the delegate has been deleted, all composition runs
// through `build_doctor_text` from `build_doctor_snapshot` at
// dispatch time, and the sync `DoctorHandler::execute` consumes the
// pre-built `DoctorSnapshot`.
// TASK-AGS-POST-6-FALLTHROUGH: `use anyhow::anyhow;`,
// `use archon_tools::task_manager;`, and
// `use crate::command::registry::CommandContext;` removed — their
// only call sites lived inside the deleted match block.
use crate::slash_context::SlashCommandContext;

/// Handle slash commands. Returns `true` if the command was recognized and handled.
///
/// TASK-SESSION-LOOP-EXTRACT: returns an explicit
/// `Pin<Box<dyn Future<Output = bool> + Send + '_>>` rather than an
/// inferred `impl Future`. Callers reach this from inside the body of
/// `session_loop::run_session_loop` where rustc's higher-ranked-Send
/// inference fails for anonymous async-fn bodies that borrow `&str` /
/// `&SlashCommandContext` across many awaits (rust-lang/rust#102211).
/// The A-2 channel flip resolved the `&Sender<TuiEvent>` HRTB variant
/// but the other borrows remain — the explicit trait object with a
/// `Send + 'a` bound keeps the spawn site concrete.
pub(crate) fn handle_slash_command<'a>(
    input: &'a str,
    _fast_mode: &'a mut FastModeState,
    effort_state: &'a mut EffortState,
    tui_tx: &'a tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    ctx: &'a mut SlashCommandContext,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>> {
    Box::pin(async move {
        // TASK-AGS-POST-6-FALLTHROUGH: match fallthrough block DELETED.
        // All 40 registered primaries (see registry.rs default_registry)
        // now route through real handler modules via Dispatcher::dispatch
        // below. The former 477-line match block carried only ONE live
        // arm (/config — moved to the pre-branch directly below) plus a
        // default `_ => true` pass-through. /compact, /clear, and /export
        // intercepts live upstream in session.rs and never reach this fn.
        // See git log for TASK-AGS-POST-6-FALLTHROUGH commit for the
        // deleted arm lineage.

        // /config async-upstream pre-branch — invoked BEFORE
        // Dispatcher::dispatch because `handle_config_command` is async
        // and requires SlashCommandContext access that is not exposed
        // through CommandContext. ConfigHandler in registry.rs remains a
        // THIN-WRAPPER no-op so Dispatcher::recognizes returns true for
        // /config (consistency with the rest of the catalog).
        if input.trim() == "/config" || input.trim().starts_with("/config ") {
            handle_config_command(input.trim(), tui_tx, ctx).await;
            return true;
        }

        // TASK-AGS-623 dispatcher gate (PATH A hybrid).
        //
        // Every slash input now flows through exactly one `Dispatcher::dispatch
        // call: parser → registry lookup → handler or
        // `TuiEvent::Error("Unknown command: /{name}")` on miss. Recognized
        // commands are fully executed by their registered handlers; there is
        // no longer a legacy match fallthrough. Non-slash / empty / bare-`/`
        // inputs short-circuit with `false` — the same behaviour the
        // TASK-AGS-621 parser gate provided.
        // TASK-AGS-807 snapshot-pattern builder. Pre-populates
        // `CommandContext::status_snapshot` (owned values, no locks) when
        // the primary command resolves to /status or its alias /info.
        // Sync CommandHandler::execute cannot await; the builder bridges
        // that gap here at the dispatch site where .await is legal.
        let mut __cmd_ctx =
            crate::command::context::build_command_context(input, tui_tx.clone(), ctx).await;
        let _ = ctx.dispatcher.dispatch(&mut __cmd_ctx, input);
        // TASK-AGS-808 effect-slot drain. Handlers that need to write to
        // async-guarded shared state (e.g. /model mutating
        // `model_override_shared`) stash a CommandEffect in
        // `pending_effect` synchronously; we consume it with `.take()`
        // here — where `.await` is legal — and apply the mutation via
        // `command::context::apply_effect`. Single-shot by construction.
        if let Some(effect) = __cmd_ctx.pending_effect.take() {
            // TASK-AGS-POST-6-BODIES-B04-DIFF: `tui_tx` threaded into
            // `apply_effect` so the RunGitDiffStat variant can call the
            // existing LIVE `handle_diff_command(tui_tx, &path)` helper
            // at slash.rs:120 without having to clone the sender into the
            // effect variant itself. Prior signature `(effect, slash_ctx)`
            // stays wire-compatible for SetModelOverride (which ignores
            // `tui_tx`).
            crate::command::context::apply_effect(effect, ctx, tui_tx).await;
        }
        // TASK-AGS-POST-6-BODIES-B11-EFFORT: sidecar drain for the local
        // `effort_state: &mut EffortState` parameter. `EffortHandler::execute`
        // stashes BOTH the shared-mutex effect (drained above via
        // `CommandEffect::SetEffortLevelShared` + apply_effect) AND this
        // sidecar slot. The shared-mutex path covers
        // `SlashCommandContext::effort_level_shared`; this drain covers the
        // session-local `EffortState` stack variable that only exists in
        // this function's scope and cannot be written from inside the
        // handler. Single-shot (.take()) by construction; a None here means
        // the handler did not hit the WRITE branch.
        if let Some(level) = __cmd_ctx.pending_effort_set.take() {
            effort_state.set_level(level);
        }
        if !ctx.dispatcher.recognizes(input) {
            return false;
        }

        // Dispatcher recognized + executed. Fall through to true
        // (matches the old default arm's Option-3
        // handler-owns-recognition pattern documented in registry.rs).
        true
    })
}

// ---------------------------------------------------------------------------
// /diff handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_diff_command<'a>(
    tui_tx: &'a tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    working_dir: &'a PathBuf,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        let result = tokio::process::Command::new("git")
            .arg("diff")
            .arg("--stat")
            .current_dir(working_dir)
            .output()
            .await;

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !output.status.success() {
                    if stderr.contains("not a git repository") {
                        let _ =
                            tui_tx.send(TuiEvent::TextDelta("\nNot in a git repository.\n".into()));
                    } else {
                        let _ = tui_tx.send(TuiEvent::Error(format!("git diff failed: {stderr}")));
                    }
                    return;
                }
                if stdout.is_empty() {
                    let _ = tui_tx.send(TuiEvent::TextDelta("\nNo uncommitted changes.\n".into()));
                } else {
                    let _ = tui_tx.send(TuiEvent::TextDelta(format!("\n{stdout}")));
                }
            }
            Err(e) => {
                let _ = tui_tx.send(TuiEvent::Error(format!("Failed to run git: {e}")));
            }
        }
    })
}
