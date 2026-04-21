//! TASK-AGS-807: async `CommandContext` builder (snapshot pattern).
//!
//! `CommandHandler::execute` is SYNC (Q1=A invariant). The shipped
//! `/status` body relies on four `tokio::sync::Mutex` guards acquired via
//! `.lock().await`. To bridge the gap, this module constructs a
//! [`CommandContext`] at the dispatch site WHERE `.await` is allowed and
//! pre-populates an owned [`StatusSnapshot`] when the primary command
//! resolves to `/status` (or its alias `/info`). Every other command
//! gets `status_snapshot = None`, so there is zero extra lock traffic
//! for unrelated slash inputs.
//!
//! The builder does not take ownership of `SlashCommandContext`; it
//! borrows it for the duration of the snapshot read and returns a
//! self-contained [`CommandContext`] with only the values the sync
//! handler will need.

use std::sync::Arc;

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, Registry};
use crate::command::{
    context_cmd, copy, cost, denials, doctor, effort, mcp, model, permissions,
    status, usage,
};
use crate::slash_context::SlashCommandContext;

/// Build the per-dispatch [`CommandContext`] for the supplied slash
/// `input`. Awaits the lock-protected shared state ONLY when the primary
/// command resolves to a handler that consumes one of the typed
/// snapshots (currently: `/status` -> [`status::StatusSnapshot`],
/// `/model` -> [`model::ModelSnapshot`], `/cost` ->
/// [`cost::CostSnapshot`]). Other primaries observe every optional
/// field as `None` and pay zero lock traffic.
///
/// # Panics
///
/// Does not panic. Parse failures / unknown names result in a
/// `CommandContext` with every optional field set to `None`; the
/// dispatcher downstream will surface "Unknown command" or a parse
/// error via its own path.
pub(crate) async fn build_command_context(
    input: &str,
    tui_tx: tokio::sync::mpsc::Sender<TuiEvent>,
    slash_ctx: &SlashCommandContext,
) -> CommandContext {
    let mut ctx = CommandContext {
        tui_tx,
        status_snapshot: None,
        model_snapshot: None,
        cost_snapshot: None,
        mcp_snapshot: None,
        context_snapshot: None,
        // TASK-AGS-815: DIRECT-pattern field. Populated UNCONDITIONALLY
        // here (not gated on the primary name, unlike the snapshot
        // fields above). Every command observes the current session id
        // via `ctx.session_id`; /fork is the first consumer but any
        // future DIRECT handler that needs the id can read it without a
        // builder match arm. Clone is a single String alloc per
        // dispatch — cheaper than stashing an `Arc<str>` threaded
        // through `SlashCommandContext`.
        session_id: Some(slash_ctx.session_id.clone()),
        // TASK-AGS-817: /memory DIRECT-pattern consumer. Populated
        // UNCONDITIONALLY here (not gated on the primary name, same
        // as AGS-815 session_id). `Arc<dyn MemoryTrait>` is cheap to
        // clone (~8 bytes + atomic refcount increment); every future
        // handler that needs a memory handle inherits this field for
        // free without a per-command builder match arm.
        memory: Some(Arc::clone(&slash_ctx.memory)),
        // TASK-AGS-POST-6-BODIES-B13-GARDEN: /garden DIRECT-pattern
        // consumer. Populated UNCONDITIONALLY here (not gated on the
        // primary name, same as AGS-815 session_id and AGS-817 memory
        // — GardenConfig is cheap to clone, small fixed-size struct
        // of numeric thresholds with no Arc/heap beyond the struct
        // itself). `/garden` (default branch) reads it to pass
        // `&GardenConfig` into the sync
        // `archon_memory::garden::consolidate(&dyn MemoryTrait,
        // &GardenConfig)` entry point; `/garden stats` does not read it.
        garden_config: Some(slash_ctx.garden_config.clone()),
        // TASK-AGS-POST-6-BODIES-B01-FAST: /fast DIRECT-pattern
        // consumer. Populated UNCONDITIONALLY here (not gated on the
        // primary name, same as AGS-815 session_id and AGS-817 memory).
        // `Arc<AtomicBool>` is cheap to clone (~8 bytes + atomic
        // refcount increment); the handler reads + atomically stores
        // through it to toggle fast mode.
        fast_mode_shared: Some(Arc::clone(&slash_ctx.fast_mode_shared)),
        // TASK-AGS-POST-6-BODIES-B02-THINKING: /thinking DIRECT-pattern
        // consumer. Populated UNCONDITIONALLY here (not gated on the
        // primary name, same as AGS-815 session_id, AGS-817 memory,
        // and B01-FAST fast_mode_shared). `Arc<AtomicBool>` is cheap
        // to clone (~8 bytes + atomic refcount increment); the handler
        // atomically stores the new state from the parsed
        // on/off/empty subcommand.
        show_thinking: Some(Arc::clone(&slash_ctx.show_thinking)),
        // TASK-AGS-POST-6-BODIES-B04-DIFF: /diff DIRECT-with-effect-
        // pattern consumer. Populated UNCONDITIONALLY here (not gated
        // on the primary name, same as AGS-815 session_id, AGS-817
        // memory, B01-FAST fast_mode_shared, and B02-THINKING
        // show_thinking). Cloning a `PathBuf` is cheap; the handler
        // clones it again into `CommandEffect::RunGitDiffStat` so the
        // effect carries owned data (no borrow on `SlashCommandContext`
        // across the effect-slot boundary).
        working_dir: Some(slash_ctx.working_dir.clone()),
        // TASK-AGS-POST-6-BODIES-B06-HELP: /help DIRECT-pattern consumer.
        // Populated UNCONDITIONALLY here (not gated on the primary name,
        // same as AGS-815 session_id, AGS-817 memory, B01-FAST
        // fast_mode_shared, B02-THINKING show_thinking, and B04-DIFF
        // working_dir). `Arc<SkillRegistry>` is cheap to clone (~8 bytes
        // + atomic refcount increment); the handler reads it via
        // `SkillRegistry::format_help()` / `format_skill_help()`.
        skill_registry: Some(Arc::clone(&slash_ctx.skill_registry)),
        // TASK-AGS-POST-6-BODIES-B08-DENIALS: SNAPSHOT-pattern field
        // (READ-only /denials). Initialised to `None` here; populated
        // BELOW in the `match primary.as_deref()` block only when the
        // primary resolves to `/denials`. Unlike DIRECT-pattern fields
        // (session_id/memory/fast_mode_shared/show_thinking/
        // working_dir/skill_registry) which populate unconditionally,
        // SNAPSHOT fields gate on the primary to avoid unnecessary
        // lock traffic on `denial_log` when the command is not
        // /denials.
        denial_snapshot: None,
        // TASK-AGS-POST-6-BODIES-B11-EFFORT: SNAPSHOT-pattern field
        // (READ-only /effort). Initialised to `None` here; populated
        // BELOW in the `match primary.as_deref()` block only when the
        // primary resolves to `/effort`. Mirrors AGS-807 status /
        // AGS-808 model / B08 denials snapshot gating rule.
        effort_snapshot: None,
        // TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: SNAPSHOT-pattern field
        // (HYBRID — READ side + bypass-allow guard for /permissions).
        // Initialised to `None` here; populated BELOW in the
        // `match primary.as_deref()` block only when the primary
        // resolves to `/permissions`. Mirrors AGS-807 status /
        // AGS-808 model / B08 denials / B11 effort snapshot gating rule.
        permissions_snapshot: None,
        // TASK-AGS-POST-6-BODIES-B14-COPY: SNAPSHOT-pattern field
        // (READ-only /copy). Initialised to `None` here; populated
        // BELOW in the `match primary.as_deref()` block only when the
        // primary resolves to `/copy`. Mirrors AGS-807 status /
        // AGS-808 model / B08 denials / B11 effort / B12 permissions
        // snapshot gating rule.
        copy_snapshot: None,
        // TASK-AGS-POST-6-BODIES-B15-DOCTOR: SNAPSHOT-DELEGATE field
        // (READ-only /doctor). Initialised to `None` here; populated
        // BELOW in the `match primary.as_deref()` block only when the
        // primary resolves to `/doctor`. Mirrors AGS-807 status /
        // AGS-808 model / B08 denials / B11 effort / B12 permissions /
        // B14 copy snapshot gating rule.
        doctor_snapshot: None,
        // TASK-AGS-POST-6-BODIES-B16-USAGE: SNAPSHOT field (READ-only
        // /usage). Initialised to `None` here; populated BELOW in the
        // `match primary.as_deref()` block only when the primary
        // resolves to `/usage`. Mirrors AGS-807 status / AGS-809 cost /
        // B08 denials / B11 effort / B12 permissions / B14 copy / B15
        // doctor snapshot gating rule.
        usage_snapshot: None,
        // TASK-AGS-POST-6-BODIES-B20-RELOAD: /reload DIRECT-pattern
        // consumer. Populated UNCONDITIONALLY here (not gated on the
        // primary name, same as AGS-815 session_id, AGS-817 memory,
        // B01-FAST fast_mode_shared, B02-THINKING show_thinking,
        // B04-DIFF working_dir, B06-HELP skill_registry, and B13-GARDEN
        // garden_config). `PathBuf` clone is cheap (one Vec<u8> alloc);
        // the handler passes `&[PathBuf]` into the sync
        // `archon_core::config_watcher::force_reload(config_paths:
        // &[PathBuf], current: &ArchonConfig)` entry point via
        // `std::slice::from_ref(config_path)`.
        config_path: Some(slash_ctx.config_path.clone()),
        // TASK-AGS-POST-6-BODIES-B22-LOGIN: /login DIRECT-pattern
        // consumer. Populated UNCONDITIONALLY here (not gated on the
        // primary name, same as AGS-815 session_id, AGS-817 memory,
        // B01-FAST fast_mode_shared, B02-THINKING show_thinking,
        // B04-DIFF working_dir, B06-HELP skill_registry, B13-GARDEN
        // garden_config, and B20-RELOAD config_path). `String` clone
        // is cheap (one heap alloc); the handler includes the label
        // in the emitted `TuiEvent::TextDelta` message.
        auth_label: Some(slash_ctx.auth_label.clone()),
        pending_effect: None,
        // TASK-AGS-POST-6-BODIES-B11-EFFORT: SIDECAR slot for the
        // session-local `&mut EffortState` write. Initialised to
        // `None` here; populated (in lockstep with `pending_effect`)
        // by `EffortHandler::execute` on the WRITE branch; drained at
        // the slash.rs dispatch site AFTER `apply_effect` returns.
        pending_effort_set: None,
    };

    // Resolve the primary command name (alias-aware) so "/info" routes
    // to the same snapshot path as "/status" and "/m" / "/switch-model"
    // route to the same snapshot path as "/model". The registry holds
    // the canonical alias→primary mapping; we delegate rather than
    // duplicating the table here.
    let primary = resolve_primary_from_input(input, slash_ctx.registry.as_ref());

    match primary.as_deref() {
        Some("status") => {
            ctx.status_snapshot =
                Some(status::build_status_snapshot(slash_ctx).await);
        }
        Some("model") => {
            ctx.model_snapshot =
                Some(model::build_model_snapshot(slash_ctx).await);
        }
        Some("cost") => {
            // TASK-AGS-809 snapshot population. /cost is read-only,
            // so there is no paired `apply_effect` branch. The alias
            // `billing` also routes here via the registry alias map;
            // `usage` remains a separate primary (UsageHandler).
            ctx.cost_snapshot =
                Some(cost::build_cost_snapshot(slash_ctx).await);
        }
        Some("mcp") => {
            // TASK-AGS-811 snapshot population. /mcp is read-only, so
            // there is no paired `apply_effect` branch. No aliases —
            // the shipped stub at registry.rs had none and the spec
            // lists none. The builder awaits
            // `McpServerManager::get_server_info` + N per-server
            // `list_tools_for` calls here so the sync handler
            // consumes pre-computed owned `McpServerEntry` values.
            ctx.mcp_snapshot =
                Some(mcp::build_mcp_snapshot(slash_ctx).await);
        }
        Some("context") => {
            // TASK-AGS-814 snapshot population. /context is read-only,
            // so there is no paired `apply_effect` branch. No aliases —
            // the shipped stub carried `["ctx"]` but the legacy match
            // arm only matched `/context` literally, so the alias was
            // cosmetic (see context_cmd.rs module rustdoc). The
            // builder awaits a single `session_stats.lock()` here so
            // the sync handler consumes pre-captured owned counters.
            ctx.context_snapshot =
                Some(context_cmd::build_context_snapshot(slash_ctx).await);
        }
        Some("denials") => {
            // TASK-AGS-POST-6-BODIES-B08-DENIALS snapshot population.
            // /denials is read-only, so there is no paired
            // `apply_effect` branch. No aliases — the shipped stub at
            // registry.rs:786 used the two-arg declare_handler! form
            // (no aliases slice) and spec lists none. The builder
            // awaits a single `denial_log.lock()` + calls
            // `DenialLog::format_display(20)` here so the sync handler
            // consumes a pre-computed owned `String`.
            ctx.denial_snapshot =
                Some(denials::build_denial_snapshot(slash_ctx).await);
        }
        Some("effort") => {
            // TASK-AGS-POST-6-BODIES-B11-EFFORT snapshot population.
            // /effort has both READ and WRITE sides; the READ side
            // consumes `ctx.effort_snapshot`, the WRITE side goes
            // through the new `CommandEffect::SetEffortLevelShared`
            // + `pending_effort_set` sidecar. No aliases (shipped
            // stub had none and spec lists none). The builder awaits
            // a single `effort_level_shared.lock()` here so the sync
            // handler consumes a pre-captured owned `EffortLevel`.
            // Mirrors AGS-808 /model snapshot gating.
            ctx.effort_snapshot =
                Some(effort::build_effort_snapshot(slash_ctx).await);
        }
        Some("permissions") => {
            // TASK-AGS-POST-6-BODIES-B12-PERMISSIONS snapshot population.
            // /permissions has both READ and WRITE sides; the READ side
            // consumes `ctx.permissions_snapshot.current_mode`, the
            // bypass-allow guard consumes
            // `ctx.permissions_snapshot.allow_bypass_permissions`, and
            // the WRITE side goes through the new
            // `CommandEffect::SetPermissionMode(String)` variant (no
            // sidecar — /permissions has no session-local stack state).
            // No aliases (shipped stub at registry.rs:914 used the
            // two-arg declare_handler! form; spec lists none). The
            // builder awaits a single `permission_mode.lock()` here AND
            // copies the sync `allow_bypass_permissions: bool` so the
            // sync handler consumes a pre-captured snapshot without
            // locking. Mirrors AGS-808 /model and B11 /effort snapshot
            // gating.
            ctx.permissions_snapshot =
                Some(permissions::build_permissions_snapshot(slash_ctx).await);
        }
        Some("copy") => {
            // TASK-AGS-POST-6-BODIES-B14-COPY snapshot population.
            // /copy is read-only (the write side is out-of-process —
            // spawning xclip/clip.exe/pbcopy — and is performed
            // synchronously by the handler via the internal
            // `ClipboardRunner` trait, NOT via a CommandEffect).
            // No aliases (shipped stub at registry.rs:1014 used the
            // two-arg declare_handler! form; spec lists none). The
            // builder awaits a single `last_assistant_response.lock()`
            // here and clones the content into an owned String so the
            // sync handler holds no lock during subprocess spawn.
            ctx.copy_snapshot =
                Some(copy::build_copy_snapshot(slash_ctx).await);
        }
        Some("doctor") => {
            // TASK-AGS-POST-6-BODIES-B15-DOCTOR snapshot population.
            // /doctor is read-only (pure diagnostic display). No aliases
            // (shipped stub at registry.rs:1095 used the two-arg
            // declare_handler! form; spec lists none). The builder
            // awaits `build_doctor_text(slash_ctx)` here (which in turn
            // awaits `mcp_manager.get_server_states().await` and
            // `model_override_shared.lock().await`) and stores the
            // composed String on the snapshot so the sync handler emits
            // via `try_send` with no locks held. Mirrors AGS-807 status
            // / AGS-808 model / B08 denials / B11 effort / B12
            // permissions / B14 copy snapshot gating.
            ctx.doctor_snapshot =
                Some(doctor::build_doctor_snapshot(slash_ctx).await);
        }
        Some("usage") => {
            // TASK-AGS-POST-6-BODIES-B16-USAGE snapshot population.
            // /usage is read-only (shipped slash.rs:315-336 emits a
            // single TextDelta with aggregate session counters, costs,
            // and the cache-stats line — no mutation). No aliases
            // (shipped stub at registry.rs:1166 used the two-arg
            // declare_handler! form; spec lists none). /usage is
            // distinct from /cost (AGS-809): same underlying
            // `session_stats` source but different format — /usage uses
            // `.4` precision + aligned labels + a Turns line, /cost uses
            // `.2` precision + Warn/Hard threshold lines. The builder
            // awaits a single `session_stats.lock()` here so the sync
            // handler consumes pre-captured owned counters + a pre-
            // computed cache_stats_line. Mirrors AGS-807 status /
            // AGS-809 cost / B08 denials / B11 effort / B12 permissions
            // / B14 copy / B15 doctor SNAPSHOT gating.
            ctx.usage_snapshot =
                Some(usage::build_usage_snapshot(slash_ctx).await);
        }
        _ => {}
    }

    ctx
}

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
pub(crate) async fn apply_effect(
    effect: CommandEffect,
    slash_ctx: &SlashCommandContext,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
) {
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
            *slash_ctx.permission_mode.lock().await = resolved.clone();
            let _ = tui_tx
                .send(TuiEvent::PermissionModeChanged(resolved.clone()))
                .await;
            tracing::info!(mode = %resolved, "set permission mode via /permissions");
        }
        // Future variants (AGS-819 /theme, etc.): add a match arm here
        // with the appropriate awaited mutex write. No fallback arm —
        // enum exhaustiveness forces new tickets to wire their effects
        // through this single point of truth.
    }
}

/// Resolve a slash `input` to its primary command name, consulting the
/// [`Registry`]'s alias map. Returns `None` for non-slash inputs, parse
/// failures, or unknown command names.
///
/// Private to this module: builder tests exercise it directly against
/// `default_registry()` because stubbing the full `SlashCommandContext`
/// is out of scope for this ticket (see AGS-807 executor report).
pub(crate) fn resolve_primary_from_input(
    input: &str,
    registry: &Registry,
) -> Option<String> {
    // Reuse the shared tokenizer so we inherit its leading-`/` handling,
    // quoted-arg rules, and flag tolerance.
    let parsed = crate::command::parser::CommandParser::parse(input).ok()?;

    // `Registry::get` is alias-aware; we re-derive the PRIMARY name by
    // asking the commands map first, then the alias map, so the caller
    // can compare against a canonical string like "status".
    if registry.get(&parsed.name).is_some() {
        // Direct primary hit: return the parsed name itself. But if the
        // name is an alias (not a primary), we need the primary string.
        if registry.is_primary(&parsed.name) {
            return Some(parsed.name);
        }
        // Alias → primary resolution.
        return registry.primary_for_alias(&parsed.name).map(str::to_string);
    }
    None
}

// ---------------------------------------------------------------------------
// TASK-AGS-807: tests for build_command_context primary-name resolution
// ---------------------------------------------------------------------------
//
// Rationale for not using a full `SlashCommandContext` fixture:
//
// `SlashCommandContext` carries 24+ fields including `McpServerManager`,
// `Arc<dyn MemoryTrait>`, `Arc<RwLock<AgentRegistry>>`, `SkillRegistry`,
// and several `Mutex`-wrapped runtime state slots. Standing up a real
// fixture would (a) drag test-only dependencies into the bin crate,
// and (b) couple AGS-807's test surface to fields that have nothing to
// do with /status. The AGS-807 executor-report directive explicitly
// permits reporting the chosen approach.
//
// The builder's interesting behaviour is the alias-aware primary-name
// resolution: "/status" → Some("status"), "/info" → Some("status"),
// "/tasks" → Some("tasks") (no snapshot populated). All three of those
// behaviours live in `resolve_primary_from_input`, which takes a
// `&Registry` and is fully testable against `default_registry()` with
// no SlashCommandContext fixture at all.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::{default_registry, CommandEffect};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[test]
    fn build_command_context_populates_status_snapshot_for_slash_status() {
        // Primary lookup proves the builder would populate the snapshot.
        // The full `build_command_context` path is exercised indirectly
        // via live smoke (Gate 5); here we pin the routing decision.
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/status", &reg).as_deref(),
            Some("status"),
            "/status must resolve to primary 'status' so build_command_context \
             populates a StatusSnapshot"
        );
    }

    #[test]
    fn build_command_context_populates_status_snapshot_for_slash_info_alias() {
        // The alias `info` must route to primary `status`. If this ever
        // regresses, /info would fall through to None and StatusHandler
        // would return Err at execute time (see status.rs handler test
        // `status_handler_execute_without_snapshot_returns_err`).
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/info", &reg).as_deref(),
            Some("status"),
            "alias '/info' must route through the registry alias map back to \
             primary 'status' so build_command_context fires the snapshot branch"
        );
    }

    #[test]
    fn build_command_context_leaves_snapshot_none_for_slash_tasks() {
        // `/tasks` is its own primary. The builder should see a primary
        // name != "status" and leave `status_snapshot` at None so the
        // TasksHandler does not pay for unused lock traffic.
        let reg = default_registry();
        let primary = resolve_primary_from_input("/tasks", &reg);
        assert_eq!(
            primary.as_deref(),
            Some("tasks"),
            "/tasks must resolve to its own primary, not 'status'"
        );
        assert_ne!(
            primary.as_deref(),
            Some("status"),
            "the snapshot branch must only fire for 'status' — other primaries \
             must observe status_snapshot = None"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-808: model snapshot routing + apply_effect mutex write.
    //
    // The builder routes `/model` (and its aliases `/m`, `/switch-model`)
    // to `model::build_model_snapshot`. Same rationale as the AGS-807
    // /status tests — we pin the routing decision via the pure
    // `resolve_primary_from_input` helper because standing up a full
    // `SlashCommandContext` fixture drags McpServerManager / MemoryTrait
    // / SkillRegistry into the test crate.
    // -----------------------------------------------------------------

    #[test]
    fn build_command_context_populates_model_snapshot_for_slash_model() {
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/model", &reg).as_deref(),
            Some("model"),
            "/model must resolve to primary 'model' so build_command_context \
             populates a ModelSnapshot"
        );
    }

    /// Verifies the apply_effect semantics for
    /// `CommandEffect::SetModelOverride`. Fixture choice: option (b)
    /// in the AGS-808 executor report — a narrow
    /// `Arc<Mutex<String>>` test harness that mirrors the apply_effect
    /// match body for the one variant under test. Full
    /// SlashCommandContext fixture is infeasible (24+ fields including
    /// McpServerManager + Arc<dyn MemoryTrait>). The production
    /// apply_effect keeps the `&SlashCommandContext` signature for
    /// future-variant symmetry; this test exercises the write-back
    /// invariant (`*mutex.lock().await = resolved`) directly.
    #[tokio::test]
    async fn apply_effect_set_model_override_writes_to_mutex() {
        let model_override_shared: Arc<Mutex<String>> =
            Arc::new(Mutex::new(String::new()));
        assert!(
            model_override_shared.lock().await.is_empty(),
            "pre-condition: override must start empty"
        );

        let effect = CommandEffect::SetModelOverride("claude-sonnet-4-6".to_string());

        // Narrow harness mirroring apply_effect's match arm. If
        // production apply_effect diverges, this test will need to be
        // updated in lockstep — that is the intended coupling.
        match effect {
            CommandEffect::SetModelOverride(resolved) => {
                *model_override_shared.lock().await = resolved;
            }
            // TASK-AGS-POST-6-BODIES-B04-DIFF: RunGitDiffStat belongs
            // to /diff. This narrow harness only constructs
            // SetModelOverride above; RunGitDiffStat is unreachable
            // here. Arm exists to keep the match exhaustive and guard
            // against silent drift on future variants.
            CommandEffect::RunGitDiffStat(_) => {
                unreachable!("narrow apply_effect harness only exercises SetModelOverride")
            }
            // TASK-AGS-POST-6-BODIES-B10-ADDDIR: AddExtraDir belongs to
            // /add-dir. This narrow harness only constructs
            // SetModelOverride above; AddExtraDir is unreachable here.
            // Arm exists to keep the match exhaustive and guard against
            // silent drift on future variants.
            CommandEffect::AddExtraDir(_) => {
                unreachable!("narrow apply_effect harness only exercises SetModelOverride")
            }
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: SetEffortLevelShared
            // belongs to /effort. This narrow harness only constructs
            // SetModelOverride above; SetEffortLevelShared is
            // unreachable here. Arm exists to keep the match exhaustive
            // and guard against silent drift on future variants.
            CommandEffect::SetEffortLevelShared(_) => {
                unreachable!("narrow apply_effect harness only exercises SetModelOverride")
            }
            // TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: SetPermissionMode
            // belongs to /permissions. This narrow harness only
            // constructs SetModelOverride above; SetPermissionMode is
            // unreachable here. Arm exists to keep the match exhaustive
            // and guard against silent drift on future variants.
            CommandEffect::SetPermissionMode(_) => {
                unreachable!("narrow apply_effect harness only exercises SetModelOverride")
            }
        }

        let got = model_override_shared.lock().await.clone();
        assert_eq!(
            got, "claude-sonnet-4-6",
            "apply_effect must overwrite model_override_shared with the \
             resolved full model id"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-809: /cost snapshot routing. Same rationale as AGS-807 +
    // AGS-808 — we pin the routing decision via
    // `resolve_primary_from_input` because standing up a full
    // `SlashCommandContext` fixture drags McpServerManager /
    // MemoryTrait / SkillRegistry into the test crate. The primary
    // name returned here is what `build_command_context` uses to
    // decide whether to populate `ctx.cost_snapshot`.
    //
    // /cost is READ-ONLY, so there is no matching `apply_effect` test
    // in this ticket — no CommandEffect variant was added for AGS-809.
    // -----------------------------------------------------------------

    #[test]
    fn build_command_context_populates_cost_snapshot_for_slash_cost() {
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/cost", &reg).as_deref(),
            Some("cost"),
            "/cost must resolve to primary 'cost' so build_command_context \
             populates a CostSnapshot"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-811: /mcp snapshot routing. Same rationale as AGS-807 /
    // AGS-808 / AGS-809 — we pin the routing decision via
    // `resolve_primary_from_input` because standing up a full
    // `SlashCommandContext` fixture drags McpServerManager /
    // MemoryTrait / SkillRegistry into the test crate. The primary
    // name returned here is what `build_command_context` uses to
    // decide whether to populate `ctx.mcp_snapshot`.
    //
    // /mcp is READ-ONLY, so there is no matching `apply_effect` test
    // in this ticket — no CommandEffect variant was added for AGS-811.
    // Also no aliases — the shipped stub had none and the AGS-811
    // spec lists none.
    // -----------------------------------------------------------------

    #[test]
    fn build_command_context_populates_mcp_snapshot_for_slash_mcp() {
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/mcp", &reg).as_deref(),
            Some("mcp"),
            "/mcp must resolve to primary 'mcp' so build_command_context \
             populates an McpSnapshot"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-814: /context snapshot routing. Same rationale as
    // AGS-807/808/809/811 — we pin the routing decision via
    // `resolve_primary_from_input` because standing up a full
    // `SlashCommandContext` fixture drags McpServerManager /
    // MemoryTrait / SkillRegistry into the test crate. The primary
    // name returned here is what `build_command_context` uses to
    // decide whether to populate `ctx.context_snapshot`.
    //
    // /context is READ-ONLY, so there is no matching `apply_effect`
    // test in this ticket — no CommandEffect variant was added for
    // AGS-814. No aliases either — shipped stub's `ctx` alias was
    // cosmetic (legacy match arm only matched `/context` literally).
    // -----------------------------------------------------------------

    #[test]
    fn build_command_context_populates_context_snapshot_for_slash_context() {
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/context", &reg).as_deref(),
            Some("context"),
            "/context must resolve to primary 'context' so \
             build_command_context populates a ContextSnapshot"
        );
    }

    #[test]
    fn build_command_context_populates_cost_snapshot_for_slash_billing_alias() {
        // Spec wanted `/usage` as an alias for /cost, but `usage` is
        // already a shipped primary (UsageHandler). Only `/billing`
        // routes to /cost; `/usage` remains bound to UsageHandler.
        // See cost.rs module rustdoc + the CONFIRM R-item in the
        // AGS-809 executor report.
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/billing", &reg).as_deref(),
            Some("cost"),
            "alias '/billing' must route through the registry alias map \
             back to primary 'cost' so build_command_context fires the \
             cost snapshot branch"
        );
        // Sanity: /usage must NOT route to 'cost'. It is a primary in
        // its own right and its snapshot branch (if any) belongs to a
        // future UsageHandler body-migrate, not AGS-809.
        assert_eq!(
            resolve_primary_from_input("/usage", &reg).as_deref(),
            Some("usage"),
            "/usage is a shipped primary (UsageHandler); must NOT resolve \
             to 'cost'"
        );
    }
}
