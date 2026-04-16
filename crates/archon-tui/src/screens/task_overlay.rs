//! Tasks overlay screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Table, Row};

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// Task identifier (placeholder).
pub type TaskId = String;

/// A single row in the tasks overlay list.
#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: TaskId,
    pub name: String,
    pub status: String,
    pub progress: u8,
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
    pub fn new() -> Self {
        Self {
            rows: VirtualList::new(Vec::new(), 10),
            last_action: TaskAction::None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn selected_index(&self) -> usize {
        self.rows.selected_index()
    }

    pub fn selected(&self) -> Option<&TaskRow> {
        self.rows.selected()
    }

    pub fn last_action(&self) -> TaskAction {
        self.last_action.clone()
    }

    /// Set task rows.
    pub fn set_rows(&mut self, rows: Vec<TaskRow>) {
        self.rows.set_items(rows);
    }

    /// Set rows and reset action.
    pub fn open(&mut self, rows: Vec<TaskRow>) {
        self.rows.set_items(rows);
        self.last_action = TaskAction::None;
    }

    pub fn move_up(&mut self) {
        self.rows.move_up();
    }

    pub fn move_down(&mut self) {
        self.rows.move_down();
    }

    pub fn page_up(&mut self) {
        self.rows.page_up();
    }

    pub fn page_down(&mut self) {
        self.rows.page_down();
    }

    /// Request cancel for currently selected task.
    pub fn cancel_selected(&mut self) {
        if let Some(row) = self.rows.selected() {
            self.last_action = TaskAction::CancelRequested(row.id.clone());
        }
    }

    /// Request inspect for currently selected task.
    pub fn inspect_selected(&mut self) {
        if let Some(row) = self.rows.selected() {
            self.last_action = TaskAction::InspectRequested(row.id.clone());
        }
    }

    /// Request refresh.
    pub fn refresh(&mut self) {
        self.last_action = TaskAction::RefreshRequested;
    }

    /// Clear last action.
    pub fn clear_action(&mut self) {
        self.last_action = TaskAction::None;
    }

    /// Render tasks overlay into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Tasks");

        let widths = [
            ratatui::layout::Constraint::Percentage(20),
            ratatui::layout::Constraint::Percentage(40),
            ratatui::layout::Constraint::Percentage(20),
            ratatui::layout::Constraint::Percentage(20),
        ];

        let header = Row::new(["ID", "Name", "Status", "Progress"]);
        let rows: Vec<Row> = self.rows.visible_items().iter().map(|r| {
            Row::new([
                r.id.clone(),
                r.name.clone(),
                r.status.clone(),
                format!("{}%", r.progress),
            ])
        }).collect();

        let table = Table::new(std::iter::once(header).chain(rows), &widths).block(block);
        f.render_widget(table, area);
    }
}

impl Default for TaskOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str) -> TaskRow {
        TaskRow {
            id: id.to_string(),
            name: name.to_string(),
            status: "running".to_string(),
            progress: 50,
        }
    }

    #[test]
    fn new_overlay_empty() {
        let overlay = TaskOverlay::new();
        assert!(overlay.is_empty());
        assert_eq!(overlay.last_action(), TaskAction::None);
    }

    #[test]
    fn open_resets_action() {
        let mut overlay = TaskOverlay::new();
        overlay.last_action = TaskAction::CancelRequested("x".into());
        overlay.open(vec![row("1", "a")]);
        assert_eq!(overlay.last_action(), TaskAction::None);
    }

    #[test]
    fn set_rows_updates_list() {
        let mut overlay = TaskOverlay::new();
        overlay.set_rows(vec![row("1", "a"), row("2", "b")]);
        assert_eq!(overlay.len(), 2);
    }

    #[test]
    fn cancel_selected_emits_action() {
        let mut overlay = TaskOverlay::new();
        overlay.set_rows(vec![row("task-1", "a"), row("task-2", "b")]);
        overlay.move_down();
        overlay.cancel_selected();
        assert_eq!(overlay.last_action(), TaskAction::CancelRequested("task-2".into()));
    }

    #[test]
    fn inspect_selected_emits_action() {
        let mut overlay = TaskOverlay::new();
        overlay.set_rows(vec![row("task-1", "a")]);
        overlay.inspect_selected();
        assert_eq!(overlay.last_action(), TaskAction::InspectRequested("task-1".into()));
    }

    #[test]
    fn cursor_wraps() {
        let mut overlay = TaskOverlay::new();
        overlay.set_rows(vec![row("1", "a"), row("2", "b")]);
        overlay.move_down();
        assert_eq!(overlay.selected_index(), 1);
        overlay.move_down();
        assert_eq!(overlay.selected_index(), 0); // wrap
    }

    #[test]
    fn clear_action_resets() {
        let mut overlay = TaskOverlay::new();
        overlay.cancel_selected();
        overlay.clear_action();
        assert_eq!(overlay.last_action(), TaskAction::None);
    }
}