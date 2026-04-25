//! Tasks overlay screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Block, Borders, Row, Table};

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// Task identifier.
pub type TaskId = String;

/// A single row in the tasks overlay list.
#[derive(Debug, Clone)]
pub struct TaskRow {
    /// Unique identifier of the task.
    pub id: TaskId,
    /// Elapsed time in seconds.
    pub elapsed_secs: u64,
    /// Current status string (e.g. "running", "queued", "done").
    pub status: String,
}

/// Task store trait for injectable task data source.
pub trait TaskStore: Send + Sync {
    fn list_tasks(&self) -> Vec<TaskRow>;
}

/// Action emitted by the tasks overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskAction {
    None,
    CancelRequested(TaskId),
    InspectRequested(TaskId),
    RefreshRequested,
}

impl Default for TaskAction {
    fn default() -> Self {
        Self::None
    }
}

/// Tasks overlay state with virtualized scrolling.
#[derive(Debug)]
pub struct TaskOverlay {
    rows: VirtualList<TaskRow>,
    last_action: TaskAction,
}

impl TaskOverlay {
    /// Create a new TaskOverlay with the given initial rows.
    pub fn new(rows: Vec<TaskRow>) -> Self {
        Self {
            rows: VirtualList::new(rows, 10),
            last_action: TaskAction::None,
        }
    }

    /// Returns true if the overlay has no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Returns the number of rows in the overlay.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns the currently selected row index.
    pub fn selected_index(&self) -> usize {
        self.rows.selected_index()
    }

    /// Returns a reference to the currently selected row, if any.
    pub fn selected(&self) -> Option<&TaskRow> {
        self.rows.selected()
    }

    /// Returns the last emitted action.
    pub fn last_action(&self) -> TaskAction {
        self.last_action.clone()
    }

    /// Set task rows.
    pub fn set_rows(&mut self, rows: Vec<TaskRow>) {
        self.rows.set_items(rows);
    }

    /// Open (or re-open) the overlay with the given rows.
    pub fn open(&mut self, rows: Vec<TaskRow>) {
        self.rows.set_items(rows);
        self.last_action = TaskAction::None;
    }

    /// Move selection up (wrapping to last if at top).
    pub fn move_up(&mut self) {
        self.rows.move_up();
    }

    /// Move selection down (wrapping to first if at bottom).
    pub fn move_down(&mut self) {
        self.rows.move_down();
    }

    /// Move selection up by one page.
    pub fn page_up(&mut self) {
        self.rows.page_up();
    }

    /// Move selection down by one page.
    pub fn page_down(&mut self) {
        self.rows.page_down();
    }

    /// Request cancel for the currently selected task.
    pub fn cancel_selected(&mut self) {
        if let Some(row) = self.rows.selected() {
            self.last_action = TaskAction::CancelRequested(row.id.clone());
        }
    }

    /// Request inspect for the currently selected task.
    pub fn inspect_selected(&mut self) {
        if let Some(row) = self.rows.selected() {
            self.last_action = TaskAction::InspectRequested(row.id.clone());
        }
    }

    /// Request a refresh of the task list.
    pub fn refresh(&mut self) {
        self.last_action = TaskAction::RefreshRequested;
    }

    /// Clear the last emitted action.
    pub fn clear_action(&mut self) {
        self.last_action = TaskAction::None;
    }

    /// Render the tasks overlay into the given area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default().borders(Borders::ALL).title("Tasks");

        let widths = [
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(50),
        ];

        let header = Row::new(["ID", "Elapsed", "Status"]);
        let rows: Vec<Row> = self
            .rows
            .visible_items()
            .iter()
            .map(|r| {
                let elapsed = format_elapsed(r.elapsed_secs);
                Row::new([r.id.clone(), elapsed, r.status.clone()])
            })
            .collect();

        let table = Table::new(std::iter::once(header).chain(rows), &widths).block(block);
        f.render_widget(table, area);
    }
}

impl Default for TaskOverlay {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

/// Format elapsed seconds as a human-readable string.
fn format_elapsed(secs: u64) -> String {
    let minutes = secs / 60;
    let seconds = secs % 60;
    if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, elapsed_secs: u64, status: &str) -> TaskRow {
        TaskRow {
            id: id.to_string(),
            elapsed_secs,
            status: status.to_string(),
        }
    }

    #[test]
    fn new_overlay_empty() {
        let overlay = TaskOverlay::new(vec![]);
        assert!(overlay.is_empty());
        assert_eq!(overlay.last_action(), TaskAction::None);
    }

    #[test]
    fn open_resets_action() {
        let mut overlay = TaskOverlay::new(vec![]);
        overlay.last_action = TaskAction::CancelRequested("x".into());
        overlay.open(vec![row("1", 10, "running")]);
        assert_eq!(overlay.last_action(), TaskAction::None);
    }

    #[test]
    fn set_rows_updates_list() {
        let mut overlay = TaskOverlay::new(vec![]);
        overlay.set_rows(vec![row("1", 10, "running"), row("2", 20, "queued")]);
        assert_eq!(overlay.len(), 2);
    }

    #[test]
    fn cancel_selected_emits_action() {
        let mut overlay = TaskOverlay::new(vec![
            row("task-1", 10, "running"),
            row("task-2", 20, "running"),
        ]);
        overlay.move_down();
        overlay.cancel_selected();
        assert_eq!(
            overlay.last_action(),
            TaskAction::CancelRequested("task-2".into())
        );
    }

    #[test]
    fn inspect_selected_emits_action() {
        let mut overlay = TaskOverlay::new(vec![row("task-1", 10, "running")]);
        overlay.inspect_selected();
        assert_eq!(
            overlay.last_action(),
            TaskAction::InspectRequested("task-1".into())
        );
    }

    #[test]
    fn cursor_wraps() {
        let mut overlay = TaskOverlay::new(vec![row("1", 10, "running"), row("2", 20, "running")]);
        assert_eq!(overlay.selected_index(), 0);
        overlay.move_down();
        assert_eq!(overlay.selected_index(), 1);
        overlay.move_down();
        assert_eq!(overlay.selected_index(), 0); // wrap
    }

    #[test]
    fn clear_action_resets() {
        let mut overlay = TaskOverlay::new(vec![row("1", 10, "running")]);
        overlay.cancel_selected();
        overlay.clear_action();
        assert_eq!(overlay.last_action(), TaskAction::None);
    }

    #[test]
    fn format_elapsed_shows_minutes_and_seconds() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(5), "5s");
        assert_eq!(format_elapsed(60), "1m 0s");
        assert_eq!(format_elapsed(65), "1m 5s");
        assert_eq!(format_elapsed(125), "2m 5s");
    }
}
