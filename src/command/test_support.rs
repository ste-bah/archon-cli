//! Shared test fixtures for command handler tests.
//! Extracted from Stage 6 body-migrate handlers (AGS-805..819) per Sherlock AGS-820 observation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use archon_llm::effort::EffortLevel;
use archon_tui::app::TuiEvent;

use crate::command::registry::CommandContext;

/// Create a bounded mpsc channel with capacity 16.
pub(crate) fn mock_tui_channel() -> (mpsc::Sender<TuiEvent>, mpsc::Receiver<TuiEvent>) {
    mpsc::channel::<TuiEvent>(16)
}

/// Drain all available events from the receiver.
pub(crate) fn drain_tui_events(rx: &mut mpsc::Receiver<TuiEvent>) -> Vec<TuiEvent> {
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
        cache_stats_line:
            "Cache hit rate: 0.0% (0 reads / 0 total)\n\
             Cache creation: 0 tokens\n\
             Estimated savings: 0 token-equivalents"
                .to_string(),
        warn_threshold: 5.0,
        hard_label: "$0.00 (disabled)".to_string(),
    }
}

/// Build a CommandContext for StatusHandler tests.
pub(crate) fn make_status_ctx(
    snapshot: Option<crate::command::status::StatusSnapshot>,
) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
    let (tx, rx) = mock_tui_channel();
    (
        CommandContext {
            tui_tx: tx,
            status_snapshot: snapshot,
            model_snapshot: None,
            cost_snapshot: None,
            mcp_snapshot: None,
            context_snapshot: None,
            session_id: None,
            memory: None,
            garden_config: None,
            fast_mode_shared: None,
            // TASK-AGS-POST-6-BODIES-B02-THINKING: /status tests never
            // exercise /thinking paths — None.
            show_thinking: None,
            // TASK-AGS-POST-6-BODIES-B04-DIFF: /status tests never
            // exercise /diff paths — None.
            working_dir: None,
            // TASK-AGS-POST-6-BODIES-B06-HELP: /status tests never
            // exercise /help paths — None.
            skill_registry: None,
            // TASK-AGS-POST-6-BODIES-B08-DENIALS: peer fixtures never
            // exercise /denials paths — None.
            denial_snapshot: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            pending_effort_set: None,
        },
        rx,
    )
}

/// Build a CommandContext for ModelHandler tests.
pub(crate) fn make_model_ctx(
    snapshot: Option<crate::command::model::ModelSnapshot>,
) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
    let (tx, rx) = mock_tui_channel();
    (
        CommandContext {
            tui_tx: tx,
            status_snapshot: None,
            model_snapshot: snapshot,
            cost_snapshot: None,
            mcp_snapshot: None,
            context_snapshot: None,
            session_id: None,
            memory: None,
            garden_config: None,
            fast_mode_shared: None,
            // TASK-AGS-POST-6-BODIES-B02-THINKING: /model tests never
            // exercise /thinking paths — None.
            show_thinking: None,
            // TASK-AGS-POST-6-BODIES-B04-DIFF: /model tests never
            // exercise /diff paths — None.
            working_dir: None,
            // TASK-AGS-POST-6-BODIES-B06-HELP: /model tests never
            // exercise /help paths — None.
            skill_registry: None,
            // TASK-AGS-POST-6-BODIES-B08-DENIALS: peer fixtures never
            // exercise /denials paths — None.
            denial_snapshot: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            pending_effort_set: None,
        },
        rx,
    )
}

/// Build a CommandContext for CostHandler tests.
pub(crate) fn make_cost_ctx(
    snapshot: Option<crate::command::cost::CostSnapshot>,
) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
    let (tx, rx) = mock_tui_channel();
    (
        CommandContext {
            tui_tx: tx,
            status_snapshot: None,
            model_snapshot: None,
            cost_snapshot: snapshot,
            mcp_snapshot: None,
            context_snapshot: None,
            session_id: None,
            memory: None,
            garden_config: None,
            fast_mode_shared: None,
            // TASK-AGS-POST-6-BODIES-B02-THINKING: /cost tests never
            // exercise /thinking paths — None.
            show_thinking: None,
            // TASK-AGS-POST-6-BODIES-B04-DIFF: /cost tests never
            // exercise /diff paths — None.
            working_dir: None,
            // TASK-AGS-POST-6-BODIES-B06-HELP: /cost tests never
            // exercise /help paths — None.
            skill_registry: None,
            // TASK-AGS-POST-6-BODIES-B08-DENIALS: peer fixtures never
            // exercise /denials paths — None.
            denial_snapshot: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            pending_effort_set: None,
        },
        rx,
    )
}

/// Build a CommandContext for FastHandler tests.
///
/// TASK-AGS-POST-6-BODIES-B01-FAST — mirrors `make_status_ctx` /
/// `make_model_ctx` / `make_cost_ctx` but populates
/// `fast_mode_shared` with a freshly-allocated
/// `Arc<AtomicBool>::new(initial)` so the handler's sync
/// load-invert-store toggle sees a real shared atomic. All other
/// optional fields are left at `None` — mirroring peer helpers.
pub(crate) fn make_fast_ctx(
    initial: bool,
) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
    let (tx, rx) = mock_tui_channel();
    (
        CommandContext {
            tui_tx: tx,
            status_snapshot: None,
            model_snapshot: None,
            cost_snapshot: None,
            mcp_snapshot: None,
            context_snapshot: None,
            session_id: None,
            memory: None,
            garden_config: None,
            fast_mode_shared: Some(Arc::new(AtomicBool::new(initial))),
            // TASK-AGS-POST-6-BODIES-B02-THINKING: /fast tests never
            // exercise /thinking paths — None.
            show_thinking: None,
            // TASK-AGS-POST-6-BODIES-B04-DIFF: /fast tests never
            // exercise /diff paths — None.
            working_dir: None,
            // TASK-AGS-POST-6-BODIES-B06-HELP: /fast tests never
            // exercise /help paths — None.
            skill_registry: None,
            // TASK-AGS-POST-6-BODIES-B08-DENIALS: peer fixtures never
            // exercise /denials paths — None.
            denial_snapshot: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            pending_effort_set: None,
        },
        rx,
    )
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
pub(crate) fn make_bug_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
    let (tx, rx) = mock_tui_channel();
    (
        CommandContext {
            tui_tx: tx,
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
            // TASK-AGS-POST-6-BODIES-B04-DIFF: /bug tests never
            // exercise /diff paths — None.
            working_dir: None,
            // TASK-AGS-POST-6-BODIES-B06-HELP: /bug tests never
            // exercise /help paths — None.
            skill_registry: None,
            // TASK-AGS-POST-6-BODIES-B08-DENIALS: peer fixtures never
            // exercise /denials paths — None.
            denial_snapshot: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            pending_effort_set: None,
        },
        rx,
    )
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
pub(crate) fn make_thinking_ctx(
    initial: bool,
) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
    let (tx, rx) = mock_tui_channel();
    (
        CommandContext {
            tui_tx: tx,
            status_snapshot: None,
            model_snapshot: None,
            cost_snapshot: None,
            mcp_snapshot: None,
            context_snapshot: None,
            session_id: None,
            memory: None,
            garden_config: None,
            fast_mode_shared: None,
            show_thinking: Some(Arc::new(AtomicBool::new(initial))),
            // TASK-AGS-POST-6-BODIES-B04-DIFF: /thinking tests never
            // exercise /diff paths — None.
            working_dir: None,
            // TASK-AGS-POST-6-BODIES-B06-HELP: /thinking tests never
            // exercise /help paths — None.
            skill_registry: None,
            // TASK-AGS-POST-6-BODIES-B08-DENIALS: peer fixtures never
            // exercise /denials paths — None.
            denial_snapshot: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            pending_effort_set: None,
        },
        rx,
    )
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
) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
    let (tx, rx) = mock_tui_channel();
    (
        CommandContext {
            tui_tx: tx,
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
            working_dir,
            // TASK-AGS-POST-6-BODIES-B06-HELP: /diff tests never
            // exercise /help paths — None.
            skill_registry: None,
            // TASK-AGS-POST-6-BODIES-B08-DENIALS: peer fixtures never
            // exercise /denials paths — None.
            denial_snapshot: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            pending_effort_set: None,
        },
        rx,
    )
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
pub(crate) fn make_help_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
    use archon_core::skills::builtin::HelpSkill;
    use archon_core::skills::SkillRegistry;
    let (tx, rx) = mock_tui_channel();
    let mut registry = SkillRegistry::new();
    registry.register(Box::new(HelpSkill));
    (
        CommandContext {
            tui_tx: tx,
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
            skill_registry: Some(Arc::new(registry)),
            // TASK-AGS-POST-6-BODIES-B08-DENIALS: /help tests never
            // exercise /denials paths — None.
            denial_snapshot: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            pending_effort_set: None,
        },
        rx,
    )
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
) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
    let (tx, rx) = mock_tui_channel();
    (
        CommandContext {
            tui_tx: tx,
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
            // TASK-AGS-POST-6-BODIES-B06-HELP: /denials tests never
            // exercise /help paths — None.
            skill_registry: None,
            denial_snapshot: snapshot,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            effort_snapshot: None,
            permissions_snapshot: None,
            copy_snapshot: None,
            doctor_snapshot: None,
            pending_effect: None,
            // TASK-AGS-POST-6-BODIES-B11-EFFORT: peer fixtures never
            // exercise /effort paths — None.
            pending_effort_set: None,
        },
        rx,
    )
}