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

use crate::command::registry::CommandContext;
use crate::command::{
    context_cmd, copy, cost, denials, doctor, effort, mcp, model, permissions, status, usage,
};
use crate::slash_context::SlashCommandContext;

use super::resolve_primary_from_input;

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
pub(crate) fn build_command_context<'a>(
    input: &'a str,
    tui_tx: archon_tui::event_channel::TuiEventSender,
    slash_ctx: &'a SlashCommandContext,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = CommandContext> + Send + 'a>> {
    Box::pin(async move {
        let mut ctx = CommandContext {
            tui_tx,
            pending_tui_events: std::sync::Mutex::new(Vec::new()),
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
            session_store: Some(Arc::clone(&slash_ctx.session_store)),
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
            // GHOST-006: sandbox flag, same DIRECT pattern as fast_mode_shared.
            // Toggled by /sandbox on/off; read by dispatch paths via SandboxBackend.
            sandbox_flag: Some(Arc::clone(&slash_ctx.sandbox_flag)),
            // GHOST-004: hook registry for /hooks enable/disable/reload.
            // Populated UNCONDITIONALLY (DIRECT pattern). The handler calls
            // set_enabled / reload through this Arc.
            hook_registry: slash_ctx.hook_registry.as_ref().map(Arc::clone),
            plugin_enable_state: Some(Arc::clone(&slash_ctx.plugin_enable_state)),
            // GHOST-007: cancel handle + dispatcher for /cancel real cancellation.
            cancel_handle: Some(Arc::clone(&slash_ctx.cancel_handle)),
            agent_dispatcher: Some(Arc::clone(&slash_ctx.agent_dispatcher)),
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
            // TASK-#211 SLASH-AGENT: clone the Arc<RwLock<AgentRegistry>>
            // unconditionally — handler reads it via the synchronous
            // `RwLock::read()`. Cheap (~8 bytes + atomic refcount).
            agent_registry: Some(Arc::clone(&slash_ctx.agent_registry)),
            // TASK-DS-001: DIRECT-pattern clone of TaskService.
            // Populated UNCONDITIONALLY (not gated on primary name).
            // `Arc<dyn TaskService>` is cheap to clone (~8 bytes +
            // atomic refcount); handlers call `ctx.task_service.submit()`
            // to spawn agents without blocking the TUI input loop.
            task_service: Some(Arc::clone(&slash_ctx.task_service)),
            // DIRECT-pattern clones for pipeline TUI commands.
            coding_pipeline: Some(Arc::clone(&slash_ctx.coding_pipeline)),
            research_pipeline: Some(Arc::clone(&slash_ctx.research_pipeline)),
            llm_adapter: Some(Arc::clone(&slash_ctx.llm_adapter)),
            // DIRECT-pattern clone for LEANN pipeline integration.
            leann: slash_ctx.leann.as_ref().map(Arc::clone),
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: SIDECAR slot for the
            // session-local `&mut EffortState` write. Initialised to
            // `None` here; populated (in lockstep with `pending_effect`)
            // by `EffortHandler::execute` on the WRITE branch; drained at
            // the slash.rs dispatch site AFTER `apply_effect` returns.
            pending_effort_set: None,
            // TASK-AGS-POST-6-EXPORT-MIGRATE: SIDECAR-SLOT shared slot
            // cloned from `SlashCommandContext::pending_export_shared`.
            // Populated UNCONDITIONALLY (not gated on the primary name) —
            // `Arc::clone` is cheap (~8 bytes + atomic refcount) and the
            // field is read only by `ExportHandler::execute`; other
            // handlers see the Arc but never acquire the inner Mutex.
            // The drain runs in session.rs, NOT in slash.rs / apply_effect,
            // because it needs `agent.lock().await` on a mutex that only
            // session.rs has in scope. See src/command/export.rs module
            // rustdoc for the full SIDECAR-SLOT rationale.
            pending_export: Some(Arc::clone(&slash_ctx.pending_export_shared)),
            cozo_db: slash_ctx.cozo_db.clone(),
            governed_learning_db: slash_ctx.governed_learning_db.clone(),
            // Reference: archon-pipeline/src/learning/gnn/auto_trainer.rs.
            // /learning-status reads .status() from this Arc to display
            // live loop state (training_count, memories_since_last_train, etc.)
            auto_trainer: slash_ctx.auto_trainer.clone(),
        };

        // Resolve the primary command name (alias-aware) so "/info" routes
        // to the same snapshot path as "/status" and "/m" / "/switch-model"
        // route to the same snapshot path as "/model". The registry holds
        // the canonical alias→primary mapping; we delegate rather than
        // duplicating the table here.
        let primary = resolve_primary_from_input(input, slash_ctx.registry.as_ref());

        match primary.as_deref() {
            Some("status") => {
                ctx.status_snapshot = Some(status::build_status_snapshot(slash_ctx).await);
            }
            Some("model") => {
                ctx.model_snapshot = Some(model::build_model_snapshot(slash_ctx).await);
            }
            Some("cost") => {
                // TASK-AGS-809 snapshot population. /cost is read-only,
                // so there is no paired `apply_effect` branch. The alias
                // `billing` also routes here via the registry alias map;
                // `usage` remains a separate primary (UsageHandler).
                ctx.cost_snapshot = Some(cost::build_cost_snapshot(slash_ctx).await);
            }
            Some("mcp") | Some("connect") => {
                // TASK-AGS-811 snapshot population. /mcp is read-only, so
                // TASK-#214 SLASH-CONNECT widens the arm: /connect (no
                // args) renders the same server list, so it consumes the
                // same async-built snapshot. The /connect WRITE path
                // (with name arg) doesn't strictly need the snapshot,
                // but populating it unconditionally keeps the gate
                // simple and preserves the no-args list view.
                // there is no paired `apply_effect` branch. No aliases —
                // the shipped stub at registry.rs had none and the spec
                // lists none. The builder awaits
                // `McpServerManager::get_server_info` + N per-server
                // `list_tools_for` calls here so the sync handler
                // consumes pre-computed owned `McpServerEntry` values.
                ctx.mcp_snapshot = Some(mcp::build_mcp_snapshot(slash_ctx).await);
            }
            Some("context") => {
                // TASK-AGS-814 snapshot population. /context is read-only,
                // so there is no paired `apply_effect` branch. No aliases —
                // the shipped stub carried `["ctx"]` but the legacy match
                // arm only matched `/context` literally, so the alias was
                // cosmetic (see context_cmd.rs module rustdoc). The
                // builder awaits a single `session_stats.lock()` here so
                // the sync handler consumes pre-captured owned counters.
                ctx.context_snapshot = Some(context_cmd::build_context_snapshot(slash_ctx).await);
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
                ctx.denial_snapshot = Some(denials::build_denial_snapshot(slash_ctx).await);
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
                ctx.effort_snapshot = Some(effort::build_effort_snapshot(slash_ctx).await);
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
                ctx.copy_snapshot = Some(copy::build_copy_snapshot(slash_ctx).await);
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
                ctx.doctor_snapshot = Some(doctor::build_doctor_snapshot(slash_ctx).await);
            }
            Some("usage") | Some("extra-usage") | Some("summary") => {
                // TASK-AGS-POST-6-BODIES-B16-USAGE snapshot population.
                // TASK-#215 SLASH-EXTRA-USAGE widens the arm to also fire
                // on primary == "extra-usage" — both handlers consume the
                // same `UsageSnapshot`; /extra-usage just renders it as
                // 6 grouped sections + per-turn / cost-per-1k metrics.
                // TASK-#209 SLASH-SUMMARY widens once more — /summary
                // emits a one-glance headline using the same snapshot
                // (turns + total tokens + total cost + first cache line).
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
                ctx.usage_snapshot = Some(usage::build_usage_snapshot(slash_ctx).await);
            }
            _ => {}
        }

        ctx
    })
}
