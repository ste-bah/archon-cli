use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use archon_tui::app::TuiEvent;

use super::effect::CommandEffect;
use crate::command::context_cmd::ContextSnapshot;
use crate::command::cost::CostSnapshot;
use crate::command::denials::DenialSnapshot;
use crate::command::mcp::McpSnapshot;
use crate::command::usage::UsageSnapshot;

pub struct CommandContext {
    /// TASK-DS-001: Async TaskService wired into CommandContext per
    /// 02-technical-spec line 1226 and REQ-ASYNC-001. Constructed at
    /// session bootstrap via `DefaultTaskService::new(agent_registry,
    /// 10000)`. Cloned into per-dispatch context from
    /// `SlashCommandContext` via the DIRECT pattern (unconditionally,
    /// no per-command gate). Handlers call `ctx.task_service.submit()`
    /// to spawn agents without blocking the TUI input loop.
    pub(crate) task_service: Option<Arc<dyn archon_core::tasks::TaskService>>,
    /// Coding pipeline facade — cloned from SlashCommandContext via
    /// DIRECT pattern. Populated unconditionally.
    pub(crate) coding_pipeline: Option<Arc<archon_pipeline::coding::facade::CodingFacade>>,
    /// Research pipeline facade — cloned from SlashCommandContext via
    /// DIRECT pattern. Populated unconditionally.
    pub(crate) research_pipeline: Option<Arc<archon_pipeline::research::facade::ResearchFacade>>,
    /// Shared LLM client for pipeline execution — cloned from
    /// SlashCommandContext via DIRECT pattern. Populated unconditionally.
    pub(crate) llm_adapter: Option<Arc<dyn archon_pipeline::runner::LlmClient>>,
    /// LEANN integration — cloned from SlashCommandContext via DIRECT
    /// pattern. Populated unconditionally.
    pub(crate) leann: Option<Arc<archon_pipeline::runner::LeannIntegration>>,
    /// TUI event sink for text deltas, errors, and state change
    /// notifications.
    ///
    /// Bounded sender with a synchronous `send(event)` API. This keeps sync
    /// slash handlers simple while preventing unbounded queue growth when the
    /// render loop falls behind.
    pub(crate) tui_tx: archon_tui::event_channel::TuiEventSender,
    /// TASK-AGS-807 snapshot-pattern field.
    ///
    /// Populated by `build_command_context` for `/status` (and its
    /// alias `/info`) ONLY. Every other command observes `None` and
    /// pays zero additional lock traffic. The sync
    /// [`CommandHandler::execute`] cannot await, so the dispatch site
    /// acquires the four `/status` async locks in advance and passes
    /// the owned values via this field.
    pub(crate) status_snapshot: Option<crate::command::status::StatusSnapshot>,
    /// TASK-AGS-808 snapshot-pattern field (READ side of /model).
    ///
    /// Populated by `build_command_context` for `/model` (and its
    /// aliases `/m`, `/switch-model`) ONLY. Every other command
    /// observes `None` and pays zero additional lock traffic. Mirrors
    /// the AGS-807 `status_snapshot` convention (separate typed struct
    /// per ticket; fields differ across handlers so snapshots are not
    /// cross-reused).
    pub(crate) model_snapshot: Option<crate::command::model::ModelSnapshot>,
    /// TASK-AGS-809 snapshot-pattern field (READ-only /cost).
    ///
    /// Populated by `build_command_context` for `/cost` (and its
    /// aliases `/usage`, `/billing`) ONLY. Every other command observes
    /// `None` and pays zero additional lock traffic. Per the AGS-822
    /// Rule 5 extension pattern: each body-migrate ticket appends one
    /// typed snapshot field — /cost is READ-only so there is NO
    /// matching `CommandEffect` variant.
    pub(crate) cost_snapshot: Option<CostSnapshot>,
    /// TASK-AGS-811 snapshot-pattern field (READ-only /mcp).
    ///
    /// Populated by `build_command_context` for `/mcp` ONLY (no
    /// aliases). Every other command observes `None` and pays zero
    /// additional lock traffic on `McpServerManager`. Per the AGS-822
    /// Rule 5 extension pattern: each body-migrate ticket appends one
    /// typed snapshot field — /mcp is READ-only so there is NO
    /// matching `CommandEffect` variant. Subcommands `connect` /
    /// `disconnect` / `reload` listed in the AGS-811 spec are
    /// SCOPE-HELD (shipped wins drift-reconcile); the first ticket
    /// that actually needs them adds the write-side field at that
    /// point.
    pub(crate) mcp_snapshot: Option<McpSnapshot>,
    /// TASK-AGS-814 snapshot-pattern field (READ-only /context).
    ///
    /// Populated by `build_command_context` for `/context` ONLY (no
    /// aliases — the shipped stub's `ctx` alias was cosmetic; see
    /// `context_cmd.rs` module rustdoc). Every other command observes
    /// `None` and pays zero additional lock traffic on `session_stats`.
    /// Per the AGS-822 Rule 5 extension pattern: each body-migrate
    /// ticket appends one typed snapshot field — /context is READ-only
    /// so there is NO matching `CommandEffect` variant.
    pub(crate) context_snapshot: Option<ContextSnapshot>,
    /// TASK-AGS-815 DIRECT-pattern field (/fork).
    ///
    /// Clone of `SlashCommandContext::session_id` populated
    /// UNCONDITIONALLY by `build_command_context` (not per-command —
    /// session_id is always meaningful and cheap to clone). `/fork`
    /// reads it to pass `source_id` to
    /// `archon_session::fork::fork_session`. `Option<String>` so the
    /// dispatcher/handler test fixtures can construct a
    /// `CommandContext` without standing up a full
    /// `SlashCommandContext` — in those tests the field observes
    /// `None` and the handler returns an Err-with-message describing
    /// the missing-session-id condition rather than panicking.
    /// No matching `CommandEffect` variant — `/fork` is a pure
    /// DIRECT-pattern sync body (no async mutex writes back to
    /// shared state).
    pub(crate) session_id: Option<String>,
    /// Shared session store for session-management slash commands.
    ///
    /// Populated unconditionally from `SlashCommandContext` so `/resume`,
    /// `/fork`, `/rename`, and `/rewind` use the same configured database
    /// as the running session instead of reopening the default path.
    pub(crate) session_store: Option<Arc<archon_session::storage::SessionStore>>,
    /// TASK-AGS-817 DIRECT-pattern field (/memory).
    ///
    /// Shared memory handle for `/memory` (DIRECT pattern). `Arc` clone
    /// per dispatch is cheap (~8 bytes + atomic refcount increment).
    /// Populated UNCONDITIONALLY in context.rs outer builder literal
    /// (mirrors the AGS-815 `session_id` cross-cutting precedent, not
    /// gated on the primary name). `None` sentinel reserved for test
    /// fixtures that construct `CommandContext` directly without
    /// standing up a full `SlashCommandContext`; in those tests the
    /// handler observes `None` and returns an Err-with-message
    /// describing the missing-memory condition rather than panicking.
    /// `archon_memory::MemoryTrait` is fully sync (all 12 trait methods
    /// are plain `fn`) so no matching `CommandEffect` variant is
    /// required — `/memory clear` performs the `clear_all()` mutation
    /// via a direct sync call inside `execute`, not an async write-back.
    pub(crate) memory: Option<Arc<dyn archon_memory::MemoryTrait>>,
    /// TASK-AGS-POST-6-BODIES-B13-GARDEN DIRECT-pattern field (/garden).
    ///
    /// Clone of `SlashCommandContext::garden_config` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id` and AGS-817 `memory` cross-cutting precedent — not
    /// gated on the primary name). `/garden` (default branch) reads it
    /// to pass `&GardenConfig` into the sync
    /// `archon_memory::garden::consolidate(&dyn MemoryTrait,
    /// &GardenConfig)` entry point; `/garden stats` does not read it.
    /// `GardenConfig` derives `Clone` so cloning it per dispatch is
    /// cheap (small fixed-size struct of numeric thresholds — no Arc,
    /// no heap allocation beyond the struct itself). `None` sentinel
    /// reserved for test fixtures that construct `CommandContext`
    /// directly without standing up a full `SlashCommandContext`; in
    /// those tests the consolidate branch observes `None` and returns
    /// an Err-with-message describing the missing-config condition
    /// rather than panicking. Both `format_garden_stats` and
    /// `consolidate` are fully sync (all 12 MemoryTrait methods are
    /// plain `fn`) so no matching `CommandEffect` variant is required.
    pub(crate) garden_config: Option<archon_memory::garden::GardenConfig>,
    /// TASK-AGS-POST-6-BODIES-B01-FAST DIRECT-pattern field (/fast).
    ///
    /// Clone of `SlashCommandContext::fast_mode_shared` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id` and AGS-817 `memory` cross-cutting precedent — not
    /// gated on the primary name). `/fast` reads and atomically mutates
    /// it. `Option<Arc<AtomicBool>>` so the handler test fixtures can
    /// construct a `CommandContext` without standing up a full
    /// `SlashCommandContext`; when `None` the handler returns an
    /// Err-with-message describing the missing-shared-state condition
    /// rather than panicking. No matching `CommandEffect` variant — the
    /// mutation is a sync atomic store.
    pub(crate) fast_mode_shared: Option<Arc<AtomicBool>>,
    /// TASK-AGS-POST-6-BODIES-B02-THINKING DIRECT-pattern field
    /// (/thinking).
    ///
    /// Clone of `SlashCommandContext::show_thinking` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, and B01-FAST `fast_mode_shared`
    /// cross-cutting precedent — not gated on the primary name).
    /// `/thinking` reads it (to log a no-op?) and atomically stores
    /// the new state from the parsed `on`/`off`/empty subcommand.
    /// `Option<Arc<AtomicBool>>` so the handler test fixtures can
    /// construct a `CommandContext` without standing up a full
    /// `SlashCommandContext`; when `None` the handler returns an
    /// Err-with-message describing the missing-shared-state condition
    /// rather than panicking. No matching `CommandEffect` variant — the
    /// mutation is a sync atomic store.
    pub(crate) show_thinking: Option<Arc<AtomicBool>>,
    /// TASK-AGS-POST-6-BODIES-B04-DIFF DIRECT-with-effect-pattern field
    /// (/diff).
    ///
    /// Clone of `SlashCommandContext::working_dir` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`, and
    /// B02-THINKING `show_thinking` cross-cutting precedent — not gated
    /// on the primary name). `/diff` reads it to produce the
    /// `CommandEffect::RunGitDiffStat(PathBuf)` effect; apply_effect
    /// passes the cloned path to `crate::command::slash::handle_diff_command`
    /// which spawns `git diff --stat` via `tokio::process::Command`.
    /// `Option<PathBuf>` so the handler test fixtures can construct a
    /// `CommandContext` without standing up a full `SlashCommandContext`;
    /// when `None` the handler emits a `TuiEvent::Error` describing the
    /// missing-shared-state condition (mirroring B01-FAST's
    /// fast_mode_shared=None Err-with-message pattern, adapted for an
    /// event emission path since /diff must stay Ok(()) to keep the
    /// dispatcher contract uniform).
    pub(crate) working_dir: Option<PathBuf>,
    /// TASK-AGS-POST-6-BODIES-B06-HELP DIRECT-pattern field (/help).
    ///
    /// Clone of `SlashCommandContext::skill_registry` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`,
    /// B02-THINKING `show_thinking`, and B04-DIFF `working_dir` cross-
    /// cutting precedent — not gated on the primary name). `/help` reads
    /// it to call the sync `SkillRegistry::format_help()` /
    /// `format_skill_help()` methods for the extended-commands suffix
    /// and per-skill detail output.
    ///
    /// `Option<Arc<SkillRegistry>>` so the handler test fixtures can
    /// construct a `CommandContext` without standing up a full
    /// `SlashCommandContext`; when `None` the handler still emits the
    /// static core-commands header (it does not depend on the registry)
    /// but skips the skill-registry-sourced suffix, and the single-
    /// command arg path falls through to the unknown-command Error
    /// branch. Production always populates
    /// `Some(Arc::clone(&slash_ctx.skill_registry))`.
    ///
    /// No matching `CommandEffect` variant — `/help` is a pure DIRECT-
    /// pattern read (no async mutex writes back to shared state).
    pub(crate) skill_registry: Option<Arc<archon_core::skills::SkillRegistry>>,
    /// TASK-AGS-POST-6-BODIES-B08-DENIALS SNAPSHOT-pattern field
    /// (READ-only /denials).
    ///
    /// Populated by `build_command_context` for `/denials` ONLY (no
    /// aliases — the shipped stub at registry.rs:786 used the two-arg
    /// declare_handler! form). Every other command observes `None`
    /// and pays zero additional lock traffic on
    /// `SlashCommandContext::denial_log`. Per the AGS-822 Rule 5
    /// extension pattern: each body-migrate ticket that needs an
    /// async-locked snapshot appends one typed snapshot field —
    /// /denials is READ-only so there is NO matching `CommandEffect`
    /// variant (mirrors AGS-811 /mcp and AGS-814 /context).
    pub(crate) denial_snapshot: Option<DenialSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B11-EFFORT SNAPSHOT-pattern field (READ
    /// side of /effort).
    ///
    /// Populated by `build_command_context` for `/effort` ONLY (no
    /// aliases — shipped stub at registry.rs:801 used the two-arg
    /// declare_handler! form). Every other command observes `None`
    /// and pays zero additional lock traffic on
    /// `SlashCommandContext::effort_level_shared`. Mirrors AGS-807
    /// `status_snapshot`, AGS-808 `model_snapshot`, and B08
    /// `denial_snapshot` convention. Carries an owned `EffortLevel`
    /// (Copy) so the sync handler reads without any lock.
    pub(crate) effort_snapshot: Option<crate::command::effort::EffortSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B12-PERMISSIONS SNAPSHOT-pattern field
    /// (HYBRID — READ side + bypass-allow guard for /permissions).
    ///
    /// Populated by `build_command_context` for `/permissions` ONLY (no
    /// aliases — shipped stub at registry.rs:914 used the two-arg
    /// declare_handler! form). Every other command observes `None` and
    /// pays zero additional lock traffic on `permission_mode`. Carries
    /// BOTH `current_mode: String` (captured via
    /// `slash_ctx.permission_mode.lock().await`) AND
    /// `allow_bypass_permissions: bool` (copied from the sync field on
    /// `SlashCommandContext`). Bundling both into one snapshot
    /// minimises the extension surface — one snapshot per primary, no
    /// second DIRECT-pattern cross-cutting field. Mirrors AGS-807
    /// `status_snapshot`, AGS-808 `model_snapshot`, B08 `denial_snapshot`,
    /// and B11 `effort_snapshot` snapshot gating rule.
    pub(crate) permissions_snapshot: Option<crate::command::permissions::PermissionsSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B14-COPY SNAPSHOT-pattern field (READ-only
    /// /copy).
    ///
    /// Populated by `build_command_context` for `/copy` ONLY (no
    /// aliases — shipped stub at registry.rs:1014 used the two-arg
    /// declare_handler! form). Every other command observes `None`
    /// and pays zero additional lock traffic on
    /// `SlashCommandContext::last_assistant_response`. Mirrors AGS-807
    /// `status_snapshot`, AGS-808 `model_snapshot`, B08 `denial_snapshot`,
    /// B11 `effort_snapshot`, and B12 `permissions_snapshot` gating rule.
    /// Carries an owned `String` (clone of the current last-assistant-
    /// response) so the sync handler reads without holding a lock across
    /// the potentially-blocking clipboard subprocess spawn.
    pub(crate) copy_snapshot: Option<crate::command::copy::CopySnapshot>,
    /// TASK-AGS-POST-6-BODIES-B15-DOCTOR SNAPSHOT-DELEGATE field
    /// (READ-only /doctor).
    ///
    /// Populated by `build_command_context` for `/doctor` ONLY (no
    /// aliases — shipped stub at registry.rs:1095 used the two-arg
    /// declare_handler! form). Every other command observes `None` and
    /// pays zero additional lock traffic on
    /// `SlashCommandContext::mcp_manager` +
    /// `SlashCommandContext::model_override_shared`. Mirrors AGS-807
    /// `status_snapshot`, AGS-808 `model_snapshot`, B08 `denial_snapshot`,
    /// B11 `effort_snapshot`, B12 `permissions_snapshot`, and B14
    /// `copy_snapshot` gating rule. Carries a single owned `String` —
    /// the fully-composed diagnostic text produced by
    /// `doctor::build_doctor_text` — so the sync handler emits via
    /// `try_send` without holding any lock.
    pub(crate) doctor_snapshot: Option<crate::command::doctor::DoctorSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B16-USAGE SNAPSHOT-pattern field
    /// (READ-only /usage).
    ///
    /// Populated by `build_command_context` for `/usage` ONLY (no
    /// aliases — shipped stub at registry.rs:1166 used the two-arg
    /// declare_handler! form). Every other command observes `None` and
    /// pays zero additional lock traffic on
    /// `SlashCommandContext::session_stats`. Mirrors AGS-807
    /// `status_snapshot`, AGS-809 `cost_snapshot`, B08 `denial_snapshot`,
    /// B11 `effort_snapshot`, B12 `permissions_snapshot`, B14
    /// `copy_snapshot`, and B15 `doctor_snapshot` gating rule. Carries
    /// owned scalar fields (input/output tokens, turn count, pre-
    /// computed costs) plus an owned `cache_stats_line: String` so the
    /// sync handler emits via `try_send` without holding any lock.
    pub(crate) usage_snapshot: Option<UsageSnapshot>,
    /// TASK-AGS-POST-6-BODIES-B20-RELOAD DIRECT-pattern field (/reload).
    ///
    /// Clone of `SlashCommandContext::config_path` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`,
    /// B02-THINKING `show_thinking`, B04-DIFF `working_dir`, B06-HELP
    /// `skill_registry`, and B13-GARDEN `garden_config` cross-cutting
    /// precedent — not gated on the primary name). `/reload` reads it
    /// to pass `&[PathBuf]` into the sync
    /// `archon_core::config_watcher::force_reload(config_paths:
    /// &[PathBuf], current: &ArchonConfig)` entry point via
    /// `std::slice::from_ref(config_path)`. `PathBuf` clone per
    /// dispatch is cheap (one Vec<u8> alloc). `None` sentinel reserved
    /// for test fixtures that construct `CommandContext` directly
    /// without standing up a full `SlashCommandContext`; in those
    /// tests the handler observes `None` and returns an
    /// Err-with-message describing the missing-config_path condition
    /// rather than panicking. `archon_core::config_watcher::force_reload`
    /// is fully sync (no `async fn`, no `.await`) so no matching
    /// `CommandEffect` variant is required — `/reload` performs its
    /// read-and-diff synchronously inside `execute`.
    pub(crate) config_path: Option<std::path::PathBuf>,
    /// TASK-AGS-POST-6-BODIES-B22-LOGIN DIRECT-pattern field (/login).
    ///
    /// Clone of `SlashCommandContext::auth_label` populated
    /// UNCONDITIONALLY by `build_command_context` (mirrors the AGS-815
    /// `session_id`, AGS-817 `memory`, B01-FAST `fast_mode_shared`,
    /// B02-THINKING `show_thinking`, B04-DIFF `working_dir`, B06-HELP
    /// `skill_registry`, B13-GARDEN `garden_config`, and B20-RELOAD
    /// `config_path` cross-cutting precedent — not gated on the
    /// primary name). `/login` reads it to include the active auth
    /// method in the emitted `TuiEvent::TextDelta` message (see
    /// `crate::command::login::LoginHandler::execute`). `String` clone
    /// per dispatch is cheap (one heap alloc). `None` sentinel
    /// reserved for test fixtures that construct `CommandContext`
    /// directly without standing up a full `SlashCommandContext`; in
    /// those tests the handler observes `None` and returns an
    /// Err-with-message describing the missing-auth_label condition
    /// rather than panicking. Matches the AGS-815
    /// `fork_handler_execute_without_session_id_returns_err` and B20
    /// `execute_without_config_path_returns_err` pattern. No matching
    /// `CommandEffect` variant — `/login` is a pure DIRECT-pattern
    /// read (no async mutex writes back to shared state).
    pub(crate) auth_label: Option<String>,
    /// TASK-#211 SLASH-AGENT DIRECT-pattern field (/agent).
    ///
    /// `Arc<RwLock<AgentRegistry>>` cloned from
    /// `SlashCommandContext::agent_registry`, populated UNCONDITIONALLY
    /// (mirrors the AGS-815 `session_id`, AGS-817 `memory`, B06-HELP
    /// `skill_registry` cross-cutting precedent — not gated on the
    /// primary name). `/agent` reads it to render the list / info /
    /// run-hint subcommands; `RwLock::read()` is sync, so the handler
    /// consumes the registry without holding any lock across `ctx.emit`.
    /// `None` sentinel reserved for test fixtures that construct
    /// `CommandContext` directly without standing up a full
    /// `SlashCommandContext`; in those tests `/agent` returns an
    /// Err-with-message describing the missing-registry condition
    /// rather than panicking.
    pub(crate) agent_registry: Option<Arc<std::sync::RwLock<archon_core::agents::AgentRegistry>>>,
    /// TASK-AGS-808 effect-slot field (WRITE side of /model and future
    /// write-tickets).
    ///
    /// The Rule-5 extension pattern: sync `CommandHandler::execute`
    /// cannot await mutex writes on shared state. Instead, a handler
    /// synchronously stashes a [`CommandEffect`] variant here; the
    /// dispatch site in `slash.rs::handle_slash_command` takes the
    /// value (consuming the slot via `.take()`) and awaits the write
    /// in `command::context::apply_effect` AFTER dispatch returns. The
    /// single-shot `Option` guarantees exactly-once application even
    /// if the dispatcher were to re-fire on the same context.
    pub(crate) pending_effect: Option<CommandEffect>,
    /// TASK-AGS-POST-6-BODIES-B11-EFFORT SIDECAR field (LOCAL write
    /// side of /effort).
    ///
    /// The /effort body-migrate is the first handler that needs to
    /// mutate session-local stack state (`&mut EffortState`) in
    /// addition to the shared-mutex state covered by the effect-slot.
    /// `pending_effect` handles the shared-mutex write via
    /// `CommandEffect::SetEffortLevelShared`; this sidecar slot
    /// carries the same `EffortLevel` to the slash.rs dispatch site,
    /// where the caller's `&mut effort_state` parameter is drained
    /// AFTER `apply_effect` returns by calling
    /// `effort_state.set_level(level)`. Single-shot (`.take()` on
    /// drain) just like `pending_effect`.
    ///
    /// Initialised to `None` by `build_command_context`; populated in
    /// lockstep with `pending_effect` by `EffortHandler::execute` so
    /// the two slots are never out of sync.
    pub(crate) pending_effort_set: Option<archon_llm::effort::EffortLevel>,
    /// TASK-AGS-POST-6-EXPORT-MIGRATE SIDECAR-SLOT field (WRITE side of
    /// /export; drain lives in session.rs).
    ///
    /// `Option<Arc<std::sync::Mutex<Option<ExportDescriptor>>>>`.
    /// Populated UNCONDITIONALLY by `build_command_context` (cloned
    /// `Arc` from `SlashCommandContext::pending_export_shared`). The
    /// outer `Option` is `None` only in test fixtures that construct
    /// `CommandContext` directly without a full `SlashCommandContext`;
    /// in that case `ExportHandler::execute` emits a wiring-regression
    /// error instead of stashing the descriptor. Production always
    /// observes `Some(Arc::clone(...))`.
    ///
    /// The handler writes `*slot.lock().unwrap() = Some(desc)` synchronously;
    /// session.rs's drain block (inside the `if handled {` branch of
    /// the input-processor task) calls `.take()` on the same shared
    /// `std::sync::Mutex`, obtains the `Agent` mutex guard via
    /// `agent.lock().await`, reads `conversation_state().messages`,
    /// and runs the file-write I/O. `std::sync::Mutex` (not
    /// `tokio::sync::Mutex`) because the handler is sync and the drain
    /// site holds the lock only across a single `.take()` call —
    /// zero `.await` is held across either lock acquisition.
    ///
    /// Why NOT a plain `Option<ExportDescriptor>` like
    /// `pending_effort_set`: the drain must run in session.rs where
    /// `__cmd_ctx` (the dispatch-local `CommandContext`) does not
    /// exist. A shared `Arc<Mutex<...>>` is the one mechanism that
    /// lets the sync handler write and session.rs drain without
    /// forcing an edit to `slash.rs` (which is a hard scope guard for
    /// this ticket).
    pub(crate) pending_export:
        Option<Arc<std::sync::Mutex<Option<crate::command::export::ExportDescriptor>>>>,
    /// CozoDB instance for learning subsystem persistence (GNN weights,
    /// trajectories, Adam state, training runs). Cloned from
    /// `SlashCommandContext::cozo_db` at dispatch time via DIRECT pattern.
    pub(crate) cozo_db: Option<Arc<cozo::DbInstance>>,
    /// Governed learning DB for permission/runtime evidence relations.
    pub(crate) governed_learning_db: Option<Arc<cozo::DbInstance>>,
    /// GNN auto-trainer Arc cloned from `SlashCommandContext::auto_trainer`
    /// at dispatch time. Used by `/learning-status` to display live loop state.
    /// Reference: `archon-pipeline/src/learning/gnn/auto_trainer.rs`.
    pub(crate) auto_trainer: Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>>,
    /// GHOST-006: shared sandbox flag, cloned from SlashCommandContext.
    /// Toggled by /sandbox on/off; read by both tool-execution dispatch paths.
    pub(crate) sandbox_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// GHOST-004: shared hook registry for /hooks enable/disable/reload.
    /// Cloned from SlashCommandContext via DIRECT pattern. The handler calls
    /// `set_enabled` / `reload` through this Arc.
    pub(crate) hook_registry: Option<Arc<archon_core::hooks::HookRegistry>>,
    /// GHOST-005: shared plugin enable/disable state. Threaded from
    /// SlashCommandContext via the DIRECT pattern. Keyed by plugin name.
    pub(crate) plugin_enable_state: Option<Arc<RwLock<HashMap<String, bool>>>>,
    /// GHOST-007: late-init slot for AgentHandle (cancel token firing).
    /// Populated inside run_session_loop; None means no session loop active.
    pub(crate) cancel_handle:
        Option<Arc<std::sync::Mutex<Option<Arc<crate::agent_handle::AgentHandle>>>>>,
    /// GHOST-007: AgentDispatcher for is_busy() + cancel_current(). Wrapped
    /// in std::sync::Mutex because cancel_current() takes &mut self and
    /// CommandHandler::execute is sync.
    pub(crate) agent_dispatcher: Option<Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>>,
}

// Wraps `tui_tx.send` at handler call sites so the copy-paste
// `let _ = ctx.tui_tx.send(...)` pattern collapses to `ctx.emit(...)`.
// Saturation is handled inside the bounded sender by progress-event shedding;
// the only error surfaced here is receiver shutdown.
impl CommandContext {
    /// Deliver a [`TuiEvent`] to the TUI channel. Logs on failure;
    /// never panics.
    ///
    /// `TuiEventSender::send` is synchronous (no `.await`), so the method body
    /// is safe to call from sync `CommandHandler::execute`. Saturation is
    /// handled inside the bounded channel; a closed channel still emits the
    /// operator-visible error trace.
    pub(crate) fn emit(&self, event: TuiEvent) {
        if self.tui_tx.send(event).is_err() {
            tracing::error!(
                target: "archon_cli::command::tui",
                "tui_tx closed — TUI receiver task is dead"
            );
        }
    }
}
