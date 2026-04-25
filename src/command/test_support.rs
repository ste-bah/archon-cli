//! Shared test fixtures for command handler tests.
//! Extracted from Stage 6 body-migrate handlers (AGS-805..819) per Sherlock AGS-820 observation.
//!
//! TASK-AGS-POST-6-SHARED-FIXTURES-V2 (2026-04-21): introduces `CtxBuilder`
//! as the single source of truth for every `CommandContext` field. Handler
//! test modules now call `CtxBuilder::new().with_*(...).build()` instead of
//! inlining a 24-field struct literal. V1 helpers (`make_status_ctx`,
//! `make_model_ctx`, ...) are retained as thin wrappers that delegate to
//! the builder — removing them is deferred to a follow-up.
//!
//! Future CommandContext field additions therefore require editing exactly
//! ONE file (this one) — the builder's `new()` pre-populates every field,
//! and any new `.with_*` setters live alongside the existing ones.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use archon_llm::effort::EffortLevel;
use archon_tui::app::TuiEvent;

use crate::command::registry::CommandContext;

/// Create an unbounded mpsc channel for TuiEvent.
///
/// TASK-SESSION-LOOP-EXTRACT (A-2): flipped from
/// `mpsc::channel::<TuiEvent>(16)` to `mpsc::unbounded_channel`. Matches
/// the production `TuiEvent` channel which was flipped to unbounded to
/// resolve the HRTB Send-bound inference failure at
/// `session_loop::run_session_loop` (rust-lang/rust#102211). Capacity-16
/// semantics are preserved de-facto — tests never produce enough events
/// to matter, and the receiver API (`try_recv`, `recv`) is the same
/// shape. See `session.rs:1414` for full rationale.
pub(crate) fn mock_tui_channel() -> (
    mpsc::UnboundedSender<TuiEvent>,
    mpsc::UnboundedReceiver<TuiEvent>,
) {
    mpsc::unbounded_channel::<TuiEvent>()
}

// ===========================================================================
// CtxBuilder — composable CommandContext fixture
// ===========================================================================

/// Composable builder for `CommandContext` test fixtures.
///
/// `CtxBuilder::new()` pre-populates every `CommandContext` field with a
/// test-safe default (`None` for every `Option`, a fresh bounded
/// `mpsc::channel::<TuiEvent>(16)` for `tui_tx`). `.with_*` setters replace
/// individual fields; `.build()` consumes the builder and returns the
/// `(CommandContext, mpsc::UnboundedReceiver<TuiEvent>)` tuple that every handler
/// test expects.
///
/// All setters are byte-equivalent to the previous inline `CommandContext
/// { ... }` literals — no behavioural drift.
///
/// Channel capacity `16` matches the existing `mock_tui_channel()` and
/// every V1 `make_*_ctx` helper so V1 call sites can migrate without
/// observing a different buffer size.
pub(crate) struct CtxBuilder {
    tui_tx: mpsc::UnboundedSender<TuiEvent>,
    tui_rx: mpsc::UnboundedReceiver<TuiEvent>,
    status_snapshot: Option<crate::command::status::StatusSnapshot>,
    model_snapshot: Option<crate::command::model::ModelSnapshot>,
    cost_snapshot: Option<crate::command::cost::CostSnapshot>,
    mcp_snapshot: Option<crate::command::mcp::McpSnapshot>,
    context_snapshot: Option<crate::command::context_cmd::ContextSnapshot>,
    session_id: Option<String>,
    memory: Option<Arc<dyn archon_memory::MemoryTrait>>,
    garden_config: Option<archon_memory::garden::GardenConfig>,
    fast_mode_shared: Option<Arc<AtomicBool>>,
    show_thinking: Option<Arc<AtomicBool>>,
    working_dir: Option<std::path::PathBuf>,
    skill_registry: Option<Arc<archon_core::skills::SkillRegistry>>,
    denial_snapshot: Option<crate::command::denials::DenialSnapshot>,
    effort_snapshot: Option<crate::command::effort::EffortSnapshot>,
    permissions_snapshot: Option<crate::command::permissions::PermissionsSnapshot>,
    copy_snapshot: Option<crate::command::copy::CopySnapshot>,
    doctor_snapshot: Option<crate::command::doctor::DoctorSnapshot>,
    usage_snapshot: Option<crate::command::usage::UsageSnapshot>,
    config_path: Option<std::path::PathBuf>,
    auth_label: Option<String>,
    pending_effect: Option<crate::command::registry::CommandEffect>,
    pending_effort_set: Option<EffortLevel>,
    pending_export: Option<Arc<Mutex<Option<crate::command::export::ExportDescriptor>>>>,
    // TASK-#211 SLASH-AGENT: agent registry handle for /agent.
    agent_registry: Option<Arc<std::sync::RwLock<archon_core::agents::AgentRegistry>>>,
}

impl CtxBuilder {
    /// Create a new builder with every field set to its test-safe default
    /// and a fresh bounded mpsc channel (capacity 16, matching V1).
    pub(crate) fn new() -> Self {
        let (tx, rx) = mock_tui_channel();
        Self {
            tui_tx: tx,
            tui_rx: rx,
            status_snapshot: None,
            model_snapshot: None,
            cost_snapshot: None,
            mcp_snapshot: None,
            context_snapshot: None,
            session_id: None,
            memory: None,
            garden_config: None,
            fast_mode_shared: None,
            show_thinking: None,
            working_dir: None,
            skill_registry: None,
            denial_snapshot: None,
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            usage_snapshot: None,
            config_path: None,
            auth_label: None,
            pending_effect: None,
            pending_effort_set: None,
            pending_export: None,
            agent_registry: None,
        }
    }

    pub(crate) fn with_status_snapshot(
        mut self,
        s: crate::command::status::StatusSnapshot,
    ) -> Self {
        self.status_snapshot = Some(s);
        self
    }

    pub(crate) fn with_status_snapshot_opt(
        mut self,
        s: Option<crate::command::status::StatusSnapshot>,
    ) -> Self {
        self.status_snapshot = s;
        self
    }

    pub(crate) fn with_model_snapshot(mut self, s: crate::command::model::ModelSnapshot) -> Self {
        self.model_snapshot = Some(s);
        self
    }

    pub(crate) fn with_model_snapshot_opt(
        mut self,
        s: Option<crate::command::model::ModelSnapshot>,
    ) -> Self {
        self.model_snapshot = s;
        self
    }

    pub(crate) fn with_cost_snapshot(mut self, s: crate::command::cost::CostSnapshot) -> Self {
        self.cost_snapshot = Some(s);
        self
    }

    pub(crate) fn with_cost_snapshot_opt(
        mut self,
        s: Option<crate::command::cost::CostSnapshot>,
    ) -> Self {
        self.cost_snapshot = s;
        self
    }

    pub(crate) fn with_mcp_snapshot(mut self, s: crate::command::mcp::McpSnapshot) -> Self {
        self.mcp_snapshot = Some(s);
        self
    }

    pub(crate) fn with_mcp_snapshot_opt(
        mut self,
        s: Option<crate::command::mcp::McpSnapshot>,
    ) -> Self {
        self.mcp_snapshot = s;
        self
    }

    pub(crate) fn with_context_snapshot(
        mut self,
        s: crate::command::context_cmd::ContextSnapshot,
    ) -> Self {
        self.context_snapshot = Some(s);
        self
    }

    pub(crate) fn with_context_snapshot_opt(
        mut self,
        s: Option<crate::command::context_cmd::ContextSnapshot>,
    ) -> Self {
        self.context_snapshot = s;
        self
    }

    pub(crate) fn with_session_id(mut self, id: String) -> Self {
        self.session_id = Some(id);
        self
    }

    pub(crate) fn with_session_id_opt(mut self, id: Option<String>) -> Self {
        self.session_id = id;
        self
    }

    pub(crate) fn with_memory(mut self, memory: Arc<dyn archon_memory::MemoryTrait>) -> Self {
        self.memory = Some(memory);
        self
    }

    pub(crate) fn with_memory_opt(
        mut self,
        memory: Option<Arc<dyn archon_memory::MemoryTrait>>,
    ) -> Self {
        self.memory = memory;
        self
    }

    pub(crate) fn with_garden_config(mut self, c: archon_memory::garden::GardenConfig) -> Self {
        self.garden_config = Some(c);
        self
    }

    pub(crate) fn with_garden_config_opt(
        mut self,
        c: Option<archon_memory::garden::GardenConfig>,
    ) -> Self {
        self.garden_config = c;
        self
    }

    pub(crate) fn with_fast_mode_shared(mut self, shared: Arc<AtomicBool>) -> Self {
        self.fast_mode_shared = Some(shared);
        self
    }

    pub(crate) fn with_show_thinking(mut self, shared: Arc<AtomicBool>) -> Self {
        self.show_thinking = Some(shared);
        self
    }

    pub(crate) fn with_working_dir(mut self, path: std::path::PathBuf) -> Self {
        self.working_dir = Some(path);
        self
    }

    pub(crate) fn with_working_dir_opt(mut self, path: Option<std::path::PathBuf>) -> Self {
        self.working_dir = path;
        self
    }

    pub(crate) fn with_skill_registry(
        mut self,
        reg: Arc<archon_core::skills::SkillRegistry>,
    ) -> Self {
        self.skill_registry = Some(reg);
        self
    }

    pub(crate) fn with_denial_snapshot(
        mut self,
        s: crate::command::denials::DenialSnapshot,
    ) -> Self {
        self.denial_snapshot = Some(s);
        self
    }

    pub(crate) fn with_denial_snapshot_opt(
        mut self,
        s: Option<crate::command::denials::DenialSnapshot>,
    ) -> Self {
        self.denial_snapshot = s;
        self
    }

    pub(crate) fn with_effort_snapshot(
        mut self,
        s: crate::command::effort::EffortSnapshot,
    ) -> Self {
        self.effort_snapshot = Some(s);
        self
    }

    pub(crate) fn with_effort_snapshot_opt(
        mut self,
        s: Option<crate::command::effort::EffortSnapshot>,
    ) -> Self {
        self.effort_snapshot = s;
        self
    }

    pub(crate) fn with_permissions_snapshot(
        mut self,
        s: crate::command::permissions::PermissionsSnapshot,
    ) -> Self {
        self.permissions_snapshot = Some(s);
        self
    }

    pub(crate) fn with_permissions_snapshot_opt(
        mut self,
        s: Option<crate::command::permissions::PermissionsSnapshot>,
    ) -> Self {
        self.permissions_snapshot = s;
        self
    }

    pub(crate) fn with_copy_snapshot(mut self, s: crate::command::copy::CopySnapshot) -> Self {
        self.copy_snapshot = Some(s);
        self
    }

    pub(crate) fn with_copy_snapshot_opt(
        mut self,
        s: Option<crate::command::copy::CopySnapshot>,
    ) -> Self {
        self.copy_snapshot = s;
        self
    }

    pub(crate) fn with_doctor_snapshot(
        mut self,
        s: crate::command::doctor::DoctorSnapshot,
    ) -> Self {
        self.doctor_snapshot = Some(s);
        self
    }

    pub(crate) fn with_doctor_snapshot_opt(
        mut self,
        s: Option<crate::command::doctor::DoctorSnapshot>,
    ) -> Self {
        self.doctor_snapshot = s;
        self
    }

    pub(crate) fn with_usage_snapshot(mut self, s: crate::command::usage::UsageSnapshot) -> Self {
        self.usage_snapshot = Some(s);
        self
    }

    pub(crate) fn with_usage_snapshot_opt(
        mut self,
        s: Option<crate::command::usage::UsageSnapshot>,
    ) -> Self {
        self.usage_snapshot = s;
        self
    }

    pub(crate) fn with_config_path(mut self, path: std::path::PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    pub(crate) fn with_config_path_opt(mut self, path: Option<std::path::PathBuf>) -> Self {
        self.config_path = path;
        self
    }

    pub(crate) fn with_auth_label(mut self, label: String) -> Self {
        self.auth_label = Some(label);
        self
    }

    pub(crate) fn with_auth_label_opt(mut self, label: Option<String>) -> Self {
        self.auth_label = label;
        self
    }

    pub(crate) fn with_pending_effect(
        mut self,
        e: crate::command::registry::CommandEffect,
    ) -> Self {
        self.pending_effect = Some(e);
        self
    }

    pub(crate) fn with_pending_effort_set(mut self, level: EffortLevel) -> Self {
        self.pending_effort_set = Some(level);
        self
    }

    pub(crate) fn with_pending_export(
        mut self,
        slot: Arc<Mutex<Option<crate::command::export::ExportDescriptor>>>,
    ) -> Self {
        self.pending_export = Some(slot);
        self
    }

    /// TASK-#211 SLASH-AGENT: install an agent registry handle.
    pub(crate) fn with_agent_registry(
        mut self,
        reg: Arc<std::sync::RwLock<archon_core::agents::AgentRegistry>>,
    ) -> Self {
        self.agent_registry = Some(reg);
        self
    }

    pub(crate) fn with_agent_registry_opt(
        mut self,
        reg: Option<Arc<std::sync::RwLock<archon_core::agents::AgentRegistry>>>,
    ) -> Self {
        self.agent_registry = reg;
        self
    }

    /// Consume the builder and return `(CommandContext, Receiver)`.
    pub(crate) fn build(self) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
        (
            CommandContext {
                tui_tx: self.tui_tx,
                status_snapshot: self.status_snapshot,
                model_snapshot: self.model_snapshot,
                cost_snapshot: self.cost_snapshot,
                mcp_snapshot: self.mcp_snapshot,
                context_snapshot: self.context_snapshot,
                session_id: self.session_id,
                memory: self.memory,
                garden_config: self.garden_config,
                fast_mode_shared: self.fast_mode_shared,
                show_thinking: self.show_thinking,
                working_dir: self.working_dir,
                skill_registry: self.skill_registry,
                denial_snapshot: self.denial_snapshot,
                effort_snapshot: self.effort_snapshot,
                permissions_snapshot: self.permissions_snapshot,
                copy_snapshot: self.copy_snapshot,
                doctor_snapshot: self.doctor_snapshot,
                usage_snapshot: self.usage_snapshot,
                config_path: self.config_path,
                auth_label: self.auth_label,
                pending_effect: self.pending_effect,
                pending_effort_set: self.pending_effort_set,
                pending_export: self.pending_export,
                agent_registry: self.agent_registry,
            },
            self.tui_rx,
        )
    }
}

/// Drain all available events from the receiver.
pub(crate) fn drain_tui_events(rx: &mut mpsc::UnboundedReceiver<TuiEvent>) -> Vec<TuiEvent> {
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    events
}

/// Minimal test-only StatusSnapshot. Values chosen so format-string
/// substitutions are obvious in assertion output.
pub(crate) fn fixture_status_snapshot() -> crate::command::status::StatusSnapshot {
    crate::command::status::StatusSnapshot {
        current_model: "claude-opus-4-7".to_string(),
        perm_mode: "default".to_string(),
        fast_mode: false,
        effort: EffortLevel::Medium,
        thinking_visible: false,
        session_id_short: "abcd1234".to_string(),
        input_tokens: 1234,
        output_tokens: 567,
        turn_count: 3,
    }
}

/// Minimal test-only ModelSnapshot.
pub(crate) fn fixture_model_snapshot() -> crate::command::model::ModelSnapshot {
    crate::command::model::ModelSnapshot {
        current_model: "claude-opus-4-7".to_string(),
    }
}

/// Minimal test-only CostSnapshot. Values chosen so format
/// substitutions are obvious: 1_000_000 input tokens @ $3/Mtok
/// = $3.00, 500_000 output tokens @ $15/Mtok = $7.50, total = $10.50.
pub(crate) fn fixture_cost_snapshot() -> crate::command::cost::CostSnapshot {
    crate::command::cost::CostSnapshot {
        input_tokens: 1_000_000,
        output_tokens: 500_000,
        input_cost: 3.00,
        output_cost: 7.50,
        total_cost: 10.50,
        cache_stats_line: "Cache hit rate: 0.0% (0 reads / 0 total)\n\
             Cache creation: 0 tokens\n\
             Estimated savings: 0 token-equivalents"
            .to_string(),
        warn_threshold: 5.0,
        hard_label: "$0.00 (disabled)".to_string(),
    }
}

/// Build a CommandContext for StatusHandler tests.
///
/// V2: thin wrapper over `CtxBuilder` (deferred cleanup).
pub(crate) fn make_status_ctx(
    snapshot: Option<crate::command::status::StatusSnapshot>,
) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new().with_status_snapshot_opt(snapshot).build()
}

/// Build a CommandContext for ModelHandler tests.
///
/// V2: thin wrapper over `CtxBuilder` (deferred cleanup).
pub(crate) fn make_model_ctx(
    snapshot: Option<crate::command::model::ModelSnapshot>,
) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new().with_model_snapshot_opt(snapshot).build()
}

/// Build a CommandContext for CostHandler tests.
///
/// V2: thin wrapper over `CtxBuilder` (deferred cleanup).
pub(crate) fn make_cost_ctx(
    snapshot: Option<crate::command::cost::CostSnapshot>,
) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new().with_cost_snapshot_opt(snapshot).build()
}

/// Build a CommandContext for FastHandler tests.
///
/// TASK-AGS-POST-6-BODIES-B01-FAST — mirrors `make_status_ctx` /
/// `make_model_ctx` / `make_cost_ctx` but populates
/// `fast_mode_shared` with a freshly-allocated
/// `Arc<AtomicBool>::new(initial)` so the handler's sync
/// load-invert-store toggle sees a real shared atomic. All other
/// optional fields are left at `None` — mirroring peer helpers.
///
/// V2: thin wrapper over `CtxBuilder` (deferred cleanup).
pub(crate) fn make_fast_ctx(initial: bool) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new()
        .with_fast_mode_shared(Arc::new(AtomicBool::new(initial)))
        .build()
}

/// Build a CommandContext for BugHandler tests.
///
/// TASK-AGS-POST-6-BODIES-B03-BUG — trivial-variant DIRECT helper. The
/// `/bug` handler adds NO new CommandContext field (no shared atomic,
/// no snapshot, no memory handle), so every optional field is left at
/// `None`. The helper mirrors the `make_status_ctx`-with-`None`-snapshot
/// shape: wire a mock TuiEvent channel and nothing else. No
/// peer-fixture rollout was needed because no new struct field was
/// added for this ticket.
pub(crate) fn make_bug_ctx() -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new().build()
}

/// Build a CommandContext for ThinkingHandler tests.
///
/// TASK-AGS-POST-6-BODIES-B02-THINKING — mirrors `make_fast_ctx` shape
/// exactly but populates `show_thinking` (instead of
/// `fast_mode_shared`) with a freshly-allocated
/// `Arc<AtomicBool>::new(initial)` so the handler's sync
/// store-on-parsed-subcommand sees a real shared atomic. All other
/// optional fields — including `fast_mode_shared` — are left at
/// `None`, mirroring peer helpers.
///
/// Suppress warning: `Ordering` from atomic is held by the inner
/// `Arc<AtomicBool>`; the helper itself never reads or stores.
///
/// V2: thin wrapper over `CtxBuilder` (deferred cleanup).
pub(crate) fn make_thinking_ctx(
    initial: bool,
) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new()
        .with_show_thinking(Arc::new(AtomicBool::new(initial)))
        .build()
}

/// Build a CommandContext for DiffHandler tests.
///
/// TASK-AGS-POST-6-BODIES-B04-DIFF — DIRECT with-effect variant. The
/// `/diff` handler reads `working_dir` to stash a
/// `CommandEffect::RunGitDiffStat(PathBuf)`. Helper signature takes an
/// `Option<PathBuf>` so a single helper covers both the Some-path and
/// None-sentinel test cases without a second constructor.
///
/// When `working_dir` is `Some(path)` the handler must stash the
/// effect and emit zero events directly. When `working_dir` is `None`
/// the handler must emit exactly one `TuiEvent::Error` describing the
/// missing-shared-state condition and leave `pending_effect` at `None`
/// (mirroring B01-FAST's `fast_mode_shared=None` handling pattern).
pub(crate) fn make_diff_ctx(
    working_dir: Option<std::path::PathBuf>,
) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new().with_working_dir_opt(working_dir).build()
}

/// Build a CommandContext for HelpHandler tests.
///
/// TASK-AGS-POST-6-BODIES-B06-HELP — DIRECT-with-field variant. The
/// `/help` handler reads `skill_registry` to call sync
/// `SkillRegistry::format_help()` (empty-args suffix) or
/// `format_skill_help(name)` (single-command detail). Helper populates
/// `skill_registry` with a freshly-built `Arc<SkillRegistry>` containing
/// one known skill (`help`) so:
///
///   - `format_help()` output contains the `Available commands:` header
///     plus the registered `/help` entry — observable from the
///     handler's empty-args TextDelta.
///   - `format_skill_help("help")` returns `Some(...)` — observable
///     from the single-command TextDelta path.
///   - `format_skill_help("bogusname")` returns `None` — observable
///     from the unknown-command Error path.
///
/// All other optional fields are left at `None`, mirroring peer
/// helpers.
pub(crate) fn make_help_ctx() -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    use archon_core::skills::SkillRegistry;
    use archon_core::skills::builtin::HelpSkill;
    let mut registry = SkillRegistry::new();
    registry.register(Box::new(HelpSkill));
    CtxBuilder::new()
        .with_skill_registry(Arc::new(registry))
        .build()
}

/// Build a CommandContext for DenialsHandler tests.
///
/// TASK-AGS-POST-6-BODIES-B08-DENIALS — SNAPSHOT-ONLY variant. The
/// `/denials` handler reads `denial_snapshot` to emit the pre-computed
/// `DenialLog::format_display(20)` text wrapped with `\n{text}\n`.
/// Helper signature takes an `Option<DenialSnapshot>` so a single helper
/// covers both the Some-path (happy, emit TextDelta) and
/// None-defensive-panic cases without a second constructor. Mirrors
/// `make_status_ctx` / `make_cost_ctx` / `make_mcp_ctx` snapshot-helper
/// shape.
pub(crate) fn make_denials_ctx(
    snapshot: Option<crate::command::denials::DenialSnapshot>,
) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new().with_denial_snapshot_opt(snapshot).build()
}

/// Minimal test-only UsageSnapshot. Values chosen so format
/// substitutions are obvious: 1_000_000 input tokens @ $3/Mtok =
/// $3.00, 500_000 output tokens @ $15/Mtok = $7.50, total = $10.50,
/// 3 turns. `cache_stats_line` matches the canonical zero-activity
/// output of `CacheStats::format_for_cost()`.
///
/// TASK-AGS-POST-6-BODIES-B16-USAGE — mirrors `fixture_cost_snapshot`
/// above but drops `warn_threshold` and `hard_label` (/usage uses the
/// `.4`-precision byte-identical shipped format at slash.rs:315-336
/// which has no Warn/Hard lines) and adds `turn_count` (/usage is the
/// only command that surfaces turn count).
pub(crate) fn fixture_usage_snapshot() -> crate::command::usage::UsageSnapshot {
    crate::command::usage::UsageSnapshot {
        input_tokens: 1_000_000,
        output_tokens: 500_000,
        turn_count: 3,
        input_cost: 3.00,
        output_cost: 7.50,
        total_cost: 10.50,
        cache_stats_line: "Cache hit rate: 0.0% (0 reads / 0 total)\n\
             Cache creation: 0 tokens\n\
             Estimated savings: 0 token-equivalents"
            .to_string(),
    }
}

/// Build a CommandContext for AgentHandler tests.
///
/// TASK-#211 SLASH-AGENT — DIRECT-with-field variant. Mirrors
/// `make_help_ctx` shape but populates `agent_registry` (instead of
/// `skill_registry`). When `Some(arc)` is supplied the handler reads
/// the registry via `RwLock::read()`; when `None` is supplied the
/// handler returns Err describing the missing-registry condition.
pub(crate) fn make_agent_ctx(
    registry: Option<Arc<std::sync::RwLock<archon_core::agents::AgentRegistry>>>,
) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new()
        .with_agent_registry_opt(registry)
        .build()
}

/// Build a CommandContext for UsageHandler tests.
///
/// TASK-AGS-POST-6-BODIES-B16-USAGE — mirrors `make_cost_ctx` shape
/// exactly but populates `usage_snapshot` (instead of `cost_snapshot`)
/// with the supplied `Option<UsageSnapshot>`. When `None` the handler
/// must return `Err` describing the missing-snapshot wiring regression;
/// when `Some(_)` the handler emits a single byte-identical TextDelta.
pub(crate) fn make_usage_ctx(
    snapshot: Option<crate::command::usage::UsageSnapshot>,
) -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
    CtxBuilder::new().with_usage_snapshot_opt(snapshot).build()
}
