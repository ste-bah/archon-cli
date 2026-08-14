use archon_tui::app::TuiEvent;

use crate::command::registry::CommandEffect;
use crate::slash_context::SlashCommandContext;

/// Apply a [`CommandEffect`] produced by a handler by awaiting the
/// write-back on the appropriate `SlashCommandContext` field.
///
/// TASK-AGS-808 introduced this helper to bridge the sync
/// `CommandHandler` boundary with `tokio::sync::Mutex` writes that
/// shipped bodies performed inline. Handlers stash an effect in
/// `CommandContext::pending_effect` synchronously; `slash.rs::
/// handle_slash_command` takes the value (consuming the slot via
/// `.take()`) after `Dispatcher::dispatch` returns and calls
/// `apply_effect`, which awaits the mutex write before returning
/// control to the main input loop.
///
/// Future body-migrate tickets add new `CommandEffect` variants and
/// extend the match below.
pub(crate) fn apply_effect<'a>(
    effect: CommandEffect,
    slash_ctx: &'a SlashCommandContext,
    tui_tx: &'a archon_tui::event_channel::TuiEventSender,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        match effect {
            CommandEffect::SetModelOverride(resolved) => {
                *slash_ctx.model_override_shared.lock().await = resolved;
            }
            // TASK-AGS-POST-6-BODIES-B04-DIFF: spawn `git diff --stat` via
            // the existing LIVE `handle_diff_command` helper at slash.rs:120.
            // Byte-identity of emitted TuiEvent strings (TextDelta for
            // "Not in a git repository.", "No uncommitted changes.",
            // stdout wrap; Error for spawn failures and git-failure
            // exit codes) is preserved by call-site reuse — this match
            // arm does not duplicate any of the five emission branches.
            // The `_` discard on `slash_ctx` is intentional — /diff does
            // not read or mutate SlashCommandContext state; the working
            // directory was already captured at build-time in the effect
            // variant.
            CommandEffect::RunGitDiffStat(path) => {
                let _ = slash_ctx;
                crate::command::slash::handle_diff_command(tui_tx, &path).await;
            }
            // TASK-AGS-POST-6-BODIES-B10-ADDDIR: await the mutex push on
            // slash_ctx.extra_dirs and emit the tracing::info! record. Byte-
            // identity with shipped slash.rs:679-683 preserved — same tracing
            // call, same log fields (`dir` kv pair with `%path.display()`
            // formatter; same message literal "added working directory via
            // /add-dir"). `tui_tx` is unused in this arm — the confirmation
            // TextDelta is emitted by the handler via try_send BEFORE
            // apply_effect runs (see src/command/add_dir.rs R6 order-
            // semantics-swap note for rationale).
            CommandEffect::AddExtraDir(path) => {
                let _ = tui_tx;
                // Clone so the tracing::info! after the push can still
                // borrow the path. Order preserves shipped slash.rs:679-683
                // exactly — push FIRST, log SECOND.
                slash_ctx.extra_dirs.lock().await.push(path.clone());
                tracing::info!(dir = %path.display(), "added working directory via /add-dir");
            }
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: await the mutex write on
            // `slash_ctx.effort_level_shared`. Byte-identity with shipped
            // slash.rs:109 preserved (`*ctx.effort_level_shared.lock().await =
            // level;`). `tui_tx` is unused in this arm — the confirmation
            // TextDelta is emitted by the handler via `try_send` BEFORE
            // apply_effect runs. The companion session-local write to
            // `&mut EffortState` is NOT applied here; the slash.rs dispatch
            // site drains `CommandContext::pending_effort_set` AFTER this
            // call returns. The tracing::info! record is an additive
            // observability line — shipped code had no /effort tracing, so
            // this is new but invariant-preserving.
            CommandEffect::SetEffortLevelShared(level) => {
                let _ = tui_tx;
                *slash_ctx.effort_level_shared.lock().await = level;
                tracing::info!(level = %level, "set effort level via /effort");
            }
            // TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: await the mutex write
            // on `slash_ctx.permission_mode` AND emit
            // `TuiEvent::PermissionModeChanged(resolved)` via
            // `tui_tx.send(..).await` (apply_effect is async, so .await is
            // legal — the event MUST be awaited to match shipped
            // emission-after-write ordering at slash.rs:320-323). Byte-
            // identity with shipped slash.rs:319-323 preserved
            // (`*ctx.permission_mode.lock().await = resolved.clone();
            // tui_tx.send(TuiEvent::PermissionModeChanged(resolved.clone()))
            // .await;`). The confirmation TextDelta
            // ("\nPermission mode set to {resolved}.\n") is emitted by
            // the handler via `try_send` BEFORE apply_effect runs (see
            // src/command/permissions.rs R6 order-semantics-swap note for
            // rationale — matches B10/B11 precedent). The tracing::info!
            // record is an additive observability line — shipped code had
            // no /permissions tracing, so this is new but invariant-
            // preserving.
            CommandEffect::SetPermissionMode(resolved) => {
                let previous_mode = {
                    let mut mode = slash_ctx.permission_mode.lock().await;
                    let previous = mode.clone();
                    *mode = resolved.clone();
                    previous
                };
                if resolved == archon_permissions::mode::PermissionMode::Default.as_str() {
                    let mut plan_mode_state = slash_ctx.plan_mode_state.lock().await;
                    if plan_mode_state.entered_via
                        == Some(archon_core::agent::plan_mode_state::PlanEntryPath::SlashCommand)
                    {
                        plan_mode_state.previous_permission_mode = None;
                        plan_mode_state.active_plan_id = None;
                        plan_mode_state.entered_via = None;
                    }
                }
                crate::runtime::permission_events::record_permission_mode_event(
                    slash_ctx.governed_learning_db.as_ref(),
                    Some(&slash_ctx.session_id),
                    Some(&previous_mode),
                    &resolved,
                    "mode_changed",
                    "slash_permissions",
                );
                let _ = tui_tx
                    .send_async(TuiEvent::PermissionModeChanged(resolved.clone()))
                    .await;
                tracing::info!(mode = %resolved, "set permission mode via /permissions");
            }
            CommandEffect::EnterPlanMode { previous_mode } => {
                let mut plan_mode_state = slash_ctx.plan_mode_state.lock().await;
                let mut mode = slash_ctx.permission_mode.lock().await;
                let previous = mode.clone();
                plan_mode_state.record_entry(
                    previous_mode,
                    archon_core::agent::plan_mode_state::PlanEntryPath::SlashCommand,
                );
                *mode = archon_permissions::mode::PermissionMode::Plan.to_string();
                drop(mode);
                drop(plan_mode_state);
                crate::runtime::permission_events::record_permission_mode_event(
                    slash_ctx.governed_learning_db.as_ref(),
                    Some(&slash_ctx.session_id),
                    Some(&previous),
                    archon_permissions::mode::PermissionMode::Plan.as_str(),
                    "mode_changed",
                    "slash_plan",
                );
                let _ = tui_tx
                    .send_async(TuiEvent::PermissionModeChanged(
                        archon_permissions::mode::PermissionMode::Plan.to_string(),
                    ))
                    .await;
                tracing::info!("entered plan mode via /plan");
            }
            CommandEffect::StartPipelineWork(work) => {
                crate::command::pipeline_slash::start_pipeline_work(slash_ctx, tui_tx, work).await;
            }
            // FCDP-DRAFT (/draft), from PR #51. Spawns a DETACHED task that
            // streams the subprocess output as TextDelta: drafting takes
            // minutes, so it must not block this inline await.
            CommandEffect::RunDraft {
                pack,
                workdir,
                model,
                gate_config,
                cwd,
            } => {
                crate::command::slash::spawn_draft_command_tui(
                    tui_tx.clone(),
                    pack,
                    workdir,
                    model,
                    gate_config,
                    cwd,
                );
            }
        }
    })
}
