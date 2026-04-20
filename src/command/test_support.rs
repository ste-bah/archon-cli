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
            fast_mode_shared: None,
            // TASK-AGS-POST-6-BODIES-B02-THINKING: /status tests never
            // exercise /thinking paths — None.
            show_thinking: None,
            pending_effect: None,
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
            fast_mode_shared: None,
            // TASK-AGS-POST-6-BODIES-B02-THINKING: /model tests never
            // exercise /thinking paths — None.
            show_thinking: None,
            pending_effect: None,
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
            fast_mode_shared: None,
            // TASK-AGS-POST-6-BODIES-B02-THINKING: /cost tests never
            // exercise /thinking paths — None.
            show_thinking: None,
            pending_effect: None,
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
            fast_mode_shared: Some(Arc::new(AtomicBool::new(initial))),
            // TASK-AGS-POST-6-BODIES-B02-THINKING: /fast tests never
            // exercise /thinking paths — None.
            show_thinking: None,
            pending_effect: None,
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
            fast_mode_shared: None,
            show_thinking: Some(Arc::new(AtomicBool::new(initial))),
            pending_effect: None,
        },
        rx,
    )
}