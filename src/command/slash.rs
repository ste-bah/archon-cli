//! Slash command handler. Extracted from main.rs.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
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
    tui_tx: &'a archon_tui::event_channel::TuiEventSender,
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
        // Dispatcher::dispatch because `handle_config_command` is async and
        // requires SlashCommandContext access that is not exposed through
        // CommandContext. `ConfigHandler` stays registered so
        // `Dispatcher::recognizes` and `/help` still know the command.
        //
        // Matched under every spelling the registry resolves to
        // `ConfigHandler`, not just the literal `/config`: the `settings` and
        // `prefs` aliases used to miss this branch and reach a handler whose
        // body was `Ok(())`, so they printed nothing and changed nothing.
        if let Some(args) = crate::command::config::command_args(input) {
            handle_config_command(args, tui_tx, ctx).await;
            return true;
        }

        // TASK-AGS-623 dispatcher gate (PATH A hybrid).
        //
        // Primary commands flow through `Dispatcher::dispatch`: parser →
        // registry lookup → handler. Skill commands fall through to the
        // session-loop skill executor below, so `/to-prd` and other built-in
        // skills do not get a premature primary-command "Unknown command"
        // diagnostic.
        let dispatcher_recognizes = ctx.dispatcher.recognizes(input);
        if !dispatcher_recognizes && resolves_skill_input(input, ctx.skill_registry.as_ref()) {
            return false;
        }

        // TASK-AGS-807 snapshot-pattern builder. Pre-populates
        // `CommandContext::status_snapshot` (owned values, no locks) when
        // the primary command resolves to /status or its alias /info.
        // Sync CommandHandler::execute cannot await; the builder bridges
        // that gap here at the dispatch site where .await is legal.
        let mut __cmd_ctx =
            crate::command::context::build_command_context(input, tui_tx.clone(), ctx).await;
        let dispatch_result = ctx.dispatcher.dispatch(&mut __cmd_ctx, input);
        let events_flushed = match __cmd_ctx.flush_events().await {
            Ok(()) => true,
            Err(error) => {
                tracing::error!(
                    target: "archon_cli::command::tui",
                    %error,
                    "failed to flush command TUI events"
                );
                false
            }
        };
        if dispatch_result.is_err() {
            return dispatcher_recognizes;
        }
        // TASK-AGS-808 effect-slot drain. Handlers that need to write to
        // async-guarded shared state (e.g. /model mutating
        // `model_override_shared`) stash a CommandEffect in
        // `pending_effect` synchronously; we consume it with `.take()`
        // here — where `.await` is legal — and apply the mutation via
        // `command::context::apply_effect`. Single-shot by construction.
        if let Some(effect) = __cmd_ctx.pending_effect.take()
            && (events_flushed
                || !matches!(
                    effect,
                    crate::command::registry::CommandEffect::StartPipelineWork(_)
                ))
        {
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
        if !dispatcher_recognizes {
            return false;
        }

        // Dispatcher recognized + executed. Fall through to true
        // (matches the old default arm's Option-3
        // handler-owns-recognition pattern documented in registry.rs).
        true
    })
}

fn resolves_skill_input(input: &str, registry: &archon_core::skills::SkillRegistry) -> bool {
    archon_core::skills::parser::parse_slash_command(input)
        .map(|(name, _)| registry.resolve(&name).is_some())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// /diff handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_diff_command<'a>(
    tui_tx: &'a archon_tui::event_channel::TuiEventSender,
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
                        let _ = tui_tx
                            .send_async(TuiEvent::TextDelta("\nNot in a git repository.\n".into()))
                            .await;
                    } else {
                        let _ = tui_tx
                            .send_async(TuiEvent::Error(format!("git diff failed: {stderr}")))
                            .await;
                    }
                    return;
                }
                if stdout.is_empty() {
                    let _ = tui_tx
                        .send_async(TuiEvent::TextDelta("\nNo uncommitted changes.\n".into()))
                        .await;
                } else {
                    let _ = tui_tx
                        .send_async(TuiEvent::TextDelta(format!("\n{stdout}")))
                        .await;
                }
            }
            Err(e) => {
                let _ = tui_tx
                    .send_async(TuiEvent::Error(format!("Failed to run git: {e}")))
                    .await;
            }
        }
    })
}

// ---------------------------------------------------------------------------
// /draft handler (streaming subprocess)
// ---------------------------------------------------------------------------

/// Spawn the FCDP drafting protocol as a DETACHED streaming subprocess.
///
/// `/draft` (via `CommandEffect::RunDraft` → `apply_effect`) calls this. Unlike
/// `handle_diff_command`, a draft runs for MINUTES, so this must NOT block the
/// inline `apply_effect` await in `handle_slash_command`. It runs the same
/// `archon` binary's `draft` subcommand (`current_exe()`) with piped stdout/
/// stderr, `tokio::spawn`s a task that streams each output line to the TUI as a
/// `TextDelta`, and returns immediately. Out-of-process also keeps the CLI
/// handler's `println!`/`eprintln!` off the TUI's terminal.
pub(crate) fn spawn_draft_command_tui(
    tui_tx: archon_tui::event_channel::TuiEventSender,
    pack: PathBuf,
    workdir: PathBuf,
    model: String,
    gate_config: Option<PathBuf>,
    cwd: PathBuf,
) {
    tokio::spawn(async move {
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                let _ = tui_tx.send(TuiEvent::Error(format!(
                    "archon draft: cannot locate archon binary: {e}"
                )));
                return;
            }
        };
        let mut cmd = tokio::process::Command::new(exe);
        cmd.arg("draft")
            .arg(&pack)
            .arg(&workdir)
            .arg("--model")
            .arg(&model);
        if let Some(gc) = &gate_config {
            cmd.arg("--gate-config").arg(gc);
        }
        cmd.current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = tui_tx.send(TuiEvent::Error(format!("archon draft: spawn failed: {e}")));
                return;
            }
        };
        stream_child_to_tui(child, tui_tx).await;
    });
}

/// Stream a spawned child's stdout + stderr into the TUI line-by-line, then
/// await exit and emit a terminal completion (`[/draft complete]`) or error
/// event. Extracted from `spawn_draft_command_tui` so the streaming/wait
/// plumbing can be exercised against a controlled subprocess in tests.
async fn stream_child_to_tui(
    mut child: tokio::process::Child,
    tui_tx: archon_tui::event_channel::TuiEventSender,
) {
    // Stream stdout + stderr concurrently, line by line, into the TUI.
    let out = child.stdout.take();
    let err = child.stderr.take();
    let tx_out = tui_tx.clone();
    let out_task = tokio::spawn(async move {
        if let Some(s) = out {
            let mut lines = BufReader::new(s).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_out.send(TuiEvent::TextDelta(format!("{line}\n")));
            }
        }
    });
    let tx_err = tui_tx.clone();
    let err_task = tokio::spawn(async move {
        if let Some(s) = err {
            let mut lines = BufReader::new(s).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_err.send(TuiEvent::TextDelta(format!("{line}\n")));
            }
        }
    });
    let _ = out_task.await;
    let _ = err_task.await;

    match child.wait().await {
        Ok(status) if status.success() => {
            let _ = tui_tx.send(TuiEvent::TextDelta("\n[/draft complete]\n".to_string()));
        }
        Ok(status) => {
            let _ = tui_tx.send(TuiEvent::Error(format!(
                "archon draft exited with {status}"
            )));
        }
        Err(e) => {
            let _ = tui_tx.send(TuiEvent::Error(format!("archon draft: wait failed: {e}")));
        }
    }
}

#[cfg(test)]
#[path = "slash_plan_mode_integrated_live_smoke.rs"]
mod plan_mode_integrated_live_smoke;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stream_child_to_tui_streams_stdout_stderr_and_completion() {
        let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel();
        let child = tokio::process::Command::new(archon_shell::resolve_posix_shell())
            .arg("-c")
            .arg("echo out-line; echo err-line 1>&2")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sh");
        stream_child_to_tui(child, tx).await;

        let mut joined = String::new();
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(t) = ev {
                joined.push_str(&t);
            }
        }
        assert!(joined.contains("out-line"), "stdout must stream: {joined}");
        assert!(joined.contains("err-line"), "stderr must stream: {joined}");
        assert!(
            joined.contains("[/draft complete]"),
            "success must emit the completion event: {joined}"
        );
    }

    #[tokio::test]
    async fn stream_child_to_tui_emits_error_on_nonzero_exit() {
        let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel();
        let child = tokio::process::Command::new(archon_shell::resolve_posix_shell())
            .arg("-c")
            .arg("exit 3")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sh");
        stream_child_to_tui(child, tx).await;

        let mut saw_error = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::Error(m) = ev
                && m.contains("exited with")
            {
                saw_error = true;
            }
        }
        assert!(saw_error, "non-zero exit must emit an Error event");
    }

    #[test]
    fn resolves_builtin_prd_skills_for_fallback() {
        let registry = archon_core::skills::builtin::register_builtins();

        assert!(resolves_skill_input("/to-prd Build a thing", &registry));
        assert!(resolves_skill_input(
            "/prd-to-spec prds/example/PRD.md",
            &registry
        ));
        assert!(resolves_skill_input("/prd Build a thing", &registry));
    }

    #[test]
    fn unknown_slash_input_is_not_a_skill_fallback_candidate() {
        let registry = archon_core::skills::builtin::register_builtins();

        assert!(!resolves_skill_input("/not-a-real-command", &registry));
        assert!(!resolves_skill_input("not a slash command", &registry));
    }
}
