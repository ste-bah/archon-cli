//! Tests for task_overlay refresh.

use archon_tui::screens::task_overlay::{TaskOverlay, TaskStore, TaskRow, TaskAction};

struct MockTaskStore {
    tasks: Vec<TaskRow>,
}

impl TaskStore for MockTaskStore {
    fn list_tasks(&self) -> Vec<TaskRow> {
        self.tasks.clone()
    }
}

fn make_row(id: &str, elapsed_secs: u64, status: &str) -> TaskRow {
    TaskRow {
        id: id.to_string(),
        elapsed_secs,
        status: status.to_string(),
    }
}

#[test]
fn task_overlay_refresh_sets_last_action() {
    let store = MockTaskStore {
        tasks: vec![
            make_row("task-1", 120, "running"),
            make_row("task-2", 60, "queued"),
        ],
    };
    let mut overlay = TaskOverlay::new(store.list_tasks());
    overlay.refresh();
    assert_eq!(overlay.last_action(), TaskAction::RefreshRequested);
}

#[test]
fn task_overlay_open_resets_action() {
    let store = MockTaskStore {
        tasks: vec![make_row("task-1", 10, "running")],
    };
    let mut overlay = TaskOverlay::new(vec![]);
    overlay.refresh();
    overlay.open(store.list_tasks());
    assert_eq!(overlay.last_action(), TaskAction::None);
}

#[test]
fn task_overlay_cancel_selected_emits_action() {
    let store = MockTaskStore {
        tasks: vec![
            make_row("task-1", 10, "running"),
            make_row("task-2", 20, "running"),
        ],
    };
    let mut overlay = TaskOverlay::new(store.list_tasks());
    overlay.move_down();
    overlay.cancel_selected();
    assert_eq!(overlay.last_action(), TaskAction::CancelRequested("task-2".to_string()));
}

#[test]
fn task_overlay_inspect_selected_emits_action() {
    let store = MockTaskStore {
        tasks: vec![make_row("task-1", 10, "running")],
    };
    let mut overlay = TaskOverlay::new(store.list_tasks());
    overlay.inspect_selected();
    assert_eq!(overlay.last_action(), TaskAction::InspectRequested("task-1".to_string()));
}

#[test]
fn task_overlay_set_rows_updates_list() {
    let store = MockTaskStore {
        tasks: vec![
            make_row("task-1", 10, "running"),
            make_row("task-2", 20, "done"),
        ],
    };
    let mut overlay = TaskOverlay::new(vec![]);
    overlay.set_rows(store.list_tasks());
    assert_eq!(overlay.len(), 2);
}

#[test]
fn task_overlay_cursor_wraps() {
    let store = MockTaskStore {
        tasks: vec![
            make_row("task-1", 10, "running"),
            make_row("task-2", 20, "running"),
        ],
    };
    let mut overlay = TaskOverlay::new(store.list_tasks());
    assert_eq!(overlay.selected_index(), 0);
    overlay.move_down();
    assert_eq!(overlay.selected_index(), 1);
    overlay.move_down();
    assert_eq!(overlay.selected_index(), 0); // wrap
}

#[test]
fn task_overlay_open_resets_cursor() {
    let store = MockTaskStore {
        tasks: vec![make_row("task-1", 10, "running")],
    };
    let mut overlay = TaskOverlay::new(vec![]);
    overlay.move_down();
    overlay.open(store.list_tasks());
    assert_eq!(overlay.selected_index(), 0);
}

#[test]
fn task_overlay_clear_action_resets() {
    let store = MockTaskStore {
        tasks: vec![make_row("task-1", 10, "running")],
    };
    let mut overlay = TaskOverlay::new(store.list_tasks());
    overlay.cancel_selected();
    overlay.clear_action();
    assert_eq!(overlay.last_action(), TaskAction::None);
}
