//! Tasks overlay view (TASK-AGS-620 stub scaffold — Path A).
//!
//! This is the initial scaffolding for the tasks overlay. The full
//! migration — wiring `TuiEvent::CancelTask(TaskId)` and
//! `TuiEvent::RefreshTasks` variants through the app event loop, adding
//! a refresh-interval tokio task in `crate::runtime` that emits
//! `RefreshTasks` ticks, adding a `TasksOverlayState` field on
//! `crate::app::App`, binding a key in `crate::input` to open the
//! overlay, pulling real rows from the `TaskService::list` API,
//! integrating with the `BACKGROUND_AGENTS` registry, and wiring the
//! `/tasks` slash command in Phase 8 — is intentionally deferred to a
//! later phase. See `TASKS_OVERLAY_PLACEHOLDER`.
//!
//! For now this module exposes the minimal API surface (`TaskId`,
//! `TaskRow`, `TaskAction`, `TasksOverlayState`, `open`, `draw`,
//! `on_key`, `on_agent_event`, `refresh`) so subsequent work and tests
//! can depend on it. Task rows are stored as a plain in-memory
//! `Vec<TaskRow>` populated by the caller; the stub does NOT reach out
//! to any real task service or background-agent registry, and `TaskId`
//! is a deliberately local `String` alias rather than the real type
//! that the full migration will introduce.
//!
//! Per the per-view isolation rule, this module MUST NOT import from
//! any other `crate::views::*` module. It also MUST NOT import any new
//! crate-level dependencies (no `archon_session`, no task-service
//! crate, no background-agent registry crate, no `tokio`) — those
//! arrive with the full migration alongside the `TuiEvent::CancelTask`
//! / `TuiEvent::RefreshTasks` variants and the corresponding `app.rs`
//! / `input.rs` / `runtime.rs` wiring.

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

/// Placeholder for the full tasks overlay view content.
///
/// The real `TaskService::list` integration, `BACKGROUND_AGENTS`
/// registry hookup, `TuiEvent::CancelTask` / `TuiEvent::RefreshTasks`
/// plumbing, refresh-interval tokio task in `crate::runtime`,
/// `app.rs` / `input.rs` key binding, and `/tasks` slash command
/// (Phase 8) are deferred to a later task. This constant exists so
/// future modules and tests have a single symbol to point at when
/// wiring tasks overlay content.
pub const TASKS_OVERLAY_PLACEHOLDER: &str =
    "Tasks overlay (placeholder — TuiEvent::CancelTask + TuiEvent::RefreshTasks + runtime refresh tick + app.rs TasksOverlayState field + input.rs key binding + TaskService::list integration + BACKGROUND_AGENTS registry + /tasks slash command wiring deferred)";

/// Local placeholder for a task identifier.
///
/// Deliberately a `String` alias rather than the real task-id type
/// that the full migration will introduce. The full migration will
/// replace this alias with the canonical type and adjust call sites
/// accordingly.
pub type TaskId = String;

/// A single row shown in the tasks overlay list.
///
/// The stub renderer does not yet display these rows — only an empty
/// bordered block is drawn — but they are stored on
/// `TasksOverlayState` so tests and downstream callers can inspect the
/// populated overlay today. The full migration will render these rows
/// via a real `Table` widget driven by `TaskService::list` plus the
/// `BACKGROUND_AGENTS` registry.
#[derive(Debug, Clone, Default)]
pub struct TaskRow {
    /// Unique identifier of the background task this row represents.
    pub id: TaskId,
    /// Human-readable name shown in the task list. The stub does not
    /// render it; the full migration will.
    pub name: String,
    /// Current status string (e.g. "running", "queued", "done"). Kept
    /// as a free-form `String` in the stub; the full migration may
    /// promote this to an enum.
    pub status: String,
    /// Progress percent in the range `0..=100`. Not validated by the
    /// stub.
    pub progress: u8,
    /// CPU usage percent (single core baseline). Display-only field;
    /// the stub does not source this from any real metric.
    pub cpu_pct: f32,
    /// Resident memory usage in megabytes. Display-only field; the
    /// stub does not source this from any real metric.
    pub mem_mb: u32,
}

/// Action emitted by the tasks overlay in response to a key event or
/// refresh tick.
///
/// Stored on `TasksOverlayState::last_action` so callers (and tests)
/// can observe what the overlay would have asked the parent app to do
/// once `TuiEvent::CancelTask` / `TuiEvent::RefreshTasks` exist. The
/// full migration will translate these actions into real `TuiEvent`
/// variants dispatched through `crate::app`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskAction {
    /// No action is currently pending. This is the default and the
    /// state immediately after `Esc`.
    None,
    /// The user pressed `c` on a populated row and would like the
    /// parent app to dispatch a cancel for the carried `TaskId`.
    /// Path A defers the actual `TuiEvent::CancelTask(TaskId)`
    /// variant — this enum value records intent only.
    CancelRequested(TaskId),
    /// The user pressed `Enter` on a populated row and would like to
    /// inspect the carried `TaskId`. Path A defers the inspect-pane
    /// integration — this enum value records intent only.
    InspectRequested(TaskId),
    /// `refresh` was called and the overlay would like the parent app
    /// to dispatch a `TuiEvent::RefreshTasks` tick. Path A defers the
    /// real refresh-interval task in `crate::runtime` — this enum
    /// value records intent only.
    RefreshRequested,
}

impl Default for TaskAction {
    fn default() -> Self {
        Self::None
    }
}

/// State for the tasks overlay.
///
/// Tracks the available task rows, cursor position over those rows,
/// the configured refresh interval (in milliseconds — read by the
/// future `crate::runtime` refresh task), and the most recently
/// emitted `TaskAction`. The stub never populates `tasks` on its own
/// — it exists so call sites stay stable across the full migration.
#[derive(Debug, Default, Clone)]
pub struct TasksOverlayState {
    /// All task rows currently shown by the overlay. Empty by
    /// default; the caller is expected to populate this before
    /// invoking `open`.
    pub tasks: Vec<TaskRow>,
    /// Index of the currently highlighted row. Always clamped to
    /// `0..tasks.len()` (or `0` when `tasks` is empty).
    pub cursor: usize,
    /// Desired refresh interval, in milliseconds. Read by the future
    /// `crate::runtime` refresh task to drive `RefreshTasks` ticks;
    /// the stub does not act on this value itself.
    pub refresh_interval_ms: u64,
    /// The most recently emitted `TaskAction`. Defaults to
    /// `TaskAction::None`. Set by `on_key` and `refresh`; observed by
    /// tests and (eventually) by the full migration's event
    /// translator in `crate::app`.
    pub last_action: TaskAction,
}

/// Open (or re-open) the tasks overlay.
///
/// Stub behaviour: resets `cursor` to `0` so the highlight always
/// lands on the top row when the overlay becomes visible, and clears
/// `last_action` back to `TaskAction::None` so a stale cancel/inspect
/// request from a previous session does not leak across opens. Does
/// NOT touch `tasks` (the caller owns that list) or
/// `refresh_interval_ms` (the caller / future migration owns that).
///
/// The full migration will additionally refresh `tasks` from the real
/// `TaskService::list` API and surface any registry errors via a
/// dedicated error field that does not yet exist on this stub.
pub fn open(state: &mut TasksOverlayState) {
    state.cursor = 0;
    state.last_action = TaskAction::None;
}

/// Draw the tasks overlay into `area`.
///
/// Renders an empty bordered block titled "Tasks" — actual table
/// rendering of `TaskRow` columns (name / status / progress / cpu /
/// mem), highlight bar, and footer hints are deferred to the full
/// migration. The stub deliberately survives an empty `tasks` list
/// without panicking so the overlay can be drawn before any refresh
/// has populated it.
pub fn draw(frame: &mut Frame, area: Rect, _state: &TasksOverlayState) {
    let block = Block::default().borders(Borders::ALL).title("Tasks");
    frame.render_widget(block, area);
}

/// Handle a key event for the tasks overlay.
///
/// Returns `false` because this scaffold does not yet consume input
/// via the parent app event loop in any meaningful way (the
/// `TuiEvent::CancelTask(TaskId)` / `TuiEvent::RefreshTasks` variants
/// that would carry confirmed actions back to `app.rs` do not yet
/// exist — Path A defers them). Cursor movement is clamped to
/// `0..tasks.len()`:
///
/// * `Down` / `j` — advance cursor by one row (clamped at the end)
/// * `Up`   / `k` — retreat cursor by one row (saturating at zero)
/// * `c`          — IF the cursor points inside the populated range,
///                  set `last_action = CancelRequested(id)` carrying
///                  the highlighted row's `id`; otherwise no-op
/// * `Enter`      — IF the cursor points inside the populated range,
///                  set `last_action = InspectRequested(id)` carrying
///                  the highlighted row's `id`; otherwise no-op
/// * `Esc`        — reset `last_action` to `TaskAction::None`
///
/// All other keys are ignored. The full migration will replace this
/// with the real keymap dispatched through `input.rs`.
pub fn on_key(state: &mut TasksOverlayState, key_code: KeyCode) -> bool {
    match key_code {
        KeyCode::Down | KeyCode::Char('j') => {
            let max_index = state.tasks.len().saturating_sub(1);
            if state.cursor < max_index {
                state.cursor += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        KeyCode::Char('c') => {
            if state.cursor < state.tasks.len() {
                let id = state.tasks[state.cursor].id.clone();
                state.last_action = TaskAction::CancelRequested(id);
            }
        }
        KeyCode::Enter => {
            if state.cursor < state.tasks.len() {
                let id = state.tasks[state.cursor].id.clone();
                state.last_action = TaskAction::InspectRequested(id);
            }
        }
        KeyCode::Esc => {
            state.last_action = TaskAction::None;
        }
        _ => {}
    }
    false
}

/// React to an agent event.
///
/// No-op stub. The full implementation will react to the eventual
/// `TuiEvent::RefreshTasks` tick from `crate::runtime` (and any
/// per-task progress events emitted by `BACKGROUND_AGENTS`) by
/// updating `tasks` in place. Those variants do NOT yet exist on
/// `crate::app::TuiEvent` — Path A explicitly defers adding them,
/// along with the `app.rs` `TasksOverlayState` field, the `input.rs`
/// key binding, and the `crate::runtime` refresh-tick task.
pub fn on_agent_event(_state: &mut TasksOverlayState) {}

/// Refresh the task list from the (future) task service and
/// background-agent registry.
///
/// STUB: this function takes `&()` placeholder arguments for both
/// `_task_service` and `_bg` so the call signature can stabilise
/// without depending on any real service type. The stub does NOT call
/// any real `TaskService::list` method or the `BACKGROUND_AGENTS`
/// registry — both are deferred. It simply records that a refresh
/// was requested by setting `last_action = TaskAction::RefreshRequested`,
/// so tests and (eventually) the full migration's event translator in
/// `crate::app` can observe the intent.
///
/// The full migration will:
///   * Replace `_task_service: &()` with a real `&TaskService` (or
///     equivalent trait object) and call its `list` method.
///   * Replace `_bg: &()` with a real reference to the
///     `BACKGROUND_AGENTS` registry and merge its progress data into
///     each `TaskRow`.
///   * Either dispatch a `TuiEvent::RefreshTasks` tick directly or
///     mutate `state.tasks` in place — to be decided alongside the
///     `crate::runtime` refresh-tick task.
pub fn refresh(state: &mut TasksOverlayState, _task_service: &(), _bg: &()) {
    state.last_action = TaskAction::RefreshRequested;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    fn row(id: &str, name: &str) -> TaskRow {
        TaskRow {
            id: id.to_string(),
            name: name.to_string(),
            status: "running".to_string(),
            progress: 0,
            cpu_pct: 0.0,
            mem_mb: 0,
        }
    }

    fn populated_state() -> TasksOverlayState {
        TasksOverlayState {
            tasks: vec![
                row("task-1", "indexing repo"),
                row("task-2", "running tests"),
                row("task-3", "synthesising plan"),
            ],
            cursor: 0,
            refresh_interval_ms: 1000,
            last_action: TaskAction::None,
        }
    }

    #[test]
    fn tasks_overlay_state_default_empty() {
        let state = TasksOverlayState::default();
        assert!(state.tasks.is_empty(), "default tasks should be empty");
        assert_eq!(state.cursor, 0, "default cursor should be 0");
        assert_eq!(
            state.last_action,
            TaskAction::None,
            "default last_action should be TaskAction::None"
        );
    }

    #[test]
    fn on_key_down_advances_cursor() {
        let mut state = populated_state();
        let consumed = on_key(&mut state, KeyCode::Down);
        assert!(!consumed, "stub on_key should not consume input");
        assert_eq!(state.cursor, 1, "Down should advance cursor to 1");

        // 'j' should behave identically to Down.
        let mut state_j = populated_state();
        on_key(&mut state_j, KeyCode::Char('j'));
        assert_eq!(state_j.cursor, 1, "j should advance cursor like Down");
    }

    #[test]
    fn on_key_down_clamps_at_end() {
        let mut state = populated_state();
        state.cursor = state.tasks.len() - 1;
        on_key(&mut state, KeyCode::Down);
        assert_eq!(
            state.cursor,
            state.tasks.len() - 1,
            "Down at last row should keep cursor clamped at tasks.len()-1"
        );
    }

    #[test]
    fn on_key_up_saturates_at_zero() {
        let mut state = populated_state();
        on_key(&mut state, KeyCode::Up);
        assert_eq!(state.cursor, 0, "Up at cursor 0 should saturate at 0");

        // 'k' should behave identically to Up.
        let mut state_k = populated_state();
        on_key(&mut state_k, KeyCode::Char('k'));
        assert_eq!(state_k.cursor, 0, "k at cursor 0 should saturate at 0");
    }

    #[test]
    fn on_key_c_emits_cancel_action() {
        let mut state = populated_state();
        state.cursor = 1;
        on_key(&mut state, KeyCode::Char('c'));
        assert_eq!(
            state.last_action,
            TaskAction::CancelRequested("task-2".to_string()),
            "c should set last_action = CancelRequested(tasks[cursor].id)"
        );
    }

    #[test]
    fn on_key_c_no_op_when_empty() {
        let mut state = TasksOverlayState::default();
        on_key(&mut state, KeyCode::Char('c'));
        assert_eq!(
            state.last_action,
            TaskAction::None,
            "c on empty tasks should not emit a cancel action"
        );
    }

    #[test]
    fn on_key_enter_emits_inspect_action() {
        let mut state = populated_state();
        state.cursor = 2;
        on_key(&mut state, KeyCode::Enter);
        assert_eq!(
            state.last_action,
            TaskAction::InspectRequested("task-3".to_string()),
            "Enter should set last_action = InspectRequested(tasks[cursor].id)"
        );
    }

    #[test]
    fn on_key_esc_resets_last_action() {
        let mut state = populated_state();
        state.last_action = TaskAction::CancelRequested("task-1".to_string());
        on_key(&mut state, KeyCode::Esc);
        assert_eq!(
            state.last_action,
            TaskAction::None,
            "Esc should reset last_action to TaskAction::None"
        );
    }

    #[test]
    fn open_resets_cursor_and_last_action() {
        let mut state = populated_state();
        state.cursor = 2;
        state.last_action = TaskAction::CancelRequested("task-3".to_string());

        open(&mut state);

        assert_eq!(state.cursor, 0, "open should reset cursor to 0");
        assert_eq!(
            state.last_action,
            TaskAction::None,
            "open should reset last_action to TaskAction::None"
        );
        assert_eq!(
            state.tasks.len(),
            3,
            "open should not touch tasks list"
        );
    }

    #[test]
    fn refresh_sets_refresh_requested_action() {
        let mut state = populated_state();
        refresh(&mut state, &(), &());
        assert_eq!(
            state.last_action,
            TaskAction::RefreshRequested,
            "refresh should set last_action = TaskAction::RefreshRequested"
        );
    }

    #[test]
    fn draw_does_not_panic_empty() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = TasksOverlayState::default();
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw should not panic on empty state");
    }

    #[test]
    fn draw_does_not_panic_populated() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = populated_state();
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw should not panic on populated state");
    }
}
