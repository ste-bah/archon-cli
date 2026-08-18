//! Tasks-overlay behaviour on `App` (#189 Phase 9).
//!
//! Three registries used to track running work and only one of them could be
//! stopped by anyone. `TASK_MANAGER` had a working cancel path reachable from
//! the `TaskStop` tool but from no human key; `BACKGROUND_AGENTS` had a cancel
//! API with no callers at all; and this overlay emitted `CancelRequested` into
//! the void because nothing constructed it. What Ctrl+O actually shows is the
//! activity stream, which is keyed by actor role and holds no task id, so it
//! can report that work is running and never act on it.
//!
//! This module is the missing connection: the overlay reads rows through
//! `TaskStore` and sends cancellation back through the same trait. The binary
//! supplies the implementation because `archon-tui` depends on `archon-tools`
//! only as a dev-dependency and cannot reach `TASK_MANAGER` itself.
//!
//! Lives outside `app.rs` to keep that file under the 500-line ceiling.

use crate::app::App;
use crate::screens::task_overlay::{TaskAction, TaskId, TaskOverlay, TaskRow};

impl App {
    /// Toggle the tasks overlay, snapshotting the store when opening.
    pub fn toggle_task_overlay(&mut self) {
        if self.task_overlay.is_some() {
            self.task_overlay = None;
            return;
        }
        if self.task_store.is_none() {
            // Distinguish "nothing is running" from "nothing can tell me what is
            // running" — an empty list would quietly claim the former.
            self.output
                .append_line("Tasks overlay unavailable: no task source is attached.");
            return;
        }
        let rows = self.task_rows();
        let mut overlay = TaskOverlay::default();
        overlay.open(rows);
        self.task_overlay = Some(overlay);
    }

    /// Close the overlay without acting on the selection.
    pub fn close_task_overlay(&mut self) {
        self.task_overlay = None;
    }

    /// Re-read the store into an open overlay.
    pub fn refresh_task_overlay(&mut self) {
        let rows = self.task_rows();
        if let Some(overlay) = self.task_overlay.as_mut() {
            overlay.set_rows(rows);
            overlay.clear_action();
        }
    }

    /// Cancel the selected task and report the outcome on the output buffer.
    ///
    /// Returns the id that was acted on, so callers and tests can assert the
    /// selection was honoured rather than inferring it from rendered text.
    pub fn cancel_selected_task(&mut self) -> Option<TaskId> {
        let id = self.selected_task_id()?;
        // Clone the handle before touching the overlay again: the store is
        // borrowed from `self`, and the refresh below needs `self` mutably.
        let store = self.task_store.clone()?;
        match store.cancel_task(&id) {
            Ok(()) => self.output.append_line(&format!("Cancelled task {id}.")),
            Err(reason) => self
                .output
                .append_line(&format!("Could not cancel task {id}: {reason}")),
        }
        let rows = store.list_tasks();
        if let Some(overlay) = self.task_overlay.as_mut() {
            overlay.set_rows(rows);
            overlay.clear_action();
        }
        Some(id)
    }

    /// Ask the overlay which task the cursor is on, via its action channel.
    fn selected_task_id(&mut self) -> Option<TaskId> {
        let overlay = self.task_overlay.as_mut()?;
        overlay.cancel_selected();
        match overlay.last_action() {
            TaskAction::CancelRequested(id) => Some(id),
            _ => None,
        }
    }

    fn task_rows(&self) -> Vec<TaskRow> {
        self.task_store
            .as_ref()
            .map(|store| store.list_tasks())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Records what it was asked to cancel, and drops the row when it succeeds
    /// so a refresh is observable.
    struct RecordingStore {
        rows: Mutex<Vec<TaskRow>>,
        cancelled: Mutex<Vec<TaskId>>,
        fail_with: Option<String>,
    }

    impl RecordingStore {
        fn new(ids: &[&str]) -> Self {
            Self {
                rows: Mutex::new(ids.iter().map(|id| row(id)).collect()),
                cancelled: Mutex::new(Vec::new()),
                fail_with: None,
            }
        }

        fn failing(reason: &str) -> Self {
            Self {
                rows: Mutex::new(vec![row("task-1")]),
                cancelled: Mutex::new(Vec::new()),
                fail_with: Some(reason.to_string()),
            }
        }
    }

    impl crate::screens::task_overlay::TaskStore for RecordingStore {
        fn list_tasks(&self) -> Vec<TaskRow> {
            self.rows.lock().expect("rows lock").clone()
        }

        fn cancel_task(&self, id: &TaskId) -> Result<(), String> {
            self.cancelled.lock().expect("cancelled lock").push(id.clone());
            if let Some(reason) = &self.fail_with {
                return Err(reason.clone());
            }
            self.rows.lock().expect("rows lock").retain(|r| &r.id != id);
            Ok(())
        }
    }

    fn row(id: &str) -> TaskRow {
        TaskRow {
            id: id.to_string(),
            elapsed_secs: 10,
            status: "running".to_string(),
        }
    }

    fn app_with(store: Arc<RecordingStore>) -> App {
        let mut app = App::default();
        app.task_store = Some(store);
        app
    }

    #[test]
    fn toggle_opens_with_rows_from_the_store_and_closes_again() {
        let mut app = app_with(Arc::new(RecordingStore::new(&["task-1", "task-2"])));

        app.toggle_task_overlay();
        assert_eq!(app.task_overlay.as_ref().expect("overlay open").len(), 2);

        app.toggle_task_overlay();
        assert!(app.task_overlay.is_none());
    }

    /// Without a store the overlay must say so rather than render an empty list
    /// that reads as "nothing is running".
    #[test]
    fn toggle_without_a_store_reports_instead_of_showing_an_empty_list() {
        let mut app = App::default();

        app.toggle_task_overlay();

        assert!(app.task_overlay.is_none());
        assert!(
            app.output
                .all_lines()
                .iter()
                .any(|line| line.contains("no task source is attached")),
            "expected an explanatory line, got {:?}",
            app.output.all_lines()
        );
    }

    #[test]
    fn cancelling_acts_on_the_selected_row_not_the_first_one() {
        let store = Arc::new(RecordingStore::new(&["task-1", "task-2", "task-3"]));
        let mut app = app_with(store.clone());
        app.toggle_task_overlay();

        app.task_overlay.as_mut().expect("overlay").move_down();
        let acted = app.cancel_selected_task();

        assert_eq!(acted.as_deref(), Some("task-2"));
        assert_eq!(
            *store.cancelled.lock().expect("cancelled lock"),
            vec!["task-2".to_string()]
        );
    }

    #[test]
    fn a_cancelled_task_leaves_the_overlay_on_the_next_refresh() {
        let store = Arc::new(RecordingStore::new(&["task-1", "task-2"]));
        let mut app = app_with(store);
        app.toggle_task_overlay();

        app.cancel_selected_task();

        assert_eq!(app.task_overlay.as_ref().expect("overlay").len(), 1);
    }

    #[test]
    fn a_refused_cancellation_is_reported_and_keeps_the_row() {
        let store = Arc::new(RecordingStore::failing("task already finished"));
        let mut app = app_with(store);
        app.toggle_task_overlay();

        app.cancel_selected_task();

        assert_eq!(app.task_overlay.as_ref().expect("overlay").len(), 1);
        assert!(
            app.output
                .all_lines()
                .iter()
                .any(|line| line.contains("task already finished")),
            "failure reason must reach the user, got {:?}",
            app.output.all_lines()
        );
    }

    #[test]
    fn cancelling_with_no_overlay_open_does_nothing() {
        let store = Arc::new(RecordingStore::new(&["task-1"]));
        let mut app = app_with(store.clone());

        assert!(app.cancel_selected_task().is_none());
        assert!(store.cancelled.lock().expect("cancelled lock").is_empty());
    }

    #[test]
    fn refresh_picks_up_rows_added_since_the_overlay_opened() {
        let store = Arc::new(RecordingStore::new(&["task-1"]));
        let mut app = app_with(store.clone());
        app.toggle_task_overlay();

        store.rows.lock().expect("rows lock").push(row("task-2"));
        app.refresh_task_overlay();

        assert_eq!(app.task_overlay.as_ref().expect("overlay").len(), 2);
    }
}
