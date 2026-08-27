use super::*;

/// Drain all available events from the receiver.
pub fn drain_tui_events(rx: &mut archon_tui::event_channel::TuiEventReceiver) -> Vec<TuiEvent> {
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    events
}

/// Minimal test-only StatusSnapshot. Values chosen so format-string
/// substitutions are obvious in assertion output.
pub fn fixture_status_snapshot() -> crate::command::status::StatusSnapshot {
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
        // The fixture has no store to fold, so the counts are absent rather
        // than zero — which is also what the status line must print.
        stats: None,
    }
}

/// Minimal test-only ModelSnapshot.
#[allow(dead_code)]
pub fn fixture_model_snapshot() -> crate::command::model::ModelSnapshot {
    crate::command::model::ModelSnapshot {
        current_model: "claude-opus-4-7".to_string(),
        codex_models: archon_core::config::OpenAiCodexModelsConfig::default(),
        anthropic_models: archon_core::config::AnthropicModelsConfig::default(),
    }
}

/// Minimal test-only CostSnapshot. Values chosen so format
/// substitutions are obvious: 1_000_000 input tokens @ $3/Mtok
/// = $3.00, 500_000 output tokens @ $15/Mtok = $7.50, total = $10.50.
pub fn fixture_cost_snapshot() -> crate::command::cost::CostSnapshot {
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
pub fn make_status_ctx(
    snapshot: Option<crate::command::status::StatusSnapshot>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    CtxBuilder::new().with_status_snapshot_opt(snapshot).build()
}

/// Build a CommandContext for ModelHandler tests.
///
/// V2: thin wrapper over `CtxBuilder` (deferred cleanup).
pub fn make_model_ctx(
    snapshot: Option<crate::command::model::ModelSnapshot>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    CtxBuilder::new().with_model_snapshot_opt(snapshot).build()
}

/// Build a CommandContext for CostHandler tests.
///
/// V2: thin wrapper over `CtxBuilder` (deferred cleanup).
pub fn make_cost_ctx(
    snapshot: Option<crate::command::cost::CostSnapshot>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
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
pub fn make_fast_ctx(
    initial: bool,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
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
pub fn make_bug_ctx() -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
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
pub fn make_thinking_ctx(
    initial: bool,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
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
pub fn make_diff_ctx(
    working_dir: Option<std::path::PathBuf>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
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
pub fn make_help_ctx() -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
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
pub fn make_denials_ctx(
    snapshot: Option<crate::command::denials::DenialSnapshot>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
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
pub fn fixture_usage_snapshot() -> crate::command::usage::UsageSnapshot {
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
pub fn make_agent_ctx(
    registry: Option<Arc<std::sync::RwLock<archon_core::agents::AgentRegistry>>>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    CtxBuilder::new().with_agent_registry_opt(registry).build()
}

/// Build a CommandContext for UsageHandler tests.
///
/// TASK-AGS-POST-6-BODIES-B16-USAGE — mirrors `make_cost_ctx` shape
/// exactly but populates `usage_snapshot` (instead of `cost_snapshot`)
/// with the supplied `Option<UsageSnapshot>`. When `None` the handler
/// must return `Err` describing the missing-snapshot wiring regression;
/// when `Some(_)` the handler emits a single byte-identical TextDelta.
pub fn make_usage_ctx(
    snapshot: Option<crate::command::usage::UsageSnapshot>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    CtxBuilder::new().with_usage_snapshot_opt(snapshot).build()
}
